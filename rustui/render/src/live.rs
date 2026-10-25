//! Live sources: the pictures a producer fills and the GPU draws without a copy (`compositor_protocol::frames`).
//!
//! A source is a few buffers (dma-bufs) of one size. Each of them is put on the GPU once, through the device's [`Importer`]: from
//! then on the texture of a buffer shows whatever its memory holds, so a picture costs the compositor the name of a buffer
//! ([`Sources::frame`]) and one more draw of a textured quad, and the CPU never reads, writes or uploads a pixel of it. The buffer
//! that was shown until then goes back to its producer ([`Event::Released`]) the moment another replaces it: the render thread
//! deals with one frame at a time, so the frame that drew the old one is over before the next one's changes are applied, and
//! nothing is reading it any more.
//!
//! **None of the work of a source's beginning or end is done in a frame.** An attach ([`Sources::attach`]) only takes the
//! descriptors; [`Sources::tidy`], which the render thread calls while a frame is on the GPU or nothing is due, puts the buffers on
//! the GPU one at a time and draws each of them once, off screen, before the source is announced ([`Event::Attached`]): whatever
//! the driver does the first time it is made to read memory it has not read, and whatever the import costs, is paid there, and the
//! frame that shows the first picture is a frame like any other. The end of a source is the same: a detach (or an attach that
//! replaces the source) takes it from the layers at once, and the GPU lets go of its buffers in the housekeeping after.
//!
//! The importer is the one part that depends on the device. On the Echo Show the EGL blob has no dma-buf import, only "the buffer
//! an `ANativeWindowBuffer` names", which for a gralloc buffer is the dma-buf in the `share_fd` slot of its handle
//! ([`GrallocImporter`], the same trick as `scanout::direct`); the tests stand a texture's own storage in for the producer's
//! memory (`alias`), which they fill the way a camera's DMA fills a dma-buf, from outside the code under test.

use crate::gl::Gles;
use crate::log::log;
use compositor_protocol::SourceId;
use compositor_protocol::frames::{Event, Format, Spec};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A buffer on the GPU: the texture that shows it, and how to let go of everything made for it.
pub struct Imported {
    pub texture: u32,
    release: Box<dyn FnOnce(&Gles)>,
}

impl Imported {
    /// `texture` shows the buffer; `release` undoes the import (on the render thread, with the GL current).
    pub fn new(texture: u32, release: impl FnOnce(&Gles) + 'static) -> Self {
        Self {
            texture,
            release: Box::new(release),
        }
    }

    fn free(self, gl: &Gles) {
        (self.release)(gl);
    }
}

/// How the device puts a producer's buffer on the GPU.
pub trait Importer: Send {
    /// A texture of `spec`'s size that shows the memory `fd` names as it is at each draw, with nothing copied. The importer
    /// owns `fd` from here and keeps it open for as long as the GPU may read the memory (until the result is released).
    fn import(&mut self, gl: &Gles, spec: &Spec, fd: OwnedFd) -> io::Result<Imported>;
}

struct Source {
    buffers: Vec<Imported>,
    /// The buffer the layers show, which is the compositor's until it is replaced.
    shown: Option<u8>,
}

/// An attach whose buffers are not all on the GPU yet: what [`Sources::tidy`] is working through.
struct Pending {
    source: SourceId,
    spec: Spec,
    /// The descriptors still to be put on the GPU, the next one first.
    fds: VecDeque<OwnedFd>,
    /// The buffers on the GPU so far, in order.
    made: Vec<Imported>,
    /// When the attach reached the render thread, and what putting the buffers on the GPU and drawing each of them once have taken
    /// so far.
    began: Instant,
    import: Duration,
    first_draw: Duration,
}

/// What the sources have done, for the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Buffers put on the GPU, over the compositor's life.
    pub imports: u64,
    /// Buffers drawn once, off screen, after they were put there.
    pub first_draws: u64,
    /// Buffers the GPU has let go of.
    pub frees: u64,
    /// Pictures that became the one drawn.
    pub flips: u64,
}

/// What one call of [`Sources::tidy`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tidied {
    /// Buffers put on the GPU (and drawn once) or let go of.
    pub buffers: u32,
    /// There is more for the next call.
    pub more: bool,
}

/// Every source a producer has made, and what they have to tell their producers.
pub struct Sources {
    importer: Option<Box<dyn Importer>>,
    sources: HashMap<SourceId, Source>,
    /// Attaches whose buffers are not all on the GPU yet, oldest first.
    pending: VecDeque<Pending>,
    /// Buffers of sources that are over, to be let go of when nothing waits ([`Sources::tidy`]).
    graveyard: Vec<Imported>,
    events: Vec<(SourceId, Event)>,
    counts: Counts,
}

impl Sources {
    /// `importer` is `None` on a device that cannot put another process's memory on the GPU: its sources are refused.
    pub fn new(importer: Option<Box<dyn Importer>>) -> Self {
        Self {
            importer,
            sources: HashMap::new(),
            pending: VecDeque::new(),
            graveyard: Vec::new(),
            events: Vec::new(),
            counts: Counts::default(),
        }
    }

    #[cfg(test)]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Takes `fds` as the buffers of the source, in order, and nothing more: the GPU gets them in [`Sources::tidy`], and the
    /// producer is told `Attached` when the last is there, or `Refused` when the GPU cannot take one. A source the producer already
    /// has (or is waiting for) is replaced: the layers show nothing of it from now on.
    pub fn attach(&mut self, source: SourceId, spec: Spec, fds: Vec<OwnedFd>) {
        self.drop_source(source);
        if self.importer.is_none() {
            return self.refuse(source, "this device cannot show a producer's frames".into());
        }
        self.pending.push_back(Pending {
            source,
            spec,
            fds: fds.into(),
            made: Vec::new(),
            began: Instant::now(),
            import: Duration::ZERO,
            first_draw: Duration::ZERO,
        });
    }

    /// The housekeeping the render thread does while a frame is on the GPU or nothing is due, for at most `budget` (a call deals
    /// with at least one buffer): lets go of the buffers of sources that are over, then puts the buffers of the attaches on the
    /// GPU, one at a time, and has `draw_once` draw each (given the picture's size and its texture) off screen before the next. A
    /// source is announced when its last buffer has been drawn; one the GPU cannot take is refused, and what it did take is let go
    /// of.
    pub fn tidy(
        &mut self,
        gl: &Gles,
        budget: Duration,
        draw_once: &mut dyn FnMut(&Gles, &Spec, u32),
    ) -> Tidied {
        let began = Instant::now();
        let mut done = Tidied::default();
        let spent = |done: &Tidied| done.buffers > 0 && began.elapsed() >= budget;
        // What sources that are over held goes first: the gralloc buffers they stood in for are what the next attach goes into.
        while !self.graveyard.is_empty() {
            if spent(&done) {
                done.more = true;
                return done;
            }
            if let Some(buffer) = self.graveyard.pop() {
                buffer.free(gl);
                self.counts.frees += 1;
                done.buffers += 1;
            }
        }
        while let Some(mut pending) = self.pending.pop_front() {
            let Some(fd) = pending.fds.pop_front() else {
                self.ready(pending);
                continue;
            };
            if spent(&done) {
                pending.fds.push_front(fd);
                self.pending.push_front(pending);
                done.more = true;
                return done;
            }
            let index = pending.made.len();
            let imported = match self.importer.as_mut() {
                Some(importer) => {
                    let at = Instant::now();
                    let imported = importer.import(gl, &pending.spec, fd);
                    pending.import += at.elapsed();
                    imported
                }
                None => Err(io::Error::other(
                    "this device cannot show a producer's frames",
                )),
            };
            match imported {
                Ok(imported) => {
                    self.counts.imports += 1;
                    // What was handed to the GPU before is waited for, so that what is timed is this buffer's first draw alone.
                    gl.finish();
                    let at = Instant::now();
                    draw_once(gl, &pending.spec, imported.texture);
                    gl.finish();
                    pending.first_draw += at.elapsed();
                    self.counts.first_draws += 1;
                    pending.made.push(imported);
                    done.buffers += 1;
                    self.pending.push_front(pending);
                }
                Err(e) => {
                    for made in pending.made {
                        made.free(gl);
                        self.counts.frees += 1;
                    }
                    self.refuse(
                        pending.source,
                        format!("buffer {index} could not be put on the GPU: {e}"),
                    );
                }
            }
        }
        done
    }

    /// Every buffer of the attach is on the GPU: the source exists, and its producer is told.
    fn ready(&mut self, pending: Pending) {
        log(format!(
            "live source {}: {} buffers of {}x{} (rows of {} bytes) are on the GPU {:.2} ms after the attach (put there in {:.2} ms, \
             each drawn once off screen in {:.2} ms, between frames)",
            pending.source.0,
            pending.made.len(),
            pending.spec.width,
            pending.spec.height,
            pending.spec.stride,
            pending.began.elapsed().as_secs_f64() * 1e3,
            pending.import.as_secs_f64() * 1e3,
            pending.first_draw.as_secs_f64() * 1e3,
        ));
        let source = pending.source;
        self.sources.insert(
            source,
            Source {
                buffers: pending.made,
                shown: None,
            },
        );
        self.events.push((source, Event::Attached { source }));
    }

    /// Tells the producer that the source it asked for was not made.
    pub fn refuse(&mut self, source: SourceId, message: String) {
        log(format!("live source {}: refused: {message}", source.0));
        self.events.push((
            source,
            Event::Refused {
                source: Some(source),
                message,
            },
        ));
    }

    /// `buffer` is the picture the layers show from now on; the one they showed goes back to the producer. A source or a
    /// buffer the GPU does not have (the attach is not done, or was refused: the producer was told) is ignored.
    pub fn frame(&mut self, source: SourceId, buffer: u8) {
        let Some(known) = self.sources.get_mut(&source) else {
            return;
        };
        if usize::from(buffer) >= known.buffers.len() {
            return;
        }
        match known.shown.replace(buffer) {
            Some(old) if old != buffer => self.events.push((
                source,
                Event::Released {
                    source,
                    buffer: old,
                },
            )),
            _ => {}
        }
        self.counts.flips += 1;
    }

    /// The layers show nothing until the next picture; the picture they showed goes back to the producer.
    pub fn clear(&mut self, source: SourceId) {
        let Some(known) = self.sources.get_mut(&source) else {
            return;
        };
        if let Some(old) = known.shown.take() {
            self.events.push((
                source,
                Event::Released {
                    source,
                    buffer: old,
                },
            ));
        }
    }

    /// The source is over: the layers show nothing of it from now on, and the GPU lets go of its buffers (which the producer may
    /// use again) in the housekeeping, not here.
    pub fn detach(&mut self, source: SourceId) {
        if self.drop_source(source) {
            self.events.push((source, Event::Detached { source }));
        }
    }

    /// Takes the source (made, or still being made) away from the layers: the buffers it has on the GPU wait to be let go of, the
    /// descriptors it has not used are closed. Whether there was one.
    fn drop_source(&mut self, source: SourceId) -> bool {
        let mut was = false;
        if let Some(known) = self.sources.remove(&source) {
            self.graveyard.extend(known.buffers);
            was = true;
        }
        if let Some(at) = self.pending.iter().position(|p| p.source == source) {
            if let Some(pending) = self.pending.remove(at) {
                self.graveyard.extend(pending.made);
                was = true;
            }
        }
        was
    }

    /// Whether [`Sources::tidy`] has something to do.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || !self.graveyard.is_empty()
    }

    /// The texture of the picture the layers of `source` show, if it has one.
    pub fn texture(&self, source: SourceId) -> Option<u32> {
        let known = self.sources.get(&source)?;
        known
            .buffers
            .get(usize::from(known.shown?))
            .map(|b| b.texture)
    }

    /// What the sources have to say to their producers, oldest first.
    pub fn take_events(&mut self) -> Vec<(SourceId, Event)> {
        std::mem::take(&mut self.events)
    }
}

/// LumaTV has no producer of frames yet, so no importer: a source that is attached is refused, as on a device without one.
pub fn native_importer() -> Option<Box<dyn Importer>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::take_uploads;
    use crate::ops::Op;
    use crate::scene::Drawn;
    use crate::testgl::{Rig, drawn, rig};
    use compositor_protocol::{Content, Fill, Live, Props, Rgba, Shape};
    use std::fs::File;
    use std::sync::atomic::{AtomicU32, Ordering};

    const SPEC: Spec = Spec {
        width: 40,
        height: 30,
        stride: 160,
        format: Format::Rgba8888,
    };
    const SOURCE: SourceId = SourceId(3);
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    /// What the importer's buffers hold when they are made, in the order they are made (three, then the same again): the producer has
    /// not written them yet, and a draw of one is known by its colour.
    const MADE_AS: [[u8; 4]; 3] = [RED, GREEN, BLUE];
    const GROUND: [u8; 4] = [10, 20, 30, 255];
    /// Where the live layer is drawn, and a pixel in the middle of it.
    const AT: (f32, f32) = (100.0, 50.0);
    const INSIDE: (u32, u32) = (200, 100);

    /// What the tests' importer made and let go of.
    #[derive(Clone, Default)]
    struct Memory {
        /// The texture behind each buffer, in the order they were imported: what a producer writes (the camera's DMA).
        textures: Arc<Mutex<Vec<u32>>>,
        freed: Arc<AtomicU32>,
        /// `import N` and `free N` (N counts the buffers from 0 in the order they were imported), in the order they happened.
        log: Arc<Mutex<Vec<String>>>,
    }

    /// Each buffer is a texture of its own that the test fills, and the renderer reads through an alias of it
    /// (`EGL_KHR_gl_texture_2D_image`): the same `glEGLImageTargetTexture2DOES` the Show's import ends in, over memory the
    /// renderer never writes. `fail_from` is the buffer (counting from 0) whose import fails.
    struct Aliasing {
        memory: Memory,
        fail_from: Option<usize>,
    }

    impl Importer for Aliasing {
        fn import(&mut self, gl: &Gles, spec: &Spec, _fd: OwnedFd) -> io::Result<Imported> {
            let index = self.memory.textures.lock().len();
            if self.fail_from == Some(index) {
                return Err(io::Error::other("the blob does not take it"));
            }
            let backing = gl.new_texture(spec.width, spec.height);
            let made_as =
                MADE_AS[index % MADE_AS.len()].repeat((spec.width * spec.height) as usize);
            gl.bind_texture(backing);
            // SAFETY: `made_as` holds the w x h block that is uploaded.
            unsafe { gl.upload(0, 0, spec.width, spec.height, spec.width, made_as.as_ptr()) };
            let alias = gl.alias(backing)?;
            self.memory.textures.lock().push(backing);
            self.memory.log.lock().push(format!("import {index}"));
            let (freed, log) = (Arc::clone(&self.memory.freed), Arc::clone(&self.memory.log));
            Ok(Imported::new(alias.texture, move |gl| {
                gl.delete_import(alias);
                gl.delete_texture(backing);
                freed.fetch_add(1, Ordering::Relaxed);
                log.lock().push(format!("free {index}"));
            }))
        }
    }

    fn some_fd() -> OwnedFd {
        File::open("/dev/null").unwrap().into()
    }

    fn rig_with(importer: Option<Aliasing>) -> Option<Rig> {
        let mut rig = rig()?;
        rig.renderer
            .set_importer(importer.map(|i| Box::new(i) as Box<dyn Importer>));
        Some(rig)
    }

    fn working() -> (Memory, Option<Rig>) {
        let memory = Memory::default();
        let rig = rig_with(Some(Aliasing {
            memory: memory.clone(),
            fail_from: None,
        }));
        (memory, rig)
    }

    fn events(rig: &mut Rig) -> Vec<Event> {
        rig.renderer
            .drain_live_events()
            .into_iter()
            .map(|(source, event)| {
                assert_eq!(source, SOURCE);
                event
            })
            .collect()
    }

    /// What the frame that takes an attach does with it: applies it (nothing is on the GPU after).
    fn apply_attach(rig: &mut Rig, source: SourceId, buffers: usize) {
        rig.renderer.apply(&Op::LiveAttach {
            source,
            spec: SPEC,
            buffers: (0..buffers).map(|_| some_fd()).collect(),
        });
    }

    /// An attach, and the housekeeping that follows it until nothing is left: what the producer has been told by then.
    fn attach(rig: &mut Rig, buffers: usize) -> Vec<Event> {
        apply_attach(rig, SOURCE, buffers);
        rig.tidy_all();
        events(rig)
    }

    /// The producer fills buffer `index` (`pixels` is a colour for the whole picture, or the picture's every pixel): the camera's DMA, and
    /// not the compositor's work.
    fn write(rig: &Rig, memory: &Memory, index: usize, pixels: &[u8]) {
        let (w, h) = (SPEC.width, SPEC.height);
        let all = if pixels.len() == 4 {
            pixels.repeat((w * h) as usize)
        } else {
            pixels.to_vec()
        };
        let gl = rig.renderer.gles();
        gl.bind_texture(memory.textures.lock()[index]);
        // SAFETY: `all` holds the w x h block that is uploaded.
        unsafe { gl.upload(0, 0, w, h, w, all.as_ptr()) };
    }

    fn flip(rig: &mut Rig, buffer: u8) {
        rig.renderer.apply(&Op::LiveFrame {
            source: SOURCE,
            buffer,
        });
    }

    fn ground() -> Drawn {
        drawn(
            1,
            (960, 480),
            Content::Shape(Shape {
                corner_radius: 0.0,
                fill: Fill::Solid(Rgba(GROUND[0], GROUND[1], GROUND[2], 255)),
                border_width: 0.0,
                border: Rgba(0, 0, 0, 0),
            }),
            Props::at(0.0, 0.0),
        )
    }

    /// The live layer over a ground, at `version` of its life.
    fn scene(version: u64) -> [Drawn; 2] {
        let mut live = drawn(
            2,
            (200, 100),
            Content::Live(Live { source: SOURCE }),
            Props::at(AT.0, AT.1),
        );
        live.version = version;
        [ground(), live]
    }

    #[test]
    fn a_picture_is_drawn_from_the_buffer_it_is_in_and_a_frame_uploads_nothing() {
        let (memory, Some(mut rig)) = working() else {
            return;
        };
        assert!(matches!(&attach(&mut rig, 3)[..], [Event::Attached { .. }]));
        assert_eq!(rig.renderer.live_counts().imports, 3);
        // Until a picture comes the layer shows nothing.
        let shot = rig.draw(&scene(1));
        assert_eq!(shot.pixel(INSIDE.0, INSIDE.1), GROUND);
        rig.renderer.gles_mut().count_calls();
        for (index, colour) in [(0, RED), (1, BLUE), (2, GREEN), (0, BLUE)] {
            // The producer writes a buffer that is not the one on show...
            write(&rig, &memory, index, &colour);
            flip(&mut rig, index as u8);
            take_uploads();
            // ... and the frame that shows it is a draw of the buffer where it lies.
            let shot = rig.draw(&scene(2 + index as u64));
            assert_eq!(shot.pixel(INSIDE.0, INSIDE.1), colour, "buffer {index}");
            assert_eq!(take_uploads().calls, 0, "no pixel went to the GPU");
            assert_eq!(rig.report.uploaded_pixels, 0);
            assert_eq!(shot.pixel(20, 20), GROUND, "the rest is as it was");
        }
        assert_eq!(
            rig.renderer.take_frame_counts().1,
            0,
            "the frame events say none either"
        );
        assert_eq!(rig.renderer.live_counts().flips, 4);
        assert_eq!(rig.renderer.gles().take_error(), None);
    }

    #[test]
    fn the_picture_is_stretched_over_the_layer_and_is_the_whole_of_the_buffer() {
        let (memory, Some(mut rig)) = working() else {
            return;
        };
        attach(&mut rig, 1);
        // Left half red, right half blue, in a picture 40 x 30 over a layer of 200 x 100.
        let mut pixels = Vec::new();
        for _ in 0..SPEC.height {
            for x in 0..SPEC.width {
                pixels.extend_from_slice(&if x < SPEC.width / 2 { RED } else { BLUE });
            }
        }
        write(&rig, &memory, 0, &pixels);
        flip(&mut rig, 0);
        let shot = rig.draw(&scene(2));
        assert_eq!(shot.pixel(110, 60), RED, "near the layer's top left");
        assert_eq!(shot.pixel(290, 140), BLUE, "near its bottom right");
        assert_eq!(shot.pixel(99, 100), GROUND, "outside it");
        assert_eq!(shot.pixel(300, 100), GROUND);
    }

    #[test]
    fn the_buffer_that_was_shown_goes_back_to_the_producer_when_another_replaces_it() {
        let (_memory, Some(mut rig)) = working() else {
            return;
        };
        attach(&mut rig, 3);
        flip(&mut rig, 0);
        assert!(events(&mut rig).is_empty(), "nothing was shown before");
        flip(&mut rig, 1);
        let released = |buffer| Event::Released {
            source: SOURCE,
            buffer,
        };
        assert_eq!(events(&mut rig), [released(0)]);
        // Two pictures in a row with no frame between: the one that was never drawn is released too, in order.
        flip(&mut rig, 2);
        flip(&mut rig, 0);
        assert_eq!(events(&mut rig), [released(1), released(2)]);
        // The same buffer named again is no replacement.
        flip(&mut rig, 0);
        assert!(events(&mut rig).is_empty());
        // A buffer that does not exist is ignored.
        flip(&mut rig, 9);
        assert!(events(&mut rig).is_empty());
    }

    #[test]
    fn a_clear_shows_nothing_and_a_detach_lets_the_gpu_go_of_every_buffer() {
        let (memory, Some(mut rig)) = working() else {
            return;
        };
        attach(&mut rig, 3);
        write(&rig, &memory, 1, &RED);
        flip(&mut rig, 1);
        assert_eq!(rig.draw(&scene(2)).pixel(INSIDE.0, INSIDE.1), RED);
        rig.renderer.apply(&Op::LiveClear { source: SOURCE });
        assert_eq!(
            events(&mut rig),
            [Event::Released {
                source: SOURCE,
                buffer: 1
            }]
        );
        assert_eq!(
            rig.draw(&scene(3)).pixel(INSIDE.0, INSIDE.1),
            GROUND,
            "no picture of the camera stays on the screen"
        );
        // A picture after a clear is shown again.
        flip(&mut rig, 1);
        assert_eq!(rig.draw(&scene(4)).pixel(INSIDE.0, INSIDE.1), RED);
        assert_eq!(memory.freed.load(Ordering::Relaxed), 0);
        rig.renderer.apply(&Op::LiveDetach { source: SOURCE });
        assert_eq!(events(&mut rig), [Event::Detached { source: SOURCE }]);
        // What the frame that takes the detach does is take the source from the layers: the GPU lets go of the buffers after it, in the
        // housekeeping.
        assert_eq!(
            memory.freed.load(Ordering::Relaxed),
            0,
            "the frame frees nothing"
        );
        assert_eq!(rig.draw(&scene(5)).pixel(INSIDE.0, INSIDE.1), GROUND);
        rig.tidy_all();
        assert_eq!(memory.freed.load(Ordering::Relaxed), 3);
        // Nothing is left to detach or clear.
        rig.renderer.apply(&Op::LiveDetach { source: SOURCE });
        rig.renderer.apply(&Op::LiveClear { source: SOURCE });
        assert!(events(&mut rig).is_empty());
        assert_eq!(rig.renderer.gles().take_error(), None);
    }

    #[test]
    fn a_buffer_the_gpu_cannot_take_refuses_the_source_and_keeps_nothing() {
        let memory = Memory::default();
        let Some(mut rig) = rig_with(Some(Aliasing {
            memory: memory.clone(),
            fail_from: Some(2),
        })) else {
            return;
        };
        // The frame that takes the attach imports nothing and refuses nothing: the housekeeping does, and says so.
        apply_attach(&mut rig, SOURCE, 3);
        assert!(events(&mut rig).is_empty());
        rig.tidy_all();
        let told = events(&mut rig);
        let [Event::Refused { source, message }] = &told[..] else {
            panic!("{told:?}");
        };
        assert_eq!(*source, Some(SOURCE));
        assert!(
            message.contains("buffer 2") && message.contains("blob"),
            "{message}"
        );
        assert_eq!(
            memory.freed.load(Ordering::Relaxed),
            2,
            "the two it took were let go of"
        );
        // The source does not exist: pictures of it are ignored and nothing is shown.
        flip(&mut rig, 0);
        assert!(events(&mut rig).is_empty());
        assert_eq!(rig.draw(&scene(2)).pixel(INSIDE.0, INSIDE.1), GROUND);
    }

    #[test]
    fn a_device_that_cannot_put_memory_on_the_gpu_refuses_every_source() {
        let Some(mut rig) = rig_with(None) else {
            return;
        };
        let told = attach(&mut rig, 3);
        assert!(
            matches!(&told[..], [Event::Refused { message, .. }] if message.contains("cannot show")),
            "{told:?}"
        );
    }

    #[test]
    fn attaching_a_source_again_replaces_it_and_lets_go_of_the_old_buffers_without_a_detach() {
        let (memory, Some(mut rig)) = working() else {
            return;
        };
        attach(&mut rig, 3);
        write(&rig, &memory, 0, &RED);
        flip(&mut rig, 0);
        events(&mut rig);
        let told = attach(&mut rig, 2);
        assert!(matches!(&told[..], [Event::Attached { .. }]), "{told:?}");
        assert_eq!(memory.freed.load(Ordering::Relaxed), 3);
        // The old buffers are let go of before the new ones are put on the GPU: on the Show the gralloc buffers they stood in for
        // are what the new ones go into, and would otherwise be made again.
        let log = memory.log.lock().clone();
        let first_new = log.iter().position(|e| e == "import 3").expect("imported");
        assert_eq!(
            log[..first_new]
                .iter()
                .filter(|e| e.starts_with("free"))
                .count(),
            3,
            "{log:?}"
        );
        // The new source has no picture yet: the old one is not shown.
        assert_eq!(rig.draw(&scene(2)).pixel(INSIDE.0, INSIDE.1), GROUND);
        write(&rig, &memory, 3, &GREEN);
        flip(&mut rig, 0);
        assert_eq!(rig.draw(&scene(3)).pixel(INSIDE.0, INSIDE.1), GREEN);
    }

    #[test]
    fn a_picture_redraws_its_layer_and_nothing_else_and_a_standing_one_draws_nothing() {
        let (memory, Some(mut rig)) = working() else {
            return;
        };
        rig.ring(2);
        attach(&mut rig, 2);
        write(&rig, &memory, 0, &RED);
        flip(&mut rig, 0);
        let layer_px = 200 * 100;
        // What is redrawn is the layer's rectangle widened to the GPU's tiles of 16 pixels.
        let tiled_px = (200 + 16) * (100 + 16);
        // The first frames of each target draw everything.
        rig.draw_damaged(0, &scene(2));
        rig.draw_damaged(1, &scene(2));
        // A new picture is a new version of the layer (the scene makes it): the layer is redrawn, and the rest of the screen,
        // which has not changed, is not touched.
        write(&rig, &memory, 1, &BLUE);
        flip(&mut rig, 1);
        let shot = rig.draw_damaged(0, &scene(3));
        assert_eq!(shot.pixel(INSIDE.0, INSIDE.1), BLUE);
        assert!(
            (layer_px..=tiled_px).contains(&rig.report.region_px),
            "{} pixels of a panel of 460800",
            rig.report.region_px
        );
        let shot = rig.draw_damaged(1, &scene(3));
        assert_eq!(shot.pixel(INSIDE.0, INSIDE.1), BLUE);
        // Nothing came: nothing is drawn.
        rig.draw_damaged(0, &scene(3));
        assert_eq!(rig.report.region_px, 0);
        rig.draw_damaged(1, &scene(3));
        assert_eq!(rig.report.region_px, 0);
    }

    // What a source costs the frames at its beginning and its end (8 of 20 entries of a camera screen on the Show lost a refresh):
    // the tests below pin where the work is done, which is the housekeeping between frames and never a frame.

    #[test]
    fn the_frame_that_takes_an_attach_makes_nothing_and_the_housekeeping_puts_the_buffers_on_the_gpu_one_a_call()
     {
        let (memory, Some(mut rig)) = working() else {
            return;
        };
        rig.renderer.gles().take_calls();
        apply_attach(&mut rig, SOURCE, 3);
        assert_eq!(
            rig.renderer.live_counts(),
            Counts::default(),
            "no buffer was imported in the frame"
        );
        assert_eq!(
            rig.renderer.gles().take_calls(),
            crate::gl::Calls::default(),
            "and the frame asked the GL for nothing"
        );
        assert!(events(&mut rig).is_empty(), "nobody is told yet");
        // A picture of a source that is not there yet is ignored, and the layer shows nothing.
        flip(&mut rig, 0);
        assert_eq!(rig.renderer.live_counts().flips, 0);
        assert_eq!(rig.draw(&scene(1)).pixel(INSIDE.0, INSIDE.1), GROUND);
        // A call of the housekeeping deals with one buffer at the least, however little time it is given, and says there is more until
        // the last; the producer hears of the source with the last.
        for done in 1..=3usize {
            let tidied = rig.renderer.tidy(Duration::ZERO);
            assert_eq!(tidied.live, 1, "call {done}");
            assert_eq!(tidied.more, done < 3, "call {done}");
            assert_eq!(memory.textures.lock().len(), done);
            assert_eq!(rig.renderer.live_counts().imports, done as u64);
            assert_eq!(
                events(&mut rig).len(),
                usize::from(done == 3),
                "call {done}"
            );
        }
        assert_eq!(
            rig.renderer.tidy(Duration::ZERO),
            crate::renderer::Tidied::default(),
            "nothing is left"
        );
        flip(&mut rig, 0);
        assert_eq!(rig.renderer.live_counts().flips, 1);
        assert_eq!(rig.renderer.gles().take_error(), None);
    }

    #[test]
    fn each_buffer_is_drawn_once_off_screen_before_its_source_is_announced_and_a_picture_costs_no_more_the_first_time()
     {
        let (_memory, Some(mut rig)) = working() else {
            return;
        };
        apply_attach(&mut rig, SOURCE, 3);
        rig.renderer.gles().take_calls();
        // Each call draws the buffer it has just put on the GPU, where nothing is shown: its own colour is what the target holds.
        for (index, colour) in MADE_AS.into_iter().enumerate() {
            rig.renderer.tidy(Duration::ZERO);
            assert_eq!(
                rig.renderer.touched_pixel(),
                Some(colour),
                "buffer {index} was drawn"
            );
        }
        let housekeeping = rig.renderer.gles().take_calls();
        assert_eq!(
            housekeeping.draws, 3,
            "one draw of each buffer, and no more"
        );
        assert_eq!(rig.renderer.live_counts().first_draws, 3);
        assert_eq!(events(&mut rig).len(), 1);
        // The frame that shows the first picture asks the driver for what the frame of any picture does: the same draws, program
        // switches, texture binds and vertices (what the GL's state cache leaves out of a frame, a viewport or a uniform that is set
        // already, is not work: the first frame after anything that drew has a few more to set).
        let stable = |calls: crate::gl::Calls| {
            (calls.draws, calls.programs, calls.binds, calls.vertex_bytes)
        };
        flip(&mut rig, 0);
        rig.draw(&scene(2));
        let first = stable(rig.report.gl);
        assert_eq!(rig.report.uploaded_pixels, 0);
        for (buffer, version) in [(1, 3), (2, 4)] {
            flip(&mut rig, buffer);
            rig.draw(&scene(version));
            assert_eq!(stable(rig.report.gl), first, "buffer {buffer}");
            assert_eq!(rig.report.uploaded_pixels, 0);
        }
        assert_eq!(rig.renderer.gles().take_error(), None);
    }

    #[test]
    fn a_source_that_is_detached_or_replaced_before_it_is_all_on_the_gpu_is_never_announced_and_leaves_nothing_there()
     {
        let (memory, Some(mut rig)) = working() else {
            return;
        };
        apply_attach(&mut rig, SOURCE, 3);
        rig.renderer.tidy(Duration::ZERO);
        assert_eq!(memory.textures.lock().len(), 1);
        rig.renderer.apply(&Op::LiveDetach { source: SOURCE });
        assert_eq!(events(&mut rig), [Event::Detached { source: SOURCE }]);
        assert_eq!(
            memory.freed.load(Ordering::Relaxed),
            0,
            "the detach frees nothing in the frame"
        );
        rig.tidy_all();
        assert_eq!(memory.freed.load(Ordering::Relaxed), 1);
        assert_eq!(
            rig.renderer.live_counts().imports,
            1,
            "the other two were never put there"
        );
        assert!(
            events(&mut rig).is_empty(),
            "and nobody was told it was there"
        );
        // Another attach in its place half way: only the second is made, and announced once.
        apply_attach(&mut rig, SOURCE, 3);
        rig.renderer.tidy(Duration::ZERO);
        apply_attach(&mut rig, SOURCE, 2);
        assert!(events(&mut rig).is_empty());
        rig.tidy_all();
        let told = events(&mut rig);
        assert!(matches!(&told[..], [Event::Attached { .. }]), "{told:?}");
        assert_eq!(rig.renderer.live_counts().imports, 4);
        assert_eq!(memory.freed.load(Ordering::Relaxed), 2);
        assert_eq!(rig.renderer.gles().take_error(), None);
    }
}
