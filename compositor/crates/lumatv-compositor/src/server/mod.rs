//! The protocol layer: one module per global, each an `impl Dispatch<…> for State`.

pub mod closetrace;
pub mod compositor;
pub mod dmabuf;
pub mod foreign;
pub mod group;
pub mod ime;
pub mod input_manager;
pub mod mali;
pub mod misc;
pub mod output;
pub mod seat;
pub mod shell;
pub mod shm;
pub mod starfish;
pub mod subtitles;
pub mod state;
pub mod webos_shell;

/// `impl Dispatch` for an object whose requests the compositor only logs.
#[macro_export]
macro_rules! noop_dispatch {
    ($iface:ty, $udata:ty) => {
        impl wayland_server::Dispatch<$iface, $udata> for $crate::State {
            fn request(
                state: &mut Self,
                _client: &wayland_server::Client,
                resource: &$iface,
                request: <$iface as wayland_server::Resource>::Request,
                _data: &$udata,
                _dhandle: &wayland_server::DisplayHandle,
                _data_init: &mut wayland_server::DataInit<'_, Self>,
            ) {
                state.note_request(
                    <$iface as wayland_server::Resource>::interface().name,
                    wayland_server::Resource::id(resource).protocol_id(),
                    &format!("{request:?}"),
                );
            }
        }
    };
}

/// `impl GlobalDispatch` for a global whose bind creates a plain object with `()` data.
#[macro_export]
macro_rules! plain_global {
    ($iface:ty) => {
        impl wayland_server::GlobalDispatch<$iface, ()> for $crate::State {
            fn bind(
                state: &mut Self,
                _handle: &wayland_server::DisplayHandle,
                client: &wayland_server::Client,
                resource: wayland_server::New<$iface>,
                _global_data: &(),
                data_init: &mut wayland_server::DataInit<'_, Self>,
            ) {
                state.note_bind(client, <$iface as wayland_server::Resource>::interface().name);
                data_init.init(resource, ());
            }
        }
    };
}
