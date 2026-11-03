//! Native UI supervision on the host with the real client, Mesa, engine socket and channel.
//! Audio and toast events update the client; an unexpected exit detaches its engine and requests stock recovery.
//! Requires rustui/target/debug/lumatv-ui; skipped without it.
#![cfg(feature = "ui")]

mod harness;

use harness::Server;
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use lumatv_compositor::shell_rust::SYSTEM_Z;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

fn binary() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../rustui/target/debug/lumatv-ui");
    p.exists().then_some(p)
}

fn wait_for(server: &Server, what: &str, timeout: Duration, f: impl FnMut(&mut lumatv_compositor::State) -> bool + Send + Clone + 'static) {
    let deadline = Instant::now() + timeout;
    loop {
        let g = f.clone();
        let done = server.eval(move |s| {
            // what the main loop does on its tick and on the channel's eventfd
            s.poll_holds();
            s.shell_rust_poll();
            let mut g = g;
            g(s)
        });
        if done {
            return;
        }
        if Instant::now() >= deadline {
            let journal = server.eval(|s| s.journal.iter().filter(|l| l.contains("shell-rust")).cloned().collect::<Vec<_>>());
            panic!("{what}: not within {timeout:?}; the journal: {journal:#?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_native_client_reports_audio_and_toasts_and_its_exit_requests_stock_recovery() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    let Some(bin) = binary() else {
        eprintln!("skipped: no rustui/target/debug/lumatv-ui");
        return;
    };
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe {
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        std::env::set_var("LUMATV_UI_BIN", &bin);
    }
    let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-e2e-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("theme")).unwrap();
    std::fs::create_dir_all(dir.join("compositor")).unwrap();
    let shm = dir.join("ui-shm");
    std::fs::create_dir_all(&shm).unwrap();
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let shm2 = shm.clone();
    let server = Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        tx.send(gl.attach_ui(MESA, SYSTEM_Z, shm2).unwrap()).unwrap();
        Box::new(gl)
    }));
    let socket = rx.recv().unwrap();
    let (d, module) = (dir.clone(), dir.join("compositor/assets/ui"));
    std::fs::create_dir_all(module.join("icons/audio")).unwrap();
    server.eval(move |s| {
        s.config.runtime_dir = d.clone();
        s.config.data_dir = d.clone();
        s.config.shell_port = port;
        s.config.shell_rust = vec!["osd".into(), "toast".into()];
        s.config.driver = Some(MESA.into());
        s.shell_rust.adopt_socket(socket);
        s.shell_rust_start();
        assert!(s.shell_rust.enabled(), "{:?}", s.shell_rust.error);
        s.shell_rust_start_now();
    });
    wait_for(&server, "the client connected to the channel and the engine", Duration::from_secs(20), |s| {
        s.shell_rust.connected && s.shell_rust.engine_connected
    });
    wait_for(&server, "the client's first report", Duration::from_secs(10), |s| s.shell_rust.ui_state.get("kinds").is_some());
    wait_for(&server, "the first native frame", Duration::from_secs(10), |s| s.shell_report()["ready"] == json!(true));
    let report = server.eval(|s| s.shell_report());
    assert_eq!(report["ui"]["kinds"], json!(["osd", "toast"]));
    assert_eq!(report["ui"]["osd"], false);

    // a volume key: the audio service's state with a cause puts the display up, over the windows (none: black)
    server.eval(|s| {
        s.shell_audio(&json!({"volumeStatus": {"volume": 31, "muteStatus": false, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": ""}}));
        s.shell_audio(&json!({"volumeStatus": {"volume": 31, "muteStatus": false, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": "volumeUp"}}));
    });
    wait_for(&server, "the display reported up", Duration::from_secs(10), |s| s.shell_rust.ui_state["osd"] == json!(true));

    // The client subscribes to LG's transient toast feed through the channel.
    let token = server.eval(|s| s.shell_rust.luna_tags.iter().find(|t| t.uri.ends_with("/getToastNotification")).map(|t| t.token));
    let token = token.expect("the client subscribed to the toast feed");
    server.eval(move |s| {
        assert!(s.shell_luna_event(token, &json!({"sourceId": "lg", "message": "Remote battery low", "timeout": 8})));
    });
    wait_for(&server, "the toast reported", Duration::from_secs(10), |s| s.shell_rust.ui_state["toasts"] == json!(1));

    // An unexpected client exit removes the layers and requests stock recovery, without restarting.
    let pid = server.eval(|s| s.shell_rust.child_pid().unwrap());
    // SAFETY: the PID belongs to the child started by this test.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) }, 0);
    wait_for(&server, "native UI exit", Duration::from_secs(10), |s| s.shell_rust.gave_up);
    wait_for(&server, "the engine detached", Duration::from_secs(10), |s| !s.output.ui_attached());
    let report = server.eval(|s| s.shell_report());
    assert_eq!(report["stats"]["starts"], 1);
    assert_eq!(report["stats"]["exits"], 1);
    assert_eq!(report["connected"], false);
    assert_eq!(report["ready"], false);
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
