//! Test support: a [`Ui`] whose compositor is a thread that answers the handshake and writes every request down, with the layers'
//! pixel files in a private directory so that image layers (text, icons) can be made and looked at. Components are tested by what
//! they ask of the compositor and by the pixels of what they draw, not by their fields.
//!
//! Compiled for the crate's own tests and, with the `testing` feature, for the tests of the crates that build on it.

use crate::context::Ui;
use crate::error::UiError;
use crate::scene::Scene;
use crate::text::Type;
use compositor_protocol::{
    AtlasId, AtlasRect, Content, Event, LayerId, PROTOCOL_VERSION, Props, Request, image_path,
    read_frame, write_frame,
};
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// A list shared between the recording thread and the test.
type Shared<T> = std::sync::Arc<std::sync::Mutex<T>>;

/// A scratch directory that removes itself when the test ends.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lumashow-ui-test-{}-{}-{tag}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A [`Ui`] and what its compositor was told.
pub struct Harness {
    pub ui: Ui,
    recorded: Shared<Vec<Request>>,
    /// How much of the recording a caller has already been given by [`Harness::take`].
    taken: usize,
    dir: TempDir,
    /// What each atlas upload carried when it was committed, by generation: the staging slot is written again after the upload
    /// is answered, so a recording that is played later needs these.
    uploads: Vec<Upload>,
    /// The answer every upload committed so far is waiting for ([`Harness::settle_atlas`] gives them).
    unanswered: Vec<Event>,
}

/// One chunk of an atlas as the app handed it over.
struct Upload {
    atlas: AtlasId,
    generation: u64,
    rect: AtlasRect,
    bytes: Vec<u8>,
}

/// A UI on a recording compositor.
pub fn harness() -> Harness {
    let dir = TempDir::new("scene");
    let (app, mut compositor) = UnixStream::pair().expect("a socket pair");
    let recorded: Shared<Vec<Request>> = Shared::default();
    let sink = Shared::clone(&recorded);
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = read_frame::<Request>(&mut compositor, &mut buf);
        let _ = write_frame(
            &mut compositor,
            &Event::Welcome {
                version: PROTOCOL_VERSION,
                panel_width: 480,
                panel_height: 960,
                refresh_hz: 59.64,
                smooth_first_full: true,
            },
        );
        while let Ok(Some(request)) = read_frame::<Request>(&mut compositor, &mut buf) {
            sink.lock().expect("the recording").push(request);
        }
    });
    let (scene, _welcome, _events) =
        Scene::over_in(app, dir.path().to_owned()).expect("the handshake");
    Harness {
        ui: Ui::new(scene),
        recorded,
        taken: 0,
        dir,
        uploads: Vec::new(),
        unanswered: Vec::new(),
    }
}

impl Harness {
    /// Commits what the UI has asked and waits until the recording compositor has all of it; returns the requests of that commit,
    /// the commit itself last.
    pub fn commit(&mut self) -> Vec<Request> {
        let tag = loop {
            match self.ui.scene.try_commit() {
                Ok(tag) => break tag.get(),
                Err(crate::SubmitError::WouldBlock) => {
                    self.ui
                        .scene
                        .wait_for_room(Duration::from_secs(5))
                        .expect("the compositor reads");
                }
                Err(e) => panic!("the commit goes out: {e}"),
            }
        };
        let start = Instant::now();
        loop {
            {
                let recorded = self.recorded.lock().expect("the recording");
                if recorded
                    .iter()
                    .any(|r| matches!(r, Request::Commit { tag: t } if *t == tag))
                {
                    break;
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the compositor never saw commit {tag}"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let sent = self.take();
        for request in &sent {
            if let Request::AtlasUpload {
                id,
                slot,
                generation,
                rect,
            } = request
            {
                self.unanswered.push(Event::AtlasUploaded {
                    id: *id,
                    slot: *slot,
                    generation: *generation,
                });
                let path = compositor_protocol::resources::staging_path(*id, *slot);
                let file = Path::new(&path).file_name().expect("a file name");
                if let Ok(bytes) = std::fs::read(self.dir.path().join(file)) {
                    let wanted = rect.bytes() as usize;
                    self.uploads.push(Upload {
                        atlas: *id,
                        generation: *generation,
                        rect: *rect,
                        bytes: bytes[..wanted.min(bytes.len())].to_vec(),
                    });
                }
            }
        }
        sent
    }

    /// The requests recorded since the last call.
    pub fn take(&mut self) -> Vec<Request> {
        let recorded = self.recorded.lock().expect("the recording");
        let new = recorded[self.taken..].to_vec();
        self.taken = recorded.len();
        new
    }

    /// Every request ever recorded.
    pub fn all(&self) -> Vec<Request> {
        self.recorded.lock().expect("the recording").clone()
    }

    /// The directory the layers' pixel files and the atlases' staging files are in.
    pub fn shm_dir(&self) -> &Path {
        self.dir.path()
    }

    /// The pixels of image layer `id` as the app last wrote them (premultiplied RGBA).
    pub fn pixels(&self, id: LayerId) -> Vec<u8> {
        let path = image_path(id);
        let name = Path::new(&path).file_name().expect("a file name");
        std::fs::read(self.dir.path().join(name)).expect("the layer's pixels")
    }

    /// The pixels layer `id` shows, whatever it is made of: a picture's own, or for a glyph run its quads blended in ink order
    /// from the atlas as every upload committed so far left it, as the compositor draws a run at scale 1 on whole pixels
    /// (premultiplied RGBA, the layer's size). What a test compares a run with a picture by.
    pub fn layer_pixels(&self, id: LayerId) -> Vec<u8> {
        let recorded = self.recorded.lock().expect("the recording");
        let (mut size, mut style, mut quads) = (None, None, Vec::new());
        let mut atlas_size = std::collections::HashMap::new();
        for r in recorded.iter() {
            match r {
                Request::Create {
                    id: made,
                    width,
                    height,
                    content,
                    ..
                } if *made == id => {
                    // A layer made again (the compositor replaces it) is its new content only: a picture that took a run's
                    // place has no quads and no style.
                    size = Some((*width as usize, *height as usize));
                    style = match content {
                        Content::Glyphs(s) => Some(*s),
                        _ => None,
                    };
                    quads.clear();
                }
                Request::Restyle {
                    id: restyled,
                    content: Content::Glyphs(s),
                } if *restyled == id => style = Some(*s),
                Request::SetGlyphs {
                    id: set, glyphs, ..
                } if *set == id => {
                    quads.clone_from(glyphs);
                }
                Request::AtlasCreate { id, width, height } => {
                    atlas_size.insert(*id, (*width as usize, *height as usize));
                }
                _ => {}
            }
        }
        drop(recorded);
        let (Some(style), Some((w, h))) = (style, size) else {
            return self.pixels(id);
        };
        let (aw, ah) = atlas_size[&style.atlas];
        let mut texture = vec![0u8; aw * ah];
        for up in self.uploads.iter().filter(|u| u.atlas == style.atlas) {
            let rect = up.rect;
            for row in 0..rect.h as usize {
                let from = row * rect.w as usize;
                let to = (rect.y as usize + row) * aw + rect.x as usize;
                texture[to..][..rect.w as usize]
                    .copy_from_slice(&up.bytes[from..][..rect.w as usize]);
            }
        }
        let mut out = vec![0u8; w * h * 4];
        for q in &quads {
            for row in 0..q.h as usize {
                for col in 0..q.w as usize {
                    let (x, y) = (
                        q.x.round() as i64 + col as i64,
                        q.y.round() as i64 + row as i64,
                    );
                    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                        continue;
                    }
                    let a = texture[(q.sy as usize + row) * aw + q.sx as usize + col];
                    if a != 0 {
                        crate::text::blend(
                            &mut out[(y as usize * w + x as usize) * 4..][..4],
                            style.palette[usize::from(q.ink)],
                            u32::from(a),
                        );
                    }
                }
            }
        }
        out
    }

    /// The content and last-known properties of every layer created, by id, as the requests so far leave them (a `Set` or the
    /// end state of an `Animate` replaces the props; a `Restyle` replaces the content).
    pub fn layers(&self) -> Vec<(LayerId, Content, Props)> {
        let mut out: Vec<(LayerId, Content, Props)> = Vec::new();
        for r in self.recorded.lock().expect("the recording").iter() {
            match r {
                Request::Create {
                    id, content, props, ..
                } => {
                    // The compositor replaces a layer that is made again, and puts it above the layers of its depth made before.
                    out.retain(|l| l.0 != *id);
                    out.push((*id, *content, *props));
                }
                Request::Set { id, props } | Request::Animate { id, to: props, .. } => {
                    if let Some(l) = out.iter_mut().find(|l| l.0 == *id) {
                        l.2 = *props;
                    }
                }
                Request::Restyle { id, content } => {
                    if let Some(l) = out.iter_mut().find(|l| l.0 == *id) {
                        l.1 = *content;
                    }
                }
                Request::Delete { id } => out.retain(|l| l.0 != *id),
                _ => {}
            }
        }
        out
    }

    /// Writes every request the UI has made, in order, as a recording the compositor's replay tool can draw, with the pixel files
    /// of every image layer: for a scene built of native groups and motion tracks, which `save_stream`'s summary of flat layers
    /// cannot say. The replay plays the requests through a real session and draws the result once, animations landed.
    pub fn save_recording(&self, dir: &Path, name: &str) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let shm = dir.join(format!("{name}.shm"));
        std::fs::create_dir_all(&shm)?;
        for entry in std::fs::read_dir(self.dir.path())? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                std::fs::copy(entry.path(), shm.join(entry.file_name()))?;
            }
        }
        for up in &self.uploads {
            // A generation counts the uploads of one atlas: two atlases (the text atlas and a screen's own) both have a first.
            std::fs::write(
                shm.join(format!("upload-{}-{}.a8", up.atlas.0, up.generation)),
                &up.bytes,
            )?;
        }
        let mut out = std::fs::File::create(dir.join(format!("{name}.stream")))?;
        write_frame(
            &mut out,
            &Request::Hello {
                version: PROTOCOL_VERSION,
            },
        )?;
        for r in self.recorded.lock().expect("the recording").iter() {
            if !matches!(r, Request::Hello { .. }) {
                write_frame(&mut out, r)?;
            }
        }
        Ok(())
    }

    /// Writes what is on screen as a recording the compositor's replay tool can draw (`crates/compositor/src/replay_tests.rs`):
    /// `name.stream` holds one `Create` for each layer that is alive, in creation order, with its content and properties as they
    /// stand (animations landed), then a commit; `name.shm/` holds the pixel files of the layers that have pixels. Nothing here
    /// depends on how the screen got that way.
    pub fn save_stream(&self, dir: &Path, name: &str) -> io::Result<()> {
        struct Live {
            id: LayerId,
            width: u32,
            height: u32,
            z: i32,
            content: Content,
            props: Props,
        }
        let mut live: Vec<Live> = Vec::new();
        for r in self.recorded.lock().expect("the recording").iter() {
            match r {
                Request::Create {
                    id,
                    width,
                    height,
                    z,
                    content,
                    props,
                } => {
                    live.retain(|l| l.id != *id);
                    live.push(Live {
                        id: *id,
                        width: *width,
                        height: *height,
                        z: *z,
                        content: *content,
                        props: *props,
                    });
                }
                Request::Set { id, props } | Request::Animate { id, to: props, .. } => {
                    if let Some(l) = live.iter_mut().find(|l| l.id == *id) {
                        l.props = *props;
                    }
                }
                Request::Restyle { id, content } => {
                    if let Some(l) = live.iter_mut().find(|l| l.id == *id) {
                        l.content = *content;
                    }
                }
                Request::Delete { id } => live.retain(|l| l.id != *id),
                _ => {}
            }
        }
        std::fs::create_dir_all(dir)?;
        let shm = dir.join(format!("{name}.shm"));
        std::fs::create_dir_all(&shm)?;
        let mut out = std::fs::File::create(dir.join(format!("{name}.stream")))?;
        write_frame(
            &mut out,
            &Request::Hello {
                version: PROTOCOL_VERSION,
            },
        )?;
        for l in &live {
            let path = image_path(l.id);
            let file = Path::new(&path).file_name().expect("a file name");
            let source = self.dir.path().join(file);
            if source.exists() {
                std::fs::copy(&source, shm.join(file))?;
            }
            write_frame(
                &mut out,
                &Request::Create {
                    id: l.id,
                    width: l.width,
                    height: l.height,
                    z: l.z,
                    content: l.content,
                    props: l.props,
                },
            )?;
        }
        write_frame(&mut out, &Request::Commit { tag: 1 })
    }
}

impl Harness {
    /// Answers every atlas upload committed so far as the compositor does, and commits again, answering those too, until every
    /// atlas has nothing waiting and nothing in flight (the text page and the older pages runs still draw from): every glyph
    /// handed over is ready to draw. (An answer frees a staging slot, which stages the next band for the commit after it.)
    pub fn settle_atlas(&mut self) {
        for _ in 0..400 {
            let answers = std::mem::take(&mut self.unanswered);
            let none_in_flight = answers.is_empty();
            for answer in answers {
                self.ui.scene.observe(&answer);
            }
            self.commit();
            if none_in_flight && self.ui.scene.atlases_settled() {
                return;
            }
        }
        panic!("the atlas never settled");
    }

    /// Fills the text page (`Ui::text_atlas`) with glyphs nobody asked for until it has no room for another of `ty`, and lets it
    /// settle. Returns a character that does not fit now: a text with it at `ty` cannot be shown from the page (and will not be
    /// able to, whatever else is prepared). Big glyphs go first, since they fill the shelves fast, then smaller down to `ty`, to
    /// fill what they left. The font draws the same box for every character it has no glyph of, so the characters are the CJK
    /// ideographs: as many as the page can take, each with a place of its own.
    pub fn fill_text_page(&mut self, ty: Type) -> char {
        let page = self
            .ui
            .text_atlas
            .as_ref()
            .expect("glyph text is on")
            .view();
        let mut chars = (0x4e00u32..).filter_map(char::from_u32);
        for px in [200, 100, 50, 25, ty.px] {
            let at = Type { px, ..ty };
            loop {
                let c = chars.next().expect("characters left");
                match page.prepare(&mut self.ui.fonts, at, &c.to_string()) {
                    Ok(()) => {}
                    Err(UiError::AtlasFull) if px == ty.px => {
                        page.pump();
                        self.settle_atlas();
                        return c;
                    }
                    Err(UiError::AtlasFull) => break,
                    Err(e) => panic!("filling the page: {e}"),
                }
            }
        }
        unreachable!("the last size fills the page or loops")
    }
}

/// A UI with glyph text on whose atlas `build` has already been shown and landed, as one that showed the page before is: `build`
/// makes whatever the test will make again (everything it will say, in every colour), and it is dropped. What the test then
/// builds finds its glyphs ready, so a layer of text is a run at once.
pub fn warm_glyph_harness<T>(build: impl FnOnce(&mut Harness) -> T) -> Harness {
    let mut h = harness();
    h.ui.enable_glyph_text().expect("glyph text");
    let first = build(&mut h);
    h.settle_atlas();
    drop(first);
    h.commit();
    h
}

/// How many requests of `sent` are `f`.
pub fn count_requests(sent: &[Request], f: impl Fn(&Request) -> bool) -> usize {
    sent.iter().filter(|r| f(r)).count()
}

/// How many pictures (image layers) `sent` makes.
pub fn pictures_made(sent: &[Request]) -> usize {
    count_requests(sent, |r| {
        matches!(
            r,
            Request::Create {
                content: Content::Image,
                ..
            }
        )
    })
}

/// The area, in pixels, `sent` tells the compositor was written.
pub fn pixels_damaged(sent: &[Request]) -> f32 {
    sent.iter()
        .map(|r| match r {
            Request::Damage { rect, .. } => rect.w * rect.h,
            _ => 0.0,
        })
        .sum()
}

/// Whether `sent` makes layer `id` a picture.
pub fn made_as_picture(sent: &[Request], id: LayerId) -> bool {
    sent.iter().any(|r| {
        matches!(
            r,
            Request::Create { id: made, content: Content::Image, .. } if *made == id
        )
    })
}

/// The area, in pixels, `sent` tells the compositor was written in layer `id`.
pub fn pixels_damaged_in(sent: &[Request], id: LayerId) -> f32 {
    sent.iter()
        .map(|r| match r {
            Request::Damage { id: hit, rect } if *hit == id => rect.w * rect.h,
            _ => 0.0,
        })
        .sum()
}

/// A contact of finger `id` as the host's input delivers it, `at_ms` milliseconds on the frame clock.
pub fn touch(
    id: u32,
    phase: crate::input::TouchPhase,
    x: f32,
    y: f32,
    at_ms: u64,
) -> crate::input::Event {
    crate::input::Event::Touch(crate::input::TouchEvent {
        id,
        phase,
        x,
        y,
        at_ns: at_ms * 1_000_000,
    })
}
