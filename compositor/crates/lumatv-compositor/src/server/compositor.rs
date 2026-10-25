//! `wl_compositor`, `wl_surface`, `wl_region`, `wl_subcompositor`, `wl_subsurface`, `wl_callback`.

use parking_lot::Mutex;
use wayland_backend::server::ClientId;
use wayland_server::protocol::wl_callback::WlCallback;
use wayland_server::protocol::wl_compositor::{self, WlCompositor};
use wayland_server::protocol::wl_region::{self, WlRegion};
use wayland_server::protocol::wl_subcompositor::{self, WlSubcompositor};
use wayland_server::protocol::wl_subsurface::{self, WlSubsurface};
use wayland_server::protocol::wl_surface::{self, WlSurface};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use crate::surface::SurfaceId;
use crate::{Rect, State, plain_global};

pub struct SurfaceData {
    pub id: SurfaceId,
}

/// Regions are add-only here: LG's clients set opaque and input regions as whole rectangles; a subtract clears
/// what it intersects, which is enough for a hit test and an opacity hint.
#[derive(Default)]
pub struct RegionData {
    pub rects: Mutex<Vec<Rect>>,
}

pub struct SubsurfaceData {
    pub surface: SurfaceId,
}

/// The `SurfaceId` of a `wl_surface` resource.
pub fn surface_id(s: &WlSurface) -> Option<SurfaceId> {
    s.data::<SurfaceData>().map(|d| d.id)
}

pub fn region_rects(r: &WlRegion) -> Vec<Rect> {
    r.data::<RegionData>().map(|d| d.rects.lock().clone()).unwrap_or_default()
}

plain_global!(WlCompositor);
plain_global!(WlSubcompositor);

impl Dispatch<WlCompositor, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &WlCompositor,
        request: wl_compositor::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                let sid = state.surfaces.create();
                let s = data_init.init(id, SurfaceData { id: sid });
                state.seat.surfaces.insert(sid, s.clone());
                state.note_request("wl_compositor", 0, &format!("create_surface -> wl_surface@{} (surface {})", s.id().protocol_id(), sid.0));
            }
            wl_compositor::Request::CreateRegion { id } => {
                data_init.init(id, RegionData::default());
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegion, RegionData> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlRegion,
        request: wl_region::Request,
        data: &RegionData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_region::Request::Add { x, y, width, height } => data.rects.lock().push(Rect::new(x, y, width, height)),
            wl_region::Request::Subtract { x, y, width, height } => {
                let r = Rect::new(x, y, width, height);
                data.rects.lock().retain(|e| e.intersect(&r).is_empty());
            }
            wl_region::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlCallback,
        _request: wayland_server::protocol::wl_callback::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<WlSurface, SurfaceData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlSurface,
        request: wl_surface::Request,
        data: &SurfaceData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let id = data.id;
        let pid = resource.id().protocol_id();
        match request {
            wl_surface::Request::Attach { buffer, x, y } => {
                state.note_request(
                    "wl_surface",
                    pid,
                    &format!(
                        "attach({}, {x}, {y})",
                        buffer
                            .as_ref()
                            .map(|b| format!("wl_buffer@{} {}", b.id().protocol_id(), crate::buffer::kind_of(b).name()))
                            .unwrap_or_else(|| "null".into())
                    ),
                );
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.buffer = Some(buffer);
                    s.pending.attach_offset = (x, y);
                }
            }
            wl_surface::Request::Damage { x, y, width, height } => {
                state.note_request("wl_surface", pid, &format!("damage({x},{y} {width}x{height})"));
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.damage.push(Rect::new(x, y, width, height));
                }
            }
            wl_surface::Request::DamageBuffer { x, y, width, height } => {
                state.note_request("wl_surface", pid, &format!("damage_buffer({x},{y} {width}x{height})"));
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.buffer_damage.push(Rect::new(x, y, width, height));
                }
            }
            wl_surface::Request::Frame { callback } => {
                let cb = data_init.init(callback, ());
                state.note_request("wl_surface", pid, "frame");
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.frame_callbacks.push(cb);
                }
            }
            wl_surface::Request::SetOpaqueRegion { region } => {
                let rects = region.as_ref().map(region_rects);
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.opaque = Some(rects);
                }
            }
            wl_surface::Request::SetInputRegion { region } => {
                let rects = region.as_ref().map(region_rects);
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.input = Some(rects);
                }
            }
            wl_surface::Request::Commit => {
                state.note_request("wl_surface", pid, "commit");
                state.commit(id);
            }
            wl_surface::Request::SetBufferTransform { transform } => {
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.transform = Some(transform.into_result().map(|t| t as u32).unwrap_or(0));
                }
            }
            wl_surface::Request::SetBufferScale { scale } => {
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.scale = Some(scale);
                }
            }
            wl_surface::Request::Offset { x, y } => {
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.attach_offset = (x, y);
                }
            }
            wl_surface::Request::Destroy => {
                state.note_request("wl_surface", pid, "destroy");
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &WlSurface, data: &SurfaceData) {
        state.seat.surfaces.remove(&data.id);
        if state.seat.focus == Some(data.id) {
            state.seat.focus = None;
        }
        state.surface_destroyed(data.id);
    }
}

impl Dispatch<WlSubcompositor, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlSubcompositor,
        request: wl_subcompositor::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_subcompositor::Request::GetSubsurface { id, surface, parent } => {
                let (Some(child), Some(par)) = (surface_id(&surface), surface_id(&parent)) else {
                    resource.post_error(wl_subcompositor::Error::BadSurface, "unknown surface");
                    return;
                };
                if !state.surfaces.add_subsurface(child, par) {
                    resource.post_error(wl_subcompositor::Error::BadSurface, "surface already has a role or is its own parent");
                    return;
                }
                data_init.init(id, SubsurfaceData { surface: child });
                state.note_request("wl_subcompositor", 0, &format!("get_subsurface(surface {} under {})", child.0, par.0));
                state.scene.layout_changed = true;
            }
            wl_subcompositor::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<WlSubsurface, SubsurfaceData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &WlSubsurface,
        request: wl_subsurface::Request,
        data: &SubsurfaceData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        let id = data.surface;
        match request {
            wl_subsurface::Request::SetPosition { x, y } => {
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.pending.sub_position = Some((x, y));
                }
            }
            wl_subsurface::Request::PlaceAbove { sibling } => {
                if let Some(sib) = surface_id(&sibling) {
                    state.surfaces.place(id, sib, true);
                    state.scene.layout_changed = true;
                }
            }
            wl_subsurface::Request::PlaceBelow { sibling } => {
                if let Some(sib) = surface_id(&sibling) {
                    state.surfaces.place(id, sib, false);
                    state.scene.layout_changed = true;
                }
            }
            wl_subsurface::Request::SetSync => {
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.sync = true;
                }
            }
            wl_subsurface::Request::SetDesync => {
                if let Some(s) = state.surfaces.get_mut(id) {
                    s.sync = false;
                }
                // Leaving sync mode applies what was cached.
                state.commit(id);
            }
            wl_subsurface::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &WlSubsurface, data: &SubsurfaceData) {
        if let Some(s) = state.surfaces.get_mut(data.surface) {
            if let crate::surface::Role::Subsurface { parent } = s.role {
                s.role = crate::surface::Role::None;
                if let Some(p) = state.surfaces.get_mut(parent) {
                    p.children.retain(|c| *c != data.surface);
                }
            }
            state.scene.layout_changed = true;
        }
    }
}
