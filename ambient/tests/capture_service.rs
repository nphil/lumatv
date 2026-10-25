//! The 32-bit capture service (`capture/service.c`, `lumatv-capture`) against this crate's own frame decoder: the bytes it
//! writes to the helper's capture socket are what `wire::decode` reads, for the plain and the games preset, and the preset
//! byte the helper sends back changes them. The service runs on the host against the fake libvtcapture, so this needs
//! `capture/build.sh` to have run (the test says so and passes without it, as the interop tests do without OpenSSL).

use lumatv_ambient::wire;
use lumatv_ambient::zones::Preset;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn accept(listener: &UnixListener) -> UnixStream {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                return stream;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("the service did not connect: {e}"),
        }
    }
}

/// One whole frame, read the way the helper reads it: the header names the preset, which names the length.
fn read_frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0u8; wire::HEADER];
    stream.read_exact(&mut header).unwrap();
    let preset = Preset::from_index(header[4]).expect("a preset the crate knows");
    let mut bytes = vec![0u8; wire::frame_len(preset)];
    bytes[..wire::HEADER].copy_from_slice(&header);
    stream.read_exact(&mut bytes[wire::HEADER..]).unwrap();
    bytes
}

#[test]
fn frames_from_the_service_decode_and_the_preset_byte_changes_them() {
    let build = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capture/build/host");
    let bin = build.join("lumatv-capture");
    if !bin.exists() {
        eprintln!(
            "skipped: {} is not built (run ambient/capture/build.sh)",
            bin.display()
        );
        return;
    }
    let socket = std::env::temp_dir().join(format!("capture-wire-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let _service = Service(
        Command::new(&bin)
            .args(["-s", socket.to_str().unwrap(), "-i", "50", "-b", "400"])
            .env("LD_LIBRARY_PATH", &build)
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut stream = accept(&listener);

    // The helper tells the preset first: plain.
    stream.write_all(&[0]).unwrap();
    let bytes = read_frame(&mut stream);
    assert_eq!(bytes.len(), 204);
    let (frame, t0) = wire::decode(&bytes).expect("the crate reads the service's frame");
    assert_eq!(frame.preset.index(), 0);
    assert_eq!(frame.zone_count(), 16);
    // The fake library puts 1000 pixels of each channel in one bin per area: green at code 8 * area, blue at 255 - 4 * area.
    for area in [0usize, 3, 15] {
        let green = (area * 8) as f32 / 255.0;
        let blue = (255 - area * 4) as f32 / 255.0;
        assert!(
            (frame.color[area].g - green).abs() < 2e-4,
            "area {area} green"
        );
        assert!(
            (frame.color[area].b - blue).abs() < 2e-4,
            "area {area} blue"
        );
        assert_eq!(frame.count[area], 1000);
        assert_eq!(frame.black[area], 0.0);
    }

    // Later frames are later.
    let (_, t1) = wire::decode(&read_frame(&mut stream)).unwrap();
    assert!(t1 >= t0);

    // The preset the engine wants changes: the games preset has 14 areas.
    stream.write_all(&[3]).unwrap();
    let mut games = None;
    for _ in 0..60 {
        let bytes = read_frame(&mut stream);
        let (frame, _) = wire::decode(&bytes).expect("every frame decodes");
        if frame.preset.index() == 3 {
            games = Some((bytes.len(), frame.zone_count()));
            break;
        }
    }
    assert_eq!(games, Some((12 + 14 * 12, 14)));
    let _ = std::fs::remove_file(&socket);
}
