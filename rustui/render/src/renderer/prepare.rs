//! The renderer's side of prepared backdrops (`plates.rs` holds the sets): drawing a plate, in the moments the compositor has
//! to spare, and composing the picture the glass over a set reads, in the frame.
//!
//! **A step** is one plate: the layers of the plate's group (or the wallpaper layer) as the scene gives them for a plate, drawn
//! at `1/downsample` of the panel into the middle of a texture that has the blur's margin round it, and blurred by the chain
//! a glass surface's own backdrop is blurred by (`Backdrops::blurred`). It is drawn from the frame's own machinery (`place` and
//! `Drawing::draw`) as a picture of the layers at rest, glass flat, no backdrop made for any of it. The plates of a generation
//! are drawn a few to a call of [`Renderer::prepare`], and the generation is published when the last is.
//!
//! **A composition** is the plates of a generation drawn into the set's picture, each where its group is now: a handful of
//! textured quads, no blur and no capture. A frame does it for the glass it draws only when the generation or a world changed.

use super::{
    Casts, DrawReport, Drawing, ImageProgram, Pass, Pictures, Placer, Renderer, Tidied, View,
    picture_bytes,
};
use crate::feather;
use crate::geometry::{Panel, PixRect, Vertex};
use crate::gl::{GL_BLEND, GL_SCISSOR_TEST, Gles, RenderTarget};
use crate::log::log;
use crate::plates::{Composition, Plate, Sets, Step, plate_quad};
use crate::prepared::Source;
use crate::scene::Drawn;
use compositor_protocol::BackdropId;
use std::time::{Duration, Instant};

/// The most plates a call of [`Renderer::prepare`] draws. A plate costs the GPU about a millisecond at the Show's slowest
/// clock (its mirror, and the passes of its blur), and what a call queues goes in behind the frame just handed over: two keep
/// that to a couple of milliseconds, and a set of four plates is made in a few frames, which is in time for anything the app
/// prepares ahead.
const PLATES_PER_CALL: u32 = 2;

/// One plate to be drawn into a picture.
#[derive(Clone, Copy)]
pub(super) struct PlateDraw {
    texture: u32,
    size: (u32, u32),
    opacity: f32,
    /// Where the picture is drawn within, in its texels, when the group is clipped.
    clip: Option<PixRect>,
    /// How the clip's feathered edges fade the picture, when the group's clip is one.
    feather: Option<feather::Uniforms>,
}

/// What composing a picture draws with.
pub(super) struct Composer<'a> {
    pub gl: &'a Gles,
    pub panel: &'a Panel,
    /// The image program, and its twin that has the fade of a feathered clip.
    pub image: &'a ImageProgram,
    pub feathered: &'a ImageProgram,
    /// The vertex buffer the quads are in, and the frame's, which the draws that follow need back.
    pub buffer: u32,
    pub frame_buffer: u32,
}

impl Composer<'_> {
    /// The picture of the prepared backdrop `id` ready for glass to read, composed again if a generation was published or a
    /// group moved since it was; its texture. `None` when the set has nothing to show (no generation finished yet), and glass
    /// over it is the tint alone. `vertices` and `draws` are the quads' scratch, reused from call to call.
    pub fn compose(
        &self,
        sets: &mut Sets,
        id: BackdropId,
        vertices: &mut Vec<Vertex>,
        draws: &mut Vec<PlateDraw>,
    ) -> Option<u32> {
        let Some(composition) = sets.to_compose(id) else {
            return sets.picture(id);
        };
        let Composition {
            picture,
            geometry,
            wallpaper,
            groups,
            worlds,
        } = composition;
        vertices.clear();
        draws.clear();
        // The wallpaper's plate is where the panel is. The groups' are where their groups are now, over it, in the order the
        // spec names them: the first is the bottom.
        let mut add = |plate: RenderTarget,
                       quad: [Vertex; 4],
                       opacity: f32,
                       clip,
                       feather: Option<feather::Uniforms>| {
            vertices.extend_from_slice(&quad);
            draws.push(PlateDraw {
                texture: plate.texture,
                size: (plate.width, plate.height),
                opacity,
                clip,
                feather,
            });
        };
        if let Some(plate) = wallpaper {
            let quad = plate_quad(
                self.panel,
                0,
                geometry.texel,
                (plate.width, plate.height),
                None,
            );
            add(plate, quad, 1.0, None, None);
        }
        for (plate, world) in groups.iter().zip(worlds) {
            let (Some(plate), Some(world)) = (plate, world) else {
                continue;
            };
            if !world.visible || world.opacity <= 0.0 || world.transform.scale <= 0.0 {
                continue;
            }
            let clip = match world.clip {
                Some(rect) => match self.panel.scissor(rect) {
                    Some(pixels) => Some(pixels.scaled(1.0 / geometry.texel as f32)),
                    None => continue,
                },
                None => None,
            };
            // A clip that is feathered fades the picture towards its edges: the picture's own pixels are the panel's
            // `1 / texel` times over.
            let feather = world
                .clip
                .and_then(|rect| self.panel.ramp(rect, world.feather))
                .map(|ramp| feather::Uniforms::new(&ramp, 1.0 / geometry.texel as f32, [0, 0]));
            let quad = plate_quad(
                self.panel,
                geometry.margin,
                geometry.texel,
                (plate.width, plate.height),
                Some(world),
            );
            add(*plate, quad, world.opacity, clip, feather);
        }
        let gl = self.gl;
        gl.bind_framebuffer(picture.framebuffer);
        gl.viewport(0, 0, picture.width as i32, picture.height as i32);
        gl.disable(GL_SCISSOR_TEST);
        gl.clear(0.0, 0.0, 0.0, 0.0);
        if !draws.is_empty() {
            gl.enable(GL_BLEND);
            gl.active_texture(0);
            gl.use_vertex_buffer(self.buffer);
            gl.stream_vertices(self.buffer, vertices);
            let mut scissored = false;
            let mut using = 0;
            for (n, draw) in draws.iter().enumerate() {
                match draw.clip {
                    Some(clip) => {
                        if !std::mem::replace(&mut scissored, true) {
                            gl.enable(GL_SCISSOR_TEST);
                        }
                        gl.scissor(clip.x as i32, clip.y as i32, clip.w as i32, clip.h as i32);
                    }
                    None if std::mem::replace(&mut scissored, false) => gl.disable(GL_SCISSOR_TEST),
                    None => {}
                }
                let program = if draw.feather.is_some() {
                    self.feathered
                } else {
                    self.image
                };
                if program.id != using {
                    using = program.id;
                    gl.use_program(program.id);
                }
                if let Some(fade) = &draw.feather {
                    program.feather.set(gl, fade);
                }
                gl.bind_texture(draw.texture);
                gl.uniform2f(
                    program.texel,
                    1.0 / draw.size.0 as f32,
                    1.0 / draw.size.1 as f32,
                );
                gl.uniform1f(program.opacity, draw.opacity);
                gl.draw_triangle_strip(4 * n as i32, 4);
            }
            gl.disable(GL_SCISSOR_TEST);
            gl.use_vertex_buffer(self.frame_buffer);
        }
        sets.composed(id);
        Some(picture.texture)
    }
}

impl Renderer {
    /// Makes the next plates of the prepared backdrops that are due, for at most `budget` (at least one plate is drawn, and no
    /// more than [`PLATES_PER_CALL`]): the engine calls it in the moments it has to spare, after the frame just handed over
    /// and when nothing is due (`Renderer::tidy` is the same kind of housekeeping). A set is due when the app asked for a
    /// new generation (`BackdropRefresh`), no lease holds it, and nothing is moving: while a finger drives a track, or one
    /// settles (`Source::moving`), no plate is drawn, so that no capture is ever attributable to motion. The plates of a
    /// generation are drawn over several calls, and the generation is published (`Event::BackdropReady`) when the last
    /// is; `more` says that a set is still due.
    pub fn prepare(&mut self, source: &dyn Source, budget: Duration) -> Tidied {
        let began = Instant::now();
        let mut done = Tidied::default();
        if source.moving() {
            return done;
        }
        while let Some(id) = self.sets.due() {
            if done.captured >= PLATES_PER_CALL || (done.captured > 0 && began.elapsed() >= budget)
            {
                break;
            }
            if !self.step(id, source, &mut done) {
                break;
            }
        }
        if done.captured > 0 {
            // The GPU starts on them now, not with the next frame.
            self.gl.flush();
        }
        done.more = self.sets.due().is_some();
        done
    }

    /// One step of the generation of `id`: a plate drawn, and the generation published if it was the last. Whether there was a
    /// step to make.
    fn step(&mut self, id: BackdropId, source: &dyn Source, done: &mut Tidied) -> bool {
        let Some(step) = self.sets.step(id) else {
            return false;
        };
        if let Some(plate) = step.plate {
            let target = self.capture(&step, plate, source, done);
            done.captured += u32::from(target.is_some());
            self.sets.finish_plate(id, target);
        }
        if step.last {
            if let Some(published) = self.sets.publish(id, &self.gl, &mut self.backdrops) {
                self.events.push(published.event);
                // Glass that showed the set in the frame drawn last shows another picture now.
                self.stale |= published.glass_frame != 0 && published.glass_frame == self.frame;
            }
        }
        true
    }

    /// Draws and blurs one plate; `None` when it has nothing in it (or the GPU has no room for it).
    fn capture(
        &mut self,
        step: &Step,
        plate: Plate,
        source: &dyn Source,
        done: &mut Tidied,
    ) -> Option<RenderTarget> {
        let mut layers = std::mem::take(&mut self.plate_layers);
        let group = match plate {
            Plate::Wallpaper(layer) => {
                layers.clear();
                layers.extend(source.layer(layer));
                false
            }
            Plate::Group(group) => {
                source.plate_layers(group, &mut layers);
                true
            }
        };
        let made = self.draw_plate(step, group, &layers, done);
        self.plate_layers = layers;
        if made.is_some() {
            self.captures_since = self.captures_since.saturating_add(1);
        }
        made
    }

    fn draw_plate(
        &mut self,
        step: &Step,
        group: bool,
        layers: &[Drawn],
        done: &mut Tidied,
    ) -> Option<RenderTarget> {
        if layers.is_empty() {
            return None;
        }
        let geometry = &step.geometry;
        let panel_size = (self.panel.width, self.panel.height);
        // What the first level of the chain is a picture of, and where in it the panel is: a group's plate has its margin
        // round the panel, which nothing is drawn in, and the wallpaper's has none.
        let (size, margin) = if group {
            (geometry.group_size, geometry.margin)
        } else {
            (panel_size, 0)
        };
        let d = u32::from(geometry.downsample);
        let origin = (margin / d) as i32;
        let inner = (panel_size.0.div_ceil(d), panel_size.1.div_ceil(d));

        self.vertices.clear();
        let mut report = DrawReport::default();
        let mut placer = Placer {
            gl: &self.gl,
            panel: &self.panel,
            glass: &mut self.glass,
            textures: &mut self.textures,
            atlases: &self.atlases,
            sets: &self.sets,
            frame: self.frame,
            skips: self.profile.skips(),
        };
        Self::place(
            &mut placer,
            layers,
            View::Rest,
            Pictures::Now,
            &mut self.vertices,
            &mut self.jobs,
            &mut report,
        );
        done.uploaded_pixels += report.uploaded_pixels;
        self.uploaded_since = self
            .uploaded_since
            .saturating_add(picture_bytes(report.uploaded_pixels));
        if self.jobs.is_empty() {
            return None;
        }
        let gl = &self.gl;
        let vertex_buffer = self.vertex_buffer;
        gl.use_vertex_buffer(vertex_buffer);
        gl.stream_vertices(vertex_buffer, &self.vertices);
        let drawing = Drawing {
            gl,
            panel: &self.panel,
            programs: &self.programs,
            glass: &self.glass,
            textures: &self.textures,
            atlases: &self.atlases,
            wallpapers: &self.wallpapers,
            meshes: &self.meshes,
            live: &self.live,
            skips: self.profile.skips(),
            vertex_buffer,
            plates: &self.plates,
            origin: [origin, origin],
            cast_done: std::cell::RefCell::new(std::mem::take(&mut self.cast_done)),
            // A prepared plate is not a frame: its fill is not the frame's.
            fills: None,
        };
        let jobs = &self.jobs;
        let casts = Casts::none();
        let factor = 1.0 / f32::from(geometry.downsample);
        let plate = self.backdrops.blurred(
            gl,
            size,
            step.radius,
            geometry.downsample,
            |level0: &RenderTarget| {
                gl.use_vertex_buffer(vertex_buffer);
                gl.bind_framebuffer(level0.framebuffer);
                gl.disable(GL_SCISSOR_TEST);
                // Transparent: what the group does not draw is not part of the plate.
                gl.clear(0.0, 0.0, 0.0, 0.0);
                gl.viewport(origin, origin, inner.0 as i32, inner.1 as i32);
                gl.enable(GL_BLEND);
                drawing.draw(
                    layers,
                    jobs,
                    &Pass {
                        view: View::Rest,
                        factor,
                        upto: usize::MAX,
                        region: None,
                        pictures: &[],
                        casts: &casts,
                        cast_after: &[],
                        sights: None,
                        reuse: &[],
                    },
                );
                gl.disable(GL_SCISSOR_TEST);
            },
        );
        self.cast_done = drawing.cast_done.into_inner();
        if plate.is_none() {
            log(format!(
                "prepared backdrop {}: no room for a plate of {}x{}",
                step.id.0, geometry.group_plate.0, geometry.group_plate.1
            ));
        }
        plate
    }
}
