//! `zwp_linux_dmabuf_v1` v3: the buffer path of LumaTV's own 64-bit clients. Linear and ARM AFBC (the 16x16
//! YTR sparse tiled layout LG's libmali produces) in the four 32-bit RGB formats.

use std::sync::Arc;

use parking_lot::Mutex;
use wayland_server::protocol::wl_buffer::WlBuffer;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};

use wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_buffer_params_v1::{self, Error as ParamsError, Flags, ZwpLinuxBufferParamsV1};
use wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_v1::{self, ZwpLinuxDmabufV1};

use crate::State;
use crate::buffer::{BufferData, BufferKind, DmabufBuffer, DmabufPlane, fourcc, modifier};

pub const FORMATS: [u32; 4] = [fourcc::ARGB8888, fourcc::XRGB8888, fourcc::ABGR8888, fourcc::XBGR8888];
pub const MODIFIERS: [u64; 2] = [modifier::LINEAR, modifier::AFBC_16X16_YTR_SPARSE_TILED];

#[derive(Default)]
pub struct ParamsData {
    planes: Mutex<Vec<(u32, DmabufPlane, u64)>>,
    used: Mutex<bool>,
}

impl GlobalDispatch<ZwpLinuxDmabufV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpLinuxDmabufV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let d = data_init.init(resource, ());
        for f in FORMATS {
            if d.version() >= 3 {
                for m in MODIFIERS {
                    d.modifier(f, (m >> 32) as u32, m as u32);
                }
            } else {
                d.format(f);
            }
        }
    }
}

impl Dispatch<ZwpLinuxDmabufV1, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &ZwpLinuxDmabufV1,
        request: zwp_linux_dmabuf_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            zwp_linux_dmabuf_v1::Request::CreateParams { params_id } => {
                data_init.init(params_id, ParamsData::default());
            }
            zwp_linux_dmabuf_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwpLinuxBufferParamsV1, ParamsData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwpLinuxBufferParamsV1,
        request: zwp_linux_buffer_params_v1::Request,
        data: &ParamsData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_linux_buffer_params_v1::Request::Add { fd, plane_idx, offset, stride, modifier_hi, modifier_lo } => {
                if *data.used.lock() {
                    resource.post_error(ParamsError::AlreadyUsed, "params already used");
                    return;
                }
                let mut planes = data.planes.lock();
                if planes.iter().any(|(i, _, _)| *i == plane_idx) {
                    resource.post_error(ParamsError::PlaneSet, "plane already set");
                    return;
                }
                planes.push((plane_idx, DmabufPlane { fd, offset, stride }, ((modifier_hi as u64) << 32) | modifier_lo as u64));
            }
            zwp_linux_buffer_params_v1::Request::Create { width, height, format, flags } => {
                let created = finish(state, resource, data, width, height, format, flags.into_result().map(|f| f.bits()).unwrap_or(0));
                match created {
                    Some(kind) => {
                        // The buffer gets a server-allocated id; wayland-server makes one through the parent.
                        let buf: WlBuffer = match _client_create(state, resource, kind) {
                            Some(b) => b,
                            None => {
                                resource.failed();
                                return;
                            }
                        };
                        resource.created(&buf);
                    }
                    None => resource.failed(),
                }
            }
            zwp_linux_buffer_params_v1::Request::CreateImmed { buffer_id, width, height, format, flags } => {
                match finish(state, resource, data, width, height, format, flags.into_result().map(|f| f.bits()).unwrap_or(0)) {
                    Some(kind) => {
                        let b = data_init.init(buffer_id, BufferData { kind });
                        state.stats.buffers_created += 1;
                        state.note_request(
                            "zwp_linux_buffer_params_v1",
                            0,
                            &format!("create_immed -> wl_buffer@{} {width}x{height} {}", b.id().protocol_id(), fourcc::name(format)),
                        );
                    }
                    None => resource.post_error(ParamsError::InvalidFormat, "unsupported buffer"),
                }
            }
            zwp_linux_buffer_params_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

fn finish(state: &mut State, resource: &ZwpLinuxBufferParamsV1, data: &ParamsData, width: i32, height: i32, format: u32, flags: u32) -> Option<BufferKind> {
    let mut used = data.used.lock();
    if *used {
        resource.post_error(ParamsError::AlreadyUsed, "params already used");
        return None;
    }
    *used = true;
    let mut planes = std::mem::take(&mut *data.planes.lock());
    if planes.is_empty() {
        resource.post_error(ParamsError::Incomplete, "no planes");
        return None;
    }
    planes.sort_by_key(|(i, _, _)| *i);
    if width <= 0 || height <= 0 {
        resource.post_error(ParamsError::InvalidDimensions, "size");
        return None;
    }
    if !FORMATS.contains(&format) {
        resource.post_error(ParamsError::InvalidFormat, format!("format {} not offered", fourcc::name(format)));
        return None;
    }
    let modifier = planes[0].2;
    let y_invert = flags & Flags::YInvert.bits() != 0;
    let planes: Vec<DmabufPlane> = planes.into_iter().map(|(_, p, _)| p).collect();
    state.note_request(
        "zwp_linux_buffer_params_v1",
        0,
        &format!("{width}x{height} {} modifier 0x{modifier:x} {} plane(s)", fourcc::name(format), planes.len()),
    );
    Some(BufferKind::Dmabuf(Arc::new(DmabufBuffer { width, height, format, modifier, planes, y_invert })))
}

/// `create` (non-immediate) needs a server-created `wl_buffer`: `Client::create_resource` makes it.
fn _client_create(state: &mut State, params: &ZwpLinuxBufferParamsV1, kind: BufferKind) -> Option<WlBuffer> {
    let client = params.client()?;
    let b = client.create_resource::<WlBuffer, BufferData, State>(&state.dh, 1, BufferData { kind }).ok()?;
    state.stats.buffers_created += 1;
    Some(b)
}
