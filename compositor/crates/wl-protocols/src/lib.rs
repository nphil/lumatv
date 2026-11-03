//! The Wayland protocols LG's webOS clients speak beyond the core, generated from XML.
//!
//! `protocol/webos/*.xml` started as the public webOS OSE files (webos-wayland-extensions, commit 4059350fd2be) and
//! are folded to LG's own tables as read on the set by `docs/research/arch/wlproto_from_elf.py`
//! (`protocol/lg-tables.txt`, checked by `tests/lg_tables.rs`): opcodes are positional, and LG's order and
//! versions differ from OSE's in six interfaces. The private protocols include LG's `mali_buffer_sharing` and
//! `wl_starfish_*` interfaces, plus LumaTV's optional subtitle bounds hint.
//!
//! Each protocol module has a `server` submodule (the compositor) and, behind the `client` feature, a `client`
//! submodule (the test client).

#![allow(clippy::all)]

macro_rules! protocol {
    ($name:ident, $path:expr) => {
        pub mod $name {
            #[cfg(feature = "client")]
            pub use self::generated::client;
            #[cfg(feature = "server")]
            pub use self::generated::server;

            mod generated {
                #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
                #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
                #![allow(missing_docs, clippy::all)]

                #[cfg(feature = "client")]
                pub mod client {
                    use wayland_client;
                    use wayland_client::protocol::*;

                    pub mod __interfaces {
                        use wayland_client::protocol::__interfaces::*;
                        wayland_scanner::generate_interfaces!($path);
                    }
                    use self::__interfaces::*;

                    wayland_scanner::generate_client_code!($path);
                }

                #[cfg(feature = "server")]
                pub mod server {
                    use wayland_server;
                    use wayland_server::protocol::*;

                    pub mod __interfaces {
                        use wayland_server::protocol::__interfaces::*;
                        wayland_scanner::generate_interfaces!($path);
                    }
                    use self::__interfaces::*;

                    wayland_scanner::generate_server_code!($path);
                }
            }
        }
    };
}

protocol!(webos_shell, "../../protocol/webos/webos-shell.xml");
protocol!(webos_foreign, "../../protocol/webos/webos-foreign.xml");
protocol!(webos_surface_group, "../../protocol/webos/webos-surface-group.xml");
protocol!(webos_input_manager, "../../protocol/webos/webos-input-manager.xml");
protocol!(webos_extension, "../../protocol/webos/webos-extension.xml");
protocol!(webos_system_ux, "../../protocol/webos/webos-system-ux.xml");
protocol!(webos_tablet, "../../protocol/webos/webos-tablet.xml");
protocol!(input_method, "../../protocol/webos/input-method.xml");
protocol!(text, "../../protocol/webos/text.xml");
protocol!(mali_buffer_sharing, "../../protocol/private/mali-buffer-sharing.xml");
protocol!(starfish, "../../protocol/private/starfish.xml");
protocol!(lumatv_subtitles_v1, "../../protocol/private/lumatv-subtitles-v1.xml");
