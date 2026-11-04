//! Pictures for the shell: a JPEG file of a known folder, decoded off the CPU where the TV can, at the size the shell needs.
//!
//! `GET /v1/image/{root}/{file}?w=&h=` answers the file decoded to a 32-bit BMP (what Qt's `Image` reads with no decoder of its
//! own to run), scaled down by 1/2, 1/4 or 1/8 as far as `w` and `h` allow, so a 4K painting for a 1080p window arrives as 1080p.
//! The decode is `lumatv-jpeg` (`jpeg/service.c`): a 32-bit program, because LG's JPEG library is, over the TV's hardware JPEG block
//! and, when the block is missing or refuses a file, over libjpeg (still not the helper's own CPU time; the service has the same
//! planes either way). It is started when a picture is wanted and ends by itself after 30 s without one, holding the hardware only
//! while pictures are being decoded.
//!
//! Where it cannot be had (no decoder staged, it fails, the picture is over `images.maxPixels`, `images.enabled` is false) the answer
//! is the file itself as `image/jpeg`, and the shell's own decoder does what it always did. Both answers carry `X-Luma-Decoder`
//! (`hardware`, `cpu` or `file`).
//!
//! The shell names a root and a file, never a path: a root is a folder from the config (or the two the helper knows), a file is a name
//! directly inside it, and what is read has to be a regular `.jpg`/`.jpeg` that stays inside the root once links are followed.

use crate::config::{Config, Paths};
use crate::http::{Request, Response, error_response};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/image/{root}/{file}",
        "a JPEG of a known folder, decoded to a BMP at the size w and h ask for (the file as it is where it cannot be)",
    ),
    (
        "GET",
        "/v1/images",
        "the picture decoder: the folders, whether it runs, how many pictures each decoder gave",
    ),
];

/// The wire to `lumatv-jpeg` (`jpeg/jpegproto.h`).
const REQ_MAGIC: u32 = 0x3151_504a;
const RSP_MAGIC: u32 = 0x3152_504a;
const RSP_BYTES: usize = 64;
const OP_DECODE: u32 = 1;
const FMT_BGRA: u32 = 3;
const RFLAG_CPU: u32 = 1;

/// The largest JPEG file read: the decoder's own limit.
const MAX_FILE: u64 = 64 * 1024 * 1024;
/// The longest the decoder is given for one picture, hardware or not.
const DECODE_TIMEOUT: Duration = Duration::from_secs(20);
/// After the decoder could not be started or stopped answering, pictures are served as files for this long before it is tried again.
const BACKOFF: Duration = Duration::from_secs(30);
const BMP_HEADER: usize = 54;
const BACKING_OFF: &str = "the picture decoder is not available just now";

/// A root's name as it may appear in a path or in `images.roots`.
pub fn root_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn jpeg_extension(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".jpg", ".jpeg", ".jpe"].iter().any(|e| lower.ends_with(e))
}

/// The dimensions in the frame header of a JPEG file, or `None` when it is not one.
pub fn jpeg_size(p: &[u8]) -> Option<(u32, u32)> {
    if p.len() < 4 || p[0] != 0xff || p[1] != 0xd8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= p.len() {
        if p[i] != 0xff {
            return None;
        }
        while i < p.len() && p[i] == 0xff {
            i += 1;
        }
        if i >= p.len() {
            break;
        }
        let m = p[i];
        i += 1;
        if m == 0x01 || (0xd0..=0xd8).contains(&m) {
            continue;
        }
        if m == 0xd9 || m == 0xda || i + 2 > p.len() {
            break;
        }
        let len = usize::from(p[i]) << 8 | usize::from(p[i + 1]);
        if len < 2 || i + len > p.len() {
            return None;
        }
        if (0xc0..=0xcf).contains(&m) && !matches!(m, 0xc4 | 0xc8 | 0xcc) {
            if len < 8 {
                return None;
            }
            let s = &p[i + 2..];
            let height = u32::from(s[1]) << 8 | u32::from(s[2]);
            let width = u32::from(s[3]) << 8 | u32::from(s[4]);
            return (width > 0 && height > 0).then_some((width, height));
        }
        i += len;
    }
    None
}

fn ceil_shift(v: u32, s: u8) -> u32 {
    (v + (1 << s) - 1) >> s
}

/// How far the decode is scaled down (0..=3, 1/1 to 1/8 per axis): the most that still leaves the picture at least `want_w` by
/// `want_h` (0 asks for nothing: the full size when both are 0).
pub fn choose_shift(width: u32, height: u32, want_w: u32, want_h: u32) -> u8 {
    if want_w == 0 && want_h == 0 {
        return 0;
    }
    let mut shift = 0;
    for s in 1..=3u8 {
        if ceil_shift(width, s) >= want_w && ceil_shift(height, s) >= want_h {
            shift = s;
        } else {
            break;
        }
    }
    shift
}

/// A decoded picture as a 32-bit bottom-up BMP: what comes back from the decoder (B, G, R, 255 a pixel, top row first) is read into
/// its place a row at a time, so the picture is copied once.
fn bmp_header(width: u32, height: u32, pixels: usize) -> [u8; BMP_HEADER] {
    let mut h = [0u8; BMP_HEADER];
    h[0..2].copy_from_slice(b"BM");
    h[2..6].copy_from_slice(&((BMP_HEADER + pixels) as u32).to_le_bytes());
    h[10..14].copy_from_slice(&(BMP_HEADER as u32).to_le_bytes());
    h[14..18].copy_from_slice(&40u32.to_le_bytes());
    h[18..22].copy_from_slice(&width.to_le_bytes());
    h[22..26].copy_from_slice(&height.to_le_bytes());
    h[26..28].copy_from_slice(&1u16.to_le_bytes());
    h[28..30].copy_from_slice(&32u16.to_le_bytes());
    h[34..38].copy_from_slice(&(pixels as u32).to_le_bytes());
    h[38..42].copy_from_slice(&2835u32.to_le_bytes());
    h[42..46].copy_from_slice(&2835u32.to_le_bytes());
    h
}

struct Running {
    child: Child,
    sock: UnixStream,
    next_id: u32,
}

impl Running {
    /// Closes the socket (the decoder gives the hardware back when it sees that) and waits a moment for it; kills it if it does not go.
    fn finish(mut self) {
        let _ = self.sock.shutdown(std::net::Shutdown::Both);
        for _ in 0..50 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    down_until: Option<Instant>,
    hardware: u64,
    cpu: u64,
    file: u64,
    last_problem: Option<String>,
    last_decode_ms: Option<u64>,
}

/// What the decoder said about one request.
enum Outcome {
    Picture {
        width: u32,
        height: u32,
        body: Vec<u8>,
        cpu: bool,
        decode_us: u32,
    },
    /// The decoder answered with a status instead of a picture; it is still there for the next request.
    Refused(u32),
}

/// The `w` and `h` a request asks for (0: not asked), or the answer to a request that asks badly.
pub fn wanted_size(request: &Request) -> Result<(u32, u32), Response> {
    let dim = |name: &str| -> Result<u32, Response> {
        match request.param(name) {
            None | Some("") => Ok(0),
            Some(text) => text
                .parse::<u32>()
                .ok()
                .filter(|v| *v <= 16384)
                .ok_or_else(|| {
                    error_response(
                        400,
                        "bad_request",
                        &format!("{name} is a whole number up to 16384"),
                    )
                }),
        }
    };
    Ok((dim("w")?, dim("h")?))
}

pub struct Images {
    enabled: bool,
    roots: BTreeMap<String, PathBuf>,
    decoder: PathBuf,
    decoder_args: Vec<String>,
    max_pixels: u32,
    timeout: Duration,
    state: Mutex<State>,
}

impl Images {
    pub fn new(config: &Config, paths: &Paths) -> Self {
        let cfg = &config.images;
        let mut roots = BTreeMap::new();
        roots.insert("images".to_owned(), paths.data_dir.join("images"));
        if let Some(module) = &config.brand_qml_module {
            roots.insert(
                "art".to_owned(),
                paths
                    .data_dir
                    .join("qml/WebOSCompositor")
                    .join(module)
                    .join("art"),
            );
        }
        for (name, dir) in &cfg.roots {
            roots.insert(name.clone(), PathBuf::from(dir));
        }
        let decoder = if cfg.decoder.is_empty() {
            paths.data_dir.join("bin/lumatv-jpeg")
        } else {
            PathBuf::from(&cfg.decoder)
        };
        Self {
            enabled: cfg.enabled,
            roots,
            decoder,
            decoder_args: cfg.decoder_args.clone(),
            max_pixels: cfg.max_pixels,
            timeout: DECODE_TIMEOUT,
            state: Mutex::new(State::default()),
        }
    }

    /// `GET /v1/images`
    pub fn status(&self) -> Value {
        let st = self.state.lock();
        let roots: serde_json::Map<String, Value> = self
            .roots
            .iter()
            .map(|(n, p)| (n.clone(), Value::String(p.display().to_string())))
            .collect();
        json!({"images": {
            "enabled": self.enabled,
            "decoder": self.decoder.display().to_string(),
            "decoderPresent": self.decoder.is_file(),
            "running": st.running.is_some(),
            "backingOff": st.down_until.is_some_and(|t| Instant::now() < t),
            "roots": roots,
            "served": {"hardware": st.hardware, "cpu": st.cpu, "file": st.file},
            "lastDecodeMs": st.last_decode_ms,
            "lastProblem": st.last_problem,
        }})
    }

    /// `GET /v1/image/{root}/{file}?w=&h=`
    pub fn serve(&self, request: &Request, root: &str, file: &str) -> Response {
        if !root_name_ok(root) || !file_name_ok(file) {
            return error_response(
                400,
                "bad_request",
                "a root is a lowercase name and a file a plain name",
            );
        }
        let Some(dir) = self.roots.get(root) else {
            return error_response(404, "no_such_root", "no such picture folder");
        };
        let (want_w, want_h) = match wanted_size(request) {
            Ok(size) => size,
            Err(r) => return r,
        };
        let bytes = match read_inside(dir, file) {
            Ok(b) => b,
            Err(ReadErr::NotFound) => {
                return error_response(404, "no_such_file", "no such picture");
            }
            Err(ReadErr::TooBig) => {
                return error_response(413, "too_big", "that file is over 64 MiB");
            }
            Err(ReadErr::Io(why)) => return error_response(500, "io", &why),
        };
        if !jpeg_extension(file) {
            return error_response(415, "not_a_jpeg", "only .jpg and .jpeg pictures are served");
        }
        self.serve_jpeg(bytes, want_w, want_h)
    }

    /// The picture in `bytes` (a JPEG from anywhere: a file, a camera) decoded for a window of `want_w` by `want_h`, as `serve` answers it:
    /// a BMP from the decoder, or the JPEG as it is when there is none.
    pub fn serve_jpeg(&self, bytes: Vec<u8>, want_w: u32, want_h: u32) -> Response {
        let Some((width, height)) = jpeg_size(&bytes) else {
            return error_response(415, "not_a_jpeg", "that file is not a JPEG");
        };
        if !self.enabled {
            return self.file_answer(bytes, "images.enabled is false");
        }
        let shift = choose_shift(width, height, want_w, want_h);
        let (ow, oh) = (ceil_shift(width, shift), ceil_shift(height, shift));
        if u64::from(ow) * u64::from(oh) > u64::from(self.max_pixels) {
            return self.file_answer(bytes, "over images.maxPixels");
        }
        match self.decode(&bytes, shift) {
            Ok(Outcome::Picture {
                width,
                height,
                body,
                cpu,
                decode_us,
            }) => {
                let mut st = self.state.lock();
                if cpu {
                    st.cpu += 1;
                } else {
                    st.hardware += 1;
                }
                st.last_decode_ms = Some(u64::from(decode_us) / 1000);
                drop(st);
                Response {
                    status: 200,
                    headers: vec![
                        ("Content-Type", "image/bmp".to_owned()),
                        (
                            "X-Luma-Decoder",
                            if cpu { "cpu" } else { "hardware" }.to_owned(),
                        ),
                        ("X-Luma-Size", format!("{width}x{height}")),
                        ("X-Luma-Decode-Us", decode_us.to_string()),
                    ],
                    body,
                }
            }
            Ok(Outcome::Refused(status)) => self.file_answer(
                bytes,
                &format!("the decoder refused the file (status {status})"),
            ),
            Err(why) => self.file_answer(bytes, &why),
        }
    }

    /// The file as it is, and why: the shell decodes it itself.
    fn file_answer(&self, bytes: Vec<u8>, why: &str) -> Response {
        {
            let mut st = self.state.lock();
            st.file += 1;
            // The reason a backoff began is the one worth keeping.
            if why != BACKING_OFF {
                st.last_problem = Some(why.to_owned());
            }
        }
        Response {
            status: 200,
            headers: vec![
                ("Content-Type", "image/jpeg".to_owned()),
                ("X-Luma-Decoder", "file".to_owned()),
                (
                    "X-Luma-Reason",
                    why.chars()
                        .filter(|c| c.is_ascii() && !c.is_control())
                        .collect(),
                ),
            ],
            body: bytes,
        }
    }

    /// One picture through the decoder, started if it is not running. An error is why there is none.
    fn decode(&self, jpeg: &[u8], shift: u8) -> Result<Outcome, String> {
        let mut st = self.state.lock();
        if st.down_until.is_some_and(|t| Instant::now() < t) {
            return Err(BACKING_OFF.to_owned());
        }
        st.down_until = None;
        let mut last = String::new();
        // A second try: the decoder may have ended by itself (idle) between the last picture and this one.
        for _ in 0..2 {
            if st
                .running
                .as_mut()
                .is_some_and(|r| matches!(r.child.try_wait(), Ok(Some(_))))
            {
                st.running = None;
            }
            if st.running.is_none() {
                match self.start() {
                    Ok(r) => st.running = Some(r),
                    Err(e) => {
                        let why = format!(
                            "the picture decoder cannot start ({}): {e}",
                            self.decoder.display()
                        );
                        st.down_until = Some(Instant::now() + BACKOFF);
                        st.last_problem = Some(why.clone());
                        return Err(why);
                    }
                }
            }
            let run = st.running.as_mut().expect("started above");
            match exchange(run, jpeg, shift, self.max_pixels) {
                Ok(outcome) => return Ok(outcome),
                Err(e) => {
                    last = format!("the picture decoder stopped answering: {e}");
                    if let Some(r) = st.running.take() {
                        r.finish();
                    }
                }
            }
        }
        st.down_until = Some(Instant::now() + BACKOFF);
        st.last_problem = Some(last.clone());
        Err(last)
    }

    fn start(&self) -> io::Result<Running> {
        let (mine, theirs) = UnixStream::pair()?;
        let child = Command::new(&self.decoder)
            .arg("-f")
            .arg("0")
            .args(&self.decoder_args)
            .stdin(Stdio::from(OwnedFd::from(theirs)))
            .stdout(Stdio::null())
            // What the decoder says goes to the helper's own log.
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()?;
        mine.set_read_timeout(Some(self.timeout))?;
        mine.set_write_timeout(Some(self.timeout))?;
        Ok(Running {
            child,
            sock: mine,
            next_id: 1,
        })
    }
}

impl Drop for Images {
    fn drop(&mut self) {
        if let Some(r) = self.state.lock().running.take() {
            r.finish();
        }
    }
}

enum ReadErr {
    NotFound,
    TooBig,
    Io(String),
}

/// The bytes of `dir/file` when it is a regular file that is still inside `dir` once links are followed.
fn read_inside(dir: &Path, file: &str) -> Result<Vec<u8>, ReadErr> {
    let root = std::fs::canonicalize(dir).map_err(|_| ReadErr::NotFound)?;
    let path = std::fs::canonicalize(dir.join(file)).map_err(|_| ReadErr::NotFound)?;
    if !path.starts_with(&root) {
        return Err(ReadErr::NotFound);
    }
    let meta = std::fs::metadata(&path).map_err(|_| ReadErr::NotFound)?;
    if !meta.is_file() {
        return Err(ReadErr::NotFound);
    }
    if meta.len() > MAX_FILE {
        return Err(ReadErr::TooBig);
    }
    std::fs::read(&path).map_err(|e| ReadErr::Io(e.to_string()))
}

fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// One request and its answer on the decoder's socket.
fn exchange(run: &mut Running, jpeg: &[u8], shift: u8, max_pixels: u32) -> io::Result<Outcome> {
    let id = run.next_id;
    run.next_id = run.next_id.wrapping_add(1);
    let mut head = [0u8; 32];
    head[0..4].copy_from_slice(&REQ_MAGIC.to_le_bytes());
    head[4..8].copy_from_slice(&id.to_le_bytes());
    head[8..12]
        .copy_from_slice(&(OP_DECODE | FMT_BGRA << 8 | u32::from(shift) << 16).to_le_bytes());
    head[12..16].copy_from_slice(&max_pixels.to_le_bytes());
    head[16..20].copy_from_slice(&(jpeg.len() as u32).to_le_bytes());
    run.sock.write_all(&head)?;
    run.sock.write_all(jpeg)?;
    let mut r = [0u8; RSP_BYTES];
    run.sock.read_exact(&mut r)?;
    if word(&r, 0) != RSP_MAGIC || word(&r, 4) != id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an answer that is not the decoder's",
        ));
    }
    let (status, width, height, stride, len) = (
        word(&r, 8),
        word(&r, 12),
        word(&r, 16),
        word(&r, 20),
        word(&r, 28),
    );
    if status != 0 {
        if len != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a status with a body",
            ));
        }
        return Ok(Outcome::Refused(status));
    }
    let bad = |what: &str| Err(io::Error::new(io::ErrorKind::InvalidData, what.to_owned()));
    if width == 0
        || height == 0
        || u64::from(stride) != u64::from(width) * 4
        || u64::from(len) != u64::from(stride) * u64::from(height)
    {
        return bad("an answer whose sizes do not agree");
    }
    let bytes = len as usize;
    let mut body: Vec<u8> = Vec::new();
    body.try_reserve_exact(BMP_HEADER + bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "no memory for the picture"))?;
    body.extend_from_slice(&bmp_header(width, height, bytes));
    body.resize(BMP_HEADER + bytes, 0);
    // The decoder sends the top row first and a BMP stores the bottom row first.
    let stride = stride as usize;
    for row in (0..height as usize).rev() {
        let at = BMP_HEADER + row * stride;
        run.sock.read_exact(&mut body[at..at + stride])?;
    }
    Ok(Outcome::Picture {
        width,
        height,
        body,
        cpu: word(&r, 60) & RFLAG_CPU != 0,
        decode_us: word(&r, 32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A 64x48 4:2:0 JPEG whose red rises down the picture and green across it (made with Pillow).
    const PICTURE: &[u8] = include_bytes!("../testdata/gradient-64x48.jpg");

    fn host_build() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../jpeg/build/host")
    }

    fn request(query: &[(&str, &str)]) -> Request {
        Request {
            method: "GET".into(),
            path: "/v1/image/images/a.jpg".into(),
            query: query
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            headers: Vec::new(),
            body: Vec::new(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }

    fn header<'a>(r: &'a Response, name: &str) -> Option<&'a str> {
        r.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    struct Rig {
        dir: tempfile::TempDir,
        images: Images,
    }

    /// `decoder`: a wrapper that runs the host build of lumatv-jpeg with (`fake`) or without LG's library, or a path of the test's own.
    fn rig(decoder: Option<String>, tweak: impl FnOnce(&mut Config)) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("images")).unwrap();
        std::fs::write(dir.path().join("images/a.jpg"), PICTURE).unwrap();
        let mut config = Config::default();
        if let Some(d) = decoder {
            config.images.decoder = d;
        }
        tweak(&mut config);
        let mut images = Images::new(&config, &Paths::new(dir.path()));
        images.timeout = Duration::from_millis(600);
        Rig { dir, images }
    }

    /// A script that starts the host build of the decoder, with the fake library on its path or not.
    fn wrapper(dir: &Path, fake: bool) -> Option<String> {
        let bin = host_build().join("lumatv-jpeg");
        if !bin.exists() {
            eprintln!("skipped: needs jpeg/build.sh (the host build of lumatv-jpeg)");
            return None;
        }
        let path = dir.join(if fake { "dec-fake" } else { "dec-cpu" });
        let lib = if fake {
            format!("export LD_LIBRARY_PATH={}\n", host_build().display())
        } else {
            String::new()
        };
        std::fs::write(
            &path,
            format!("#!/bin/sh\n{lib}exec {} \"$@\"\n", bin.display()),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Some(path.display().to_string())
    }

    fn bmp_pixel(body: &[u8], row_from_top: usize) -> (u8, u8, u8) {
        let w = word(body, 18) as usize;
        let h = word(body, 22) as usize;
        let at = BMP_HEADER + (h - 1 - row_from_top) * w * 4;
        (body[at + 2], body[at + 1], body[at])
    }

    #[test]
    fn the_frame_header_gives_the_size_and_anything_else_gives_none() {
        assert_eq!(jpeg_size(PICTURE), Some((64, 48)));
        assert_eq!(jpeg_size(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(jpeg_size(&PICTURE[..20]), None);
        assert_eq!(jpeg_size(&[]), None);
    }

    #[test]
    fn the_scale_is_the_most_that_keeps_the_size_asked_for() {
        // a 3840x2160 painting
        assert_eq!(choose_shift(3840, 2160, 0, 0), 0);
        assert_eq!(choose_shift(3840, 2160, 1920, 1080), 1);
        assert_eq!(choose_shift(3840, 2160, 1921, 1080), 0);
        assert_eq!(choose_shift(3840, 2160, 960, 540), 2);
        assert_eq!(choose_shift(3840, 2160, 100, 100), 3);
        assert_eq!(choose_shift(3840, 2160, 1920, 0), 1);
        assert_eq!(choose_shift(3840, 2160, 0, 1080), 1);
        assert_eq!(choose_shift(101, 101, 51, 51), 1);
        // the blurred fill behind a portrait camera (32 by 18 asked of the bridge's 320 pixel pictures, wide or tall) is decoded at one eighth
        assert_eq!(choose_shift(320, 180, 32, 18), 3);
        assert_eq!(choose_shift(320, 426, 32, 18), 3);
    }

    #[test]
    fn a_bmp_header_is_the_one_qt_reads() {
        let h = bmp_header(64, 48, 64 * 48 * 4);
        assert_eq!(&h[0..2], b"BM");
        assert_eq!(word(&h, 2) as usize, BMP_HEADER + 64 * 48 * 4);
        assert_eq!(
            (word(&h, 10), word(&h, 14), word(&h, 18), word(&h, 22)),
            (54, 40, 64, 48)
        );
        assert_eq!((h[28], h[30]), (32, 0)); // 32 bits, no compression
    }

    #[test]
    fn names_that_could_leave_the_folder_are_refused() {
        let r = rig(None, |_| {});
        for (root, file) in [
            ("images", "../a.jpg"),
            ("images", ".a.jpg"),
            ("images", "a/b.jpg"),
            ("Images", "a.jpg"),
            ("images", ""),
        ] {
            assert_eq!(
                r.images.serve(&request(&[]), root, file).status,
                400,
                "{root}/{file}"
            );
        }
        assert_eq!(
            r.images.serve(&request(&[]), "nowhere", "a.jpg").status,
            404
        );
        assert_eq!(
            r.images
                .serve(&request(&[]), "images", "missing.jpg")
                .status,
            404
        );
        for bad in ["-1", "x", "16385"] {
            assert_eq!(
                r.images
                    .serve(&request(&[("w", bad)]), "images", "a.jpg")
                    .status,
                400,
                "{bad}"
            );
        }
    }

    #[test]
    fn a_link_out_of_the_folder_and_what_is_not_a_jpeg_are_not_served() {
        let r = rig(None, |_| {});
        let outside = r.dir.path().join("secret.jpg");
        std::fs::write(&outside, PICTURE).unwrap();
        std::os::unix::fs::symlink(&outside, r.dir.path().join("images/link.jpg")).unwrap();
        assert_eq!(
            r.images.serve(&request(&[]), "images", "link.jpg").status,
            404
        );
        std::fs::write(r.dir.path().join("images/x.png"), b"\x89PNG").unwrap();
        assert_eq!(r.images.serve(&request(&[]), "images", "x.png").status, 415);
        std::fs::write(r.dir.path().join("images/y.jpg"), b"not a picture").unwrap();
        assert_eq!(r.images.serve(&request(&[]), "images", "y.jpg").status, 415);
        std::fs::create_dir(r.dir.path().join("images/d.jpg")).unwrap();
        assert_eq!(r.images.serve(&request(&[]), "images", "d.jpg").status, 404);
    }

    #[test]
    fn without_a_decoder_the_file_is_served_as_it_is_and_the_decoder_is_left_alone_for_a_while() {
        let r = rig(Some("/nonexistent/lumatv-jpeg".into()), |_| {});
        for _ in 0..3 {
            let a = r.images.serve(&request(&[("w", "32")]), "images", "a.jpg");
            assert_eq!(
                (
                    a.status,
                    header(&a, "X-Luma-Decoder"),
                    header(&a, "Content-Type")
                ),
                (200, Some("file"), Some("image/jpeg"))
            );
            assert_eq!(a.body, PICTURE);
        }
        let s = r.images.status();
        assert_eq!(s["images"]["served"]["file"], 3);
        assert_eq!(s["images"]["backingOff"], true);
        assert!(
            s["images"]["lastProblem"]
                .as_str()
                .unwrap()
                .contains("cannot start")
        );
        assert_eq!(s["images"]["decoderPresent"], false);
    }

    #[test]
    fn a_disabled_helper_and_a_picture_over_the_limit_serve_the_file() {
        let r = rig(None, |c| c.images.enabled = false);
        assert_eq!(
            header(
                &r.images.serve(&request(&[]), "images", "a.jpg"),
                "X-Luma-Decoder"
            ),
            Some("file")
        );
        let dir = tempfile::tempdir().unwrap();
        if let Some(d) = wrapper(dir.path(), false) {
            let r = rig(Some(d), |c| c.images.max_pixels = 64 * 48 - 1);
            let a = r.images.serve(&request(&[]), "images", "a.jpg");
            assert_eq!(header(&a, "X-Luma-Decoder"), Some("file"));
            assert!(header(&a, "X-Luma-Reason").unwrap().contains("maxPixels"));
        }
    }

    #[test]
    fn the_picture_arrives_as_a_bottom_up_bmp_scaled_as_asked_decoded_on_the_cpu_without_the_hardware_library()
     {
        let dir = tempfile::tempdir().unwrap();
        let Some(d) = wrapper(dir.path(), false) else {
            return;
        };
        let r = rig(Some(d), |_| {});
        let full = r.images.serve(&request(&[]), "images", "a.jpg");
        assert_eq!(
            (
                full.status,
                header(&full, "Content-Type"),
                header(&full, "X-Luma-Decoder")
            ),
            (200, Some("image/bmp"), Some("cpu"))
        );
        assert_eq!(header(&full, "X-Luma-Size"), Some("64x48"));
        assert_eq!(full.body.len(), BMP_HEADER + 64 * 48 * 4);
        assert_eq!((word(&full.body, 18), word(&full.body, 22)), (64, 48));
        // red rises down the picture: the top row is less red than the bottom one, and the file stores the bottom one first
        let (top, bottom) = (bmp_pixel(&full.body, 0), bmp_pixel(&full.body, 47));
        assert!(
            i32::from(bottom.0) - i32::from(top.0) > 100,
            "top {top:?} bottom {bottom:?}"
        );
        assert!(
            full.body[BMP_HEADER + 2] > full.body[BMP_HEADER + (47 * 64 * 4) + 2],
            "the first row stored is the picture's last"
        );
        let half = r
            .images
            .serve(&request(&[("w", "32"), ("h", "24")]), "images", "a.jpg");
        assert_eq!(header(&half, "X-Luma-Size"), Some("32x24"));
        let eighth = r
            .images
            .serve(&request(&[("w", "1"), ("h", "1")]), "images", "a.jpg");
        assert_eq!(header(&eighth, "X-Luma-Size"), Some("8x6"));
        let s = r.images.status();
        assert_eq!(
            (
                s["images"]["served"]["cpu"].as_u64(),
                s["images"]["served"]["hardware"].as_u64()
            ),
            (Some(3), Some(0))
        );
        assert_eq!(s["images"]["running"], true);
    }

    #[test]
    fn with_the_library_the_hardware_gives_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let Some(d) = wrapper(dir.path(), true) else {
            return;
        };
        let r = rig(Some(d), |_| {});
        let a = r.images.serve(&request(&[("w", "32")]), "images", "a.jpg");
        assert_eq!(
            (
                a.status,
                header(&a, "X-Luma-Decoder"),
                header(&a, "X-Luma-Size")
            ),
            (200, Some("hardware"), Some("32x24"))
        );
        assert_eq!(r.images.status()["images"]["served"]["hardware"], 1);
    }

    #[test]
    fn a_decoder_that_ended_by_itself_is_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let Some(d) = wrapper(dir.path(), false) else {
            return;
        };
        let r = rig(Some(d), |c| {
            c.images.decoder_args = vec!["-x".into(), "300".into()]
        });
        assert_eq!(
            header(
                &r.images.serve(&request(&[]), "images", "a.jpg"),
                "X-Luma-Decoder"
            ),
            Some("cpu")
        );
        std::thread::sleep(Duration::from_millis(900)); // the decoder ends after 300 ms without a request
        assert_eq!(
            header(
                &r.images.serve(&request(&[]), "images", "a.jpg"),
                "X-Luma-Decoder"
            ),
            Some("cpu")
        );
        assert_eq!(r.images.status()["images"]["served"]["file"], 0);
    }

    #[test]
    fn a_decoder_that_answers_nonsense_is_a_file_and_then_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dec-bad");
        std::fs::write(&path, "#!/bin/sh\ncat > /dev/null\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = rig(Some(path.display().to_string()), |_| {});
        let a = r.images.serve(&request(&[]), "images", "a.jpg");
        assert_eq!(header(&a, "X-Luma-Decoder"), Some("file"));
        assert_eq!(a.body, PICTURE);
        assert_eq!(r.images.status()["images"]["backingOff"], true);
    }

    #[test]
    fn a_status_from_the_decoder_is_a_file_and_the_decoder_stays() {
        let dir = tempfile::tempdir().unwrap();
        let Some(d) = wrapper(dir.path(), false) else {
            return;
        };
        let r = rig(Some(d), |_| {});
        // a file whose frame header parses and whose tables are cut off: libjpeg cannot decode it
        let mut broken = PICTURE[..PICTURE.len().min(400)].to_vec();
        for b in broken.iter_mut().skip(330) {
            *b = 0xaa;
        }
        std::fs::write(r.dir.path().join("images/broken.jpg"), &broken).unwrap();
        let a = r.images.serve(&request(&[]), "images", "broken.jpg");
        assert_eq!(a.status, 200);
        // either libjpeg shows what it could (a truncated picture is a picture) or it refuses; neither takes the decoder down
        assert!(matches!(header(&a, "X-Luma-Decoder"), Some("cpu" | "file")));
        assert_eq!(
            header(
                &r.images.serve(&request(&[]), "images", "a.jpg"),
                "X-Luma-Decoder"
            ),
            Some("cpu")
        );
        assert_eq!(r.images.status()["images"]["backingOff"], false);
    }

    #[test]
    fn the_config_checks_the_images_section() {
        assert!(Config::parse(r#"{"images":{"roots":{"posters":"/data/p"},"decoderArgs":["-A","64"],"maxPixels":1000}}"#).is_ok());
        for bad in [
            r#"{"images":{"decoder":"lumatv-jpeg"}}"#,
            r#"{"images":{"decoderArgs":[""]}}"#,
            r#"{"images":{"roots":{"Bad":"/x"}}}"#,
            r#"{"images":{"roots":{"ok":"relative"}}}"#,
            r#"{"images":{"maxPixels":0}}"#,
            r#"{"images":{"maxPixels":67108865}}"#,
            r#"{"images":{"surprise":1}}"#,
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }
}
