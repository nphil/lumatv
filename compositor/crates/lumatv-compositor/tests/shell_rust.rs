//! The Rust shell end to end on the host (feature `ui`, Mesa's software GL): the real `lumatv-ui` binary started by the compositor's
//! own supervision (`shell_rust.rs`) with the engine's socket and the key socket, speaking the UI channel on its own port; the audio
//! topic puts its volume display on the output, the toast feed its toast, and a restart drops its layers and brings it back.
//! Needs `rustui/target/debug/lumatv-ui` (built by `cargo build -p lumatv-ui` in `rustui/`); skipped without it.
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
fn the_rust_shell_is_started_by_the_compositor_and_draws_the_volume_display_and_a_toast() {
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
    let (d, module) = (dir.clone(), dir.join("module"));
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
    let report = server.eval(|s| s.shell_report());
    assert_eq!(report["rust"]["ui"]["kinds"], json!(["osd", "toast"]));
    assert_eq!(report["rust"]["ui"]["osd"], false);

    // a volume key: the audio service's state with a cause puts the display up, over the windows (none: black)
    server.eval(|s| {
        s.shell_audio(&json!({"volumeStatus": {"volume": 31, "muteStatus": false, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": ""}}));
        s.shell_audio(&json!({"volumeStatus": {"volume": 31, "muteStatus": false, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": "volumeUp"}}));
    });
    wait_for(&server, "the display reported up", Duration::from_secs(10), |s| s.shell_rust.ui_state["osd"] == json!(true));
    wait_for(&server, "the display drawn", Duration::from_secs(10), |s| {
        let image = harness::drawn(s);
        // the pane over nothing (no Home drawn): lifted on the bottom edge, 440 wide about the middle
        image.pixel(760, 865) != [0, 0, 0, 255]
    });
    let (image, errors) = server.eval(|s| (s.output.capture().unwrap(), s.stats.gl_errors));
    assert_ne!(image.pixel(960, 865), [0, 0, 0, 255], "the bar's pane is drawn");
    assert_eq!(image.pixel(960, 300), [0, 0, 0, 255], "nothing else is");
    assert_eq!(errors, 0);

    // the toast feed: the client subscribed through the channel; a push becomes a toast in the upper right corner
    let token = server.eval(|s| s.shell_rust.luna_tags.iter().find(|t| t.uri.ends_with("/getToastNotification")).map(|t| t.token));
    let token = token.expect("the client subscribed to the toast feed");
    server.eval(move |s| {
        assert!(s.shell_luna_event(token, &json!({"sourceId": "lg", "message": "Remote battery low", "timeout": 8})));
    });
    wait_for(&server, "the toast reported", Duration::from_secs(10), |s| s.shell_rust.ui_state["toasts"] == json!(1));
    wait_for(&server, "the toast drawn", Duration::from_secs(10), |s| harness::drawn(s).pixel(1600, 70) != [0, 0, 0, 255]);

    // a restart: the layers go with the client in one frame, and the client comes back
    server.eval(|s| s.shell_rust_restart());
    wait_for(&server, "the layers gone", Duration::from_secs(10), |s| harness::drawn(s).pixel(1600, 70) == [0, 0, 0, 255]);
    wait_for(&server, "the client back on the engine", Duration::from_secs(20), |s| {
        s.shell_rust.connected && s.shell_rust.engine_connected && s.shell_rust.stats.starts == 2
    });
    let report = server.eval(|s| s.shell_report());
    assert_eq!(report["rust"]["stats"]["starts"], 2);
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
