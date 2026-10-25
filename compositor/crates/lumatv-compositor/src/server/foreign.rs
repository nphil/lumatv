//! `wl_webos_foreign`: exported elements (video regions) and imported punch-throughs, plus the video placement
//! the compositor sends to videooutputd for them (`docs/research/compositor-c2-result.md` §7).

use std::collections::HashMap;

use wayland_backend::server::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource, WEnum};

use lumatv_wl_protocols::webos_foreign::server::wl_webos_exported::{self, WlWebosExported};
use lumatv_wl_protocols::webos_foreign::server::wl_webos_foreign::{self, WlWebosForeign};
use lumatv_wl_protocols::webos_foreign::server::wl_webos_imported::{self, WlWebosImported};

use super::compositor::{region_rects, surface_id};
use crate::luna::{self, Outbound, ToBridge};
use crate::scene::Hole;
use crate::surface::SurfaceId;
use crate::{Rect, State, plain_global};

pub struct Exported {
    pub window_id: String,
    pub surface: SurfaceId,
    pub exported_type: u32,
    pub source: Option<Rect>,
    pub original: Option<Rect>,
    pub destination: Rect,
    pub props: Vec<(String, String)>,
    /// Punch-through contexts attached to this element.
    pub contexts: Vec<String>,
    pub resource: WlWebosExported,
}

pub struct Imported {
    pub window_id: String,
    pub context: Option<String>,
    pub attached_surface: Option<SurfaceId>,
    pub z_index: i32,
    pub alignment: u32,
}

#[derive(Default)]
pub struct ForeignState {
    next_id: u32,
    pub exported: HashMap<String, Exported>,
    pub imported: HashMap<wayland_backend::server::ObjectId, Imported>,
    next_token: u64,
}

impl ForeignState {
    fn new_window_id(&mut self) -> String {
        self.next_id += 1;
        format!("_Window_Id_{}", self.next_id)
    }

    pub fn surface_destroyed(&mut self, id: SurfaceId) {
        self.exported.retain(|_, e| e.surface != id);
        for i in self.imported.values_mut() {
            if i.attached_surface == Some(id) {
                i.attached_surface = None;
            }
        }
    }

    /// What `getForegroundVideoWindow` reports: every exported element with a context.
    pub fn video_windows(&self) -> Vec<serde_json::Value> {
        self.exported
            .values()
            .filter(|e| !e.contexts.is_empty())
            .map(|e| {
                serde_json::json!({
                    "windowId": e.window_id,
                    "context": e.contexts.last(),
                    "displayOutput": {"x": e.destination.x, "y": e.destination.y, "width": e.destination.w, "height": e.destination.h},
                    "fullScreen": e.destination.w >= crate::OUTPUT_WIDTH as i32 && e.destination.h >= crate::OUTPUT_HEIGHT as i32,
                })
            })
            .collect()
    }
}

pub struct ExportedData {
    pub window_id: String,
}

plain_global!(WlWebosForeign);

impl Dispatch<WlWebosForeign, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WlWebosForeign,
        request: wl_webos_foreign::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_webos_foreign::Request::ExportElement { id, surface, exported_type } => {
                let Some(sid) = surface_id(&surface) else {
                    resource.post_error(0u32, "unknown surface");
                    return;
                };
                let window_id = state.foreign.new_window_id();
                let res = data_init.init(id, ExportedData { window_id: window_id.clone() });
                res.window_id_assigned(window_id.clone(), exported_type);
                state.foreign.exported.insert(
                    window_id.clone(),
                    Exported {
                        window_id: window_id.clone(),
                        surface: sid,
                        exported_type,
                        source: None,
                        original: None,
                        destination: Rect::default(),
                        props: Vec::new(),
                        contexts: Vec::new(),
                        resource: res,
                    },
                );
                state.journal(format!("foreign: surface {} exported as {window_id} (type {exported_type})", sid.0));
            }
            wl_webos_foreign::Request::ImportElement { id, window_id, exported_type } => {
                let res = data_init.init(id, ());
                let t = exported_type;
                state
                    .foreign
                    .imported
                    .insert(res.id(), Imported { window_id: window_id.clone(), context: None, attached_surface: None, z_index: 0, alignment: 0 });
                state.journal(format!(
                    "foreign: {window_id} imported (type {t}){}",
                    if state.foreign.exported.contains_key(&window_id) { "" } else { " (no such export)" }
                ));
            }
            wl_webos_foreign::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<WlWebosExported, ExportedData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlWebosExported,
        request: wl_webos_exported::Request,
        data: &ExportedData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        let pid = resource.id().protocol_id();
        match request {
            wl_webos_exported::Request::SetExportedWindow { source_region, destination_region } => {
                let src = region_rects(&source_region).first().copied();
                let dst = region_rects(&destination_region).first().copied().unwrap_or_default();
                state.note_request("wl_webos_exported", pid, &format!("set_exported_window(src {src:?}, dst {dst:?})"));
                if let Some(e) = state.foreign.exported.get_mut(&data.window_id) {
                    e.source = src;
                    e.destination = dst;
                }
                state.exported_changed(&data.window_id);
            }
            wl_webos_exported::Request::SetCropRegion { original_input, source_region, destination_region } => {
                let orig = region_rects(&original_input).first().copied();
                let src = region_rects(&source_region).first().copied();
                let dst = region_rects(&destination_region).first().copied().unwrap_or_default();
                state.note_request("wl_webos_exported", pid, &format!("set_crop_region(orig {orig:?}, src {src:?}, dst {dst:?})"));
                if let Some(e) = state.foreign.exported.get_mut(&data.window_id) {
                    e.original = orig;
                    e.source = src;
                    e.destination = dst;
                }
                state.exported_changed(&data.window_id);
            }
            wl_webos_exported::Request::SetProperty { name, value } => {
                state.note_request("wl_webos_exported", pid, &format!("set_property({name:?}, {value:?})"));
                let contexts = state.foreign.exported.get(&data.window_id).map(|e| e.contexts.clone()).unwrap_or_default();
                if let Some(e) = state.foreign.exported.get_mut(&data.window_id) {
                    e.props.retain(|(n, _)| *n != name);
                    e.props.push((name.clone(), value.clone()));
                }
                for ctx in contexts {
                    let (uri, payload) = luna::video::set_property(&ctx, &name, &value);
                    state.luna_call(uri, payload);
                }
            }
            wl_webos_exported::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &WlWebosExported, data: &ExportedData) {
        if let Some(e) = state.foreign.exported.remove(&data.window_id) {
            for ctx in &e.contexts {
                let (uri, payload) = luna::video::set_display_window(ctx, None, &Rect::default(), None);
                state.luna_call(uri, payload);
            }
            state.scene.holes.retain(|h| h.surface != e.surface);
            state.scene.layout_changed = true;
        }
    }
}

impl Dispatch<WlWebosImported, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlWebosImported,
        request: wl_webos_imported::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        let id = resource.id();
        let pid = id.protocol_id();
        let window_id = state.foreign.imported.get(&id).map(|i| i.window_id.clone()).unwrap_or_default();
        match request {
            wl_webos_imported::Request::AttachPunchthrough { context_id: context } => {
                state.note_request("wl_webos_imported", pid, &format!("attach_punchthrough({context:?})"));
                state.attach_punchthrough(&id, &window_id, context.clone());
                resource.punchthrough_attached(context);
            }
            wl_webos_imported::Request::SetPunchthrough { context_id: context } => {
                state.note_request("wl_webos_imported", pid, &format!("set_punchthrough({context:?})"));
                state.attach_punchthrough(&id, &window_id, context);
            }
            wl_webos_imported::Request::DetachPunchthrough => {
                state.note_request("wl_webos_imported", pid, "detach_punchthrough");
                let ctx = state.detach_punchthrough(&id, &window_id);
                resource.punchthrough_detached(ctx.unwrap_or_default());
            }
            wl_webos_imported::Request::AttachSurface { surface } => {
                let sid = surface_id(&surface);
                state.note_request("wl_webos_imported", pid, &format!("attach_surface(surface {:?})", sid.map(|s| s.0)));
                if let (Some(sid), Some(i)) = (sid, state.foreign.imported.get_mut(&id)) {
                    i.attached_surface = Some(sid);
                }
                resource.surface_attached(&surface);
            }
            wl_webos_imported::Request::DetachSurface { surface } => {
                if let Some(i) = state.foreign.imported.get_mut(&id) {
                    i.attached_surface = None;
                }
                resource.surface_detached(&surface);
            }
            wl_webos_imported::Request::SetSurfaceAlignment { format } => {
                let v = match format {
                    WEnum::Value(f) => f.bits(),
                    WEnum::Unknown(u) => u,
                };
                if let Some(i) = state.foreign.imported.get_mut(&id) {
                    i.alignment = v;
                }
            }
            wl_webos_imported::Request::SetZIndex { z_index } => {
                if let Some(i) = state.foreign.imported.get_mut(&id) {
                    i.z_index = z_index;
                }
                if let Some(ctx) = state.foreign.imported.get(&id).and_then(|i| i.context.clone()) {
                    let (uri, payload) = luna::video::set_compositing(&ctx, z_index);
                    state.luna_call(uri, payload);
                }
            }
            wl_webos_imported::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &WlWebosImported, _data: &()) {
        let id = resource.id();
        let window_id = state.foreign.imported.get(&id).map(|i| i.window_id.clone()).unwrap_or_default();
        state.detach_punchthrough(&id, &window_id);
        state.foreign.imported.remove(&id);
    }
}

impl State {
    pub fn luna_call(&mut self, uri: String, payload: serde_json::Value) {
        self.foreign.next_token += 1;
        // Placements are what the gate's Netflix check counts; the per-video property calls are left out.
        if uri.ends_with("/setDisplayWindow") || uri.ends_with("/setCropRegion") || uri.ends_with("/setCompositing") {
            let method = uri.rsplit('/').next().unwrap_or_default();
            self.journal(format!("luna: {method} {payload}"));
        }
        self.luna.send(ToBridge::Outbound(Outbound { token: self.foreign.next_token, uri, payload, subscribe: false }));
    }

    /// Output-space rectangle of an exported element: the exporting surface's window origin plus its destination.
    fn exported_output_rect(&self, e: &Exported) -> Rect {
        let origin =
            self.scene.window_of(e.surface).and_then(|w| self.scene.window(w)).map(|w| if w.kind.is_card() { (0, 0) } else { w.position }).unwrap_or((0, 0));
        e.destination.translated(origin.0, origin.1)
    }

    fn exported_changed(&mut self, window_id: &str) {
        let Some(e) = self.foreign.exported.get(window_id) else { return };
        let (surface, dest, source, original, contexts) = (e.surface, e.destination, e.source, e.original, e.contexts.clone());
        let out_rect = self.exported_output_rect(e);
        self.scene.holes.retain(|h| h.surface != surface);
        if !contexts.is_empty() && !dest.is_empty() {
            self.scene.holes.push(Hole { surface, rect: dest });
        }
        self.scene.layout_changed = true;
        for (id, i) in &self.foreign.imported {
            if i.window_id == window_id
                && let Ok(res) = WlWebosImported::from_id(&self.dh, id.clone())
            {
                res.destination_region_changed(dest.w.max(0) as u32, dest.h.max(0) as u32);
            }
        }
        for ctx in contexts {
            let (uri, payload) = match (original, source) {
                (Some(o), Some(s)) => luna::video::set_crop_region(&ctx, &o, &s, &out_rect, None),
                _ => luna::video::set_display_window(&ctx, source.as_ref(), &out_rect, None),
            };
            self.luna_call(uri, payload);
        }
    }

    fn attach_punchthrough(&mut self, imported: &wayland_backend::server::ObjectId, window_id: &str, ctx: String) {
        if let Some(i) = self.foreign.imported.get_mut(imported) {
            i.context = Some(ctx.clone());
        }
        let Some(e) = self.foreign.exported.get_mut(window_id) else {
            log::warn!("punch-through for unknown export {window_id}");
            return;
        };
        if !e.contexts.contains(&ctx) {
            e.contexts.push(ctx.clone());
        }
        self.journal(format!("foreign: punch-through {ctx} attached to {window_id}"));
        self.exported_changed(window_id);
        for (name, value) in [("appRotation", "Deg0"), ("aspectRatio", "original"), ("fastMode", "on"), ("textureRotation", "Deg0")] {
            let (uri, payload) = luna::video::set_property(&ctx, name, value);
            self.luna_call(uri, payload);
        }
    }

    fn detach_punchthrough(&mut self, imported: &wayland_backend::server::ObjectId, window_id: &str) -> Option<String> {
        let ctx = self.foreign.imported.get_mut(imported).and_then(|i| i.context.take());
        if let Some(e) = self.foreign.exported.get_mut(window_id) {
            match &ctx {
                Some(c) => e.contexts.retain(|x| x != c),
                None => e.contexts.clear(),
            }
        }
        if let Some(c) = &ctx {
            let (uri, payload) = luna::video::set_display_window(c, None, &Rect::default(), None);
            self.luna_call(uri, payload);
        }
        self.exported_changed(window_id);
        ctx
    }
}
