//! `wl_webos_surface_group_compositor`, `wl_webos_surface_group`, `wl_webos_surface_group_layer`: named z-stacks
//! owned by one surface, into which other clients attach surfaces (LG uses them for overlays over an app).

use std::collections::BTreeMap;

use wayland_backend::server::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use lumatv_wl_protocols::webos_surface_group::server::wl_webos_surface_group::{self, WlWebosSurfaceGroup};
use lumatv_wl_protocols::webos_surface_group::server::wl_webos_surface_group_compositor::{self, WlWebosSurfaceGroupCompositor};
use lumatv_wl_protocols::webos_surface_group::server::wl_webos_surface_group_layer::{self, WlWebosSurfaceGroupLayer};

use super::compositor::surface_id;
use crate::scene::{Group, GroupLayer};
use crate::surface::Role;
use crate::{State, plain_global};

pub struct GroupData {
    pub name: String,
    /// The owner's handle sees `owner_destroyed` semantics differently from a client that only looked it up.
    pub is_owner: bool,
}

pub struct LayerData {
    pub group: String,
    pub name: String,
}

plain_global!(WlWebosSurfaceGroupCompositor);

impl Dispatch<WlWebosSurfaceGroupCompositor, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WlWebosSurfaceGroupCompositor,
        request: wl_webos_surface_group_compositor::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_webos_surface_group_compositor::Request::CreateSurfaceGroup { id, parent, name } => {
                let Some(owner) = surface_id(&parent) else {
                    resource.post_error(0u32, "unknown parent surface");
                    return;
                };
                if state.scene.groups.iter().any(|g| g.name == name) {
                    log::warn!("surface group {name:?} already exists; replacing");
                    state.scene.groups.retain(|g| g.name != name);
                }
                state.scene.groups.push(Group { name: name.clone(), owner, layers: BTreeMap::new(), allow_anonymous: false, anonymous: Vec::new() });
                data_init.init(id, GroupData { name: name.clone(), is_owner: true });
                state.journal(format!("surface group {name:?} created by surface {}", owner.0));
            }
            wl_webos_surface_group_compositor::Request::GetSurfaceGroup { id, name } => {
                let exists = state.scene.groups.iter().any(|g| g.name == name);
                let g = data_init.init(id, GroupData { name: name.clone(), is_owner: false });
                state.note_request(
                    "wl_webos_surface_group_compositor",
                    0,
                    &format!("get_surface_group({name:?}) {}", if exists { "found" } else { "unknown" }),
                );
                if !exists {
                    // LG's compositor answers an unknown group with owner_destroyed, which clients treat as "gone".
                    g.owner_destroyed();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlWebosSurfaceGroup, GroupData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlWebosSurfaceGroup,
        request: wl_webos_surface_group::Request,
        data: &GroupData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let pid = resource.id().protocol_id();
        let gname = data.name.clone();
        let gi = state.scene.groups.iter().position(|g| g.name == gname);
        match request {
            wl_webos_surface_group::Request::CreateLayer { id, name, z_index } => {
                state.note_request("wl_webos_surface_group", pid, &format!("create_layer({name:?}, z {z_index})"));
                if let Some(gi) = gi {
                    state.scene.groups[gi].layers.insert(name.clone(), GroupLayer { z: z_index, key_index: 0, member: None });
                }
                data_init.init(id, LayerData { group: gname, name });
            }
            wl_webos_surface_group::Request::Attach { surface, layer_name } => {
                let sid = surface_id(&surface);
                state.note_request("wl_webos_surface_group", pid, &format!("attach(surface {:?}, layer {layer_name:?})", sid.map(|s| s.0)));
                if let (Some(sid), Some(gi)) = (sid, gi) {
                    let g = &mut state.scene.groups[gi];
                    let owner = g.owner;
                    let attached = match g.layers.get_mut(&layer_name) {
                        Some(l) => {
                            l.member = Some(sid);
                            true
                        }
                        None => false,
                    };
                    if attached {
                        if let Some(s) = state.surfaces.get_mut(sid)
                            && s.role == Role::None
                        {
                            s.role = Role::GroupMember { owner };
                        }
                        state.scene.layout_changed = true;
                    } else {
                        log::warn!("attach to unknown layer {layer_name:?} of group {gname:?}");
                    }
                }
            }
            wl_webos_surface_group::Request::AttachAnonymous { surface, z_hint } => {
                let sid = surface_id(&surface);
                let hint = z_hint;
                state.note_request("wl_webos_surface_group", pid, &format!("attach_anonymous(surface {:?}, z_hint {hint})", sid.map(|s| s.0)));
                if let (Some(sid), Some(gi)) = (sid, gi) {
                    let g = &mut state.scene.groups[gi];
                    let owner = g.owner;
                    if g.allow_anonymous {
                        g.anonymous.push((sid, hint));
                        if let Some(s) = state.surfaces.get_mut(sid)
                            && s.role == Role::None
                        {
                            s.role = Role::GroupMember { owner };
                        }
                        state.scene.layout_changed = true;
                    } else {
                        log::warn!("anonymous attach to group {gname:?} that does not allow it");
                    }
                }
            }
            wl_webos_surface_group::Request::AllowAnonymousLayers { allow } => {
                if let Some(gi) = gi {
                    state.scene.groups[gi].allow_anonymous = allow != 0;
                }
            }
            wl_webos_surface_group::Request::Detach { surface } => {
                let sid = surface_id(&surface);
                state.note_request("wl_webos_surface_group", pid, &format!("detach(surface {:?})", sid.map(|s| s.0)));
                if let (Some(sid), Some(gi)) = (sid, gi) {
                    let g = &mut state.scene.groups[gi];
                    for l in g.layers.values_mut() {
                        if l.member == Some(sid) {
                            l.member = None;
                        }
                    }
                    g.anonymous.retain(|(s, _)| *s != sid);
                    if let Some(s) = state.surfaces.get_mut(sid)
                        && matches!(s.role, Role::GroupMember { .. })
                    {
                        s.role = Role::None;
                    }
                    state.scene.layout_changed = true;
                }
            }
            wl_webos_surface_group::Request::FocusOwner => state.note_request("wl_webos_surface_group", pid, "focus_owner"),
            wl_webos_surface_group::Request::FocusLayer { layer } => state.note_request("wl_webos_surface_group", pid, &format!("focus_layer({layer:?})")),
            wl_webos_surface_group::Request::CommitKeyIndex { commit } => {
                state.note_request("wl_webos_surface_group", pid, &format!("commit_key_index({commit})"))
            }
            wl_webos_surface_group::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &WlWebosSurfaceGroup, data: &GroupData) {
        if data.is_owner {
            state.scene.groups.retain(|g| g.name != data.name);
            state.scene.layout_changed = true;
        }
    }
}

impl Dispatch<WlWebosSurfaceGroupLayer, LayerData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlWebosSurfaceGroupLayer,
        request: wl_webos_surface_group_layer::Request,
        data: &LayerData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        let pid = resource.id().protocol_id();
        fn layer<'a>(state: &'a mut State, data: &LayerData) -> Option<&'a mut GroupLayer> {
            state.scene.groups.iter_mut().find(|g| g.name == data.group).and_then(|g| g.layers.get_mut(&data.name))
        }
        match request {
            wl_webos_surface_group_layer::Request::SetZIndex { z_index } => {
                if let Some(l) = layer(state, data) {
                    l.z = z_index;
                }
                state.scene.layout_changed = true;
                state.note_request("wl_webos_surface_group_layer", pid, &format!("set_z_index({z_index})"));
            }
            wl_webos_surface_group_layer::Request::SetKeyIndex { key_index } => {
                if let Some(l) = layer(state, data) {
                    l.key_index = key_index;
                }
            }
            wl_webos_surface_group_layer::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &WlWebosSurfaceGroupLayer, data: &LayerData) {
        if let Some(g) = state.scene.groups.iter_mut().find(|g| g.name == data.group) {
            g.layers.remove(&data.name);
            state.scene.layout_changed = true;
        }
    }
}
