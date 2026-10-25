//! The compositor's own `mali_buffer_sharing` v5 (`--buffers own`): the three messages LG's libmali speaks,
//! answered without the driver. The dma-buf fd stays with the buffer, which is what C3's direct scan-out needs;
//! the GL renderer imports it with the modifier the client sent.

use std::sync::Arc;

use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};

use lumatv_wl_protocols::mali_buffer_sharing::server::mali_buffer_sharing::{self, MaliBufferSharing};

use crate::buffer::{BufferData, BufferKind, DmabufBuffer, DmabufPlane, fourcc};
use crate::State;

/// What the client allocates its window buffers from (X2 r1: LG's libmali used this when told so; the library
/// also knows `/dev/dma_buf_unified` and `/dev/hma`).
pub const ALLOC_DEVICE: &str = "/dev/dma_buf_te";

impl GlobalDispatch<MaliBufferSharing, ()> for State {
    fn bind(state: &mut Self, _handle: &DisplayHandle, _client: &Client, resource: New<MaliBufferSharing>, _global_data: &(), data_init: &mut DataInit<'_, Self>) {
        let m = data_init.init(resource, ());
        m.alloc_device(ALLOC_DEVICE.into());
        state.journal(format!("client bound mali_buffer_sharing v{} (own); alloc_device {ALLOC_DEVICE}", m.version()));
    }
}

impl Dispatch<MaliBufferSharing, ()> for State {
    fn request(state: &mut Self, client: &Client, _resource: &MaliBufferSharing, request: mali_buffer_sharing::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        state.touch_client(client);
        match request {
            mali_buffer_sharing::Request::CreateBuffer { id, width, height, stride, format, modifier_hi, modifier_lo, fd } => {
                let modifier = ((modifier_hi as u64) << 32) | modifier_lo as u64;
                let b = data_init.init(
                    id,
                    BufferData { kind: BufferKind::Dmabuf(Arc::new(DmabufBuffer { width, height, format, modifier, planes: vec![DmabufPlane { fd, offset: 0, stride }], y_invert: true })) },
                );
                state.stats.buffers_created += 1;
                state.journal(format!("mali_buffer_sharing.create_buffer -> wl_buffer@{} {width}x{height} stride {stride} {} modifier 0x{modifier:016x}", b.id().protocol_id(), fourcc::name(format)));
            }
            mali_buffer_sharing::Request::Auth { magic } => state.note_request("mali_buffer_sharing", 0, &format!("auth({magic})")),
            _ => {}
        }
    }
}
