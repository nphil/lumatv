//! `wl_shm`, `wl_shm_pool` and the `wl_buffer` objects this compositor creates (shm and dma-buf alike).

use std::sync::Arc;

use wayland_backend::server::ClientId;
use wayland_server::protocol::wl_buffer::{self, WlBuffer};
use wayland_server::protocol::wl_shm::{self, WlShm};
use wayland_server::protocol::wl_shm_pool::{self, WlShmPool};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum};

use crate::buffer::{BufferData, BufferKind, ShmBuffer, ShmPool};
use crate::State;

impl GlobalDispatch<WlShm, ()> for State {
    fn bind(_state: &mut Self, _handle: &DisplayHandle, _client: &Client, resource: New<WlShm>, _global_data: &(), data_init: &mut DataInit<'_, Self>) {
        let shm = data_init.init(resource, ());
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
    }
}

impl Dispatch<WlShm, ()> for State {
    fn request(state: &mut Self, client: &Client, resource: &WlShm, request: wl_shm::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        state.touch_client(client);
        match request {
            wl_shm::Request::CreatePool { id, fd, size } => match ShmPool::new(fd, size) {
                Ok(pool) => {
                    data_init.init(id, pool);
                    state.note_request("wl_shm", 0, &format!("create_pool({size})"));
                }
                Err(e) => resource.post_error(wl_shm::Error::InvalidFd, format!("cannot map pool: {e}")),
            },
            wl_shm::Request::Release => {}
            _ => {}
        }
    }
}

impl Dispatch<WlShmPool, Arc<ShmPool>> for State {
    fn request(state: &mut Self, _client: &Client, resource: &WlShmPool, request: wl_shm_pool::Request, pool: &Arc<ShmPool>, _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        match request {
            wl_shm_pool::Request::CreateBuffer { id, offset, width, height, stride, format } => {
                let format = match format {
                    WEnum::Value(wl_shm::Format::Argb8888) => 0,
                    WEnum::Value(wl_shm::Format::Xrgb8888) => 1,
                    other => {
                        resource.post_error(wl_shm::Error::InvalidFormat, format!("format {other:?} not offered"));
                        return;
                    }
                };
                if width <= 0 || height <= 0 || stride < width * 4 || offset < 0 || (offset as i64 + stride as i64 * height as i64) > pool.len() as i64 {
                    resource.post_error(wl_shm::Error::InvalidStride, format!("buffer {width}x{height} stride {stride} offset {offset} does not fit the pool of {} bytes", pool.len()));
                    return;
                }
                let b = data_init.init(id, BufferData { kind: BufferKind::Shm(ShmBuffer { pool: pool.clone(), offset, width, height, stride, format }) });
                state.stats.buffers_created += 1;
                state.note_request("wl_shm_pool", 0, &format!("create_buffer -> wl_buffer@{} {width}x{height} stride {stride} {}", b.id().protocol_id(), if format == 0 { "ARGB8888" } else { "XRGB8888" }));
            }
            wl_shm_pool::Request::Resize { size } => {
                if let Err(e) = pool.resize(size) {
                    resource.post_error(wl_shm::Error::InvalidFd, format!("cannot resize pool: {e}"));
                }
            }
            wl_shm_pool::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, BufferData> for State {
    fn request(state: &mut Self, _client: &Client, resource: &WlBuffer, request: wl_buffer::Request, _data: &BufferData, _dh: &DisplayHandle, _di: &mut DataInit<'_, Self>) {
        if let wl_buffer::Request::Destroy = request {
            state.note_request("wl_buffer", resource.id().protocol_id(), "destroy");
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &WlBuffer, _data: &BufferData) {
        state.output.forget_buffer(resource);
        // A destroyed buffer must drop out of every surface that still points at it.
        for s in state.surfaces.iter_mut() {
            if s.current.buffer.as_ref().map(|b| b.id() == resource.id()).unwrap_or(false) {
                s.current.buffer = None;
                s.current.size = None;
            }
            s.current.retired.retain(|b| b.id() != resource.id());
            if let Some(Some(b)) = &s.pending.buffer
                && b.id() == resource.id()
            {
                s.pending.buffer = Some(None);
            }
        }
    }
}
