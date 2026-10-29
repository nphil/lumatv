//! The protocol tap (`scanout::tap`) on a real libwayland-server display: a client socket, a `mali_buffer_sharing` of the driver's shape (its
//! `create_buffer` answered by a C callback that makes the `wl_buffer` and closes the fd, as the blob's does), and what the tap keeps:
//! the descriptor under `(wl_client, id)`, its own duplicate of the fd after the callback closed the original, the entry gone with the
//! `wl_buffer.destroy` request, and the entries of a client that leaves without destroying its buffers gone with the client.
//!
//! The library is the host's (`libwayland-server.so.0`); a machine without it skips the test.

use std::ffi::{CString, c_char, c_int, c_void};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::OnceLock;

use lumatv_compositor::scanout::meta::Registry;
use lumatv_compositor::scanout::tap::{Status, Tap};

#[repr(C)]
struct Message {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const Interface,
}

#[repr(C)]
struct Interface {
    name: *const c_char,
    version: c_int,
    method_count: c_int,
    methods: *const Message,
    event_count: c_int,
    events: *const Message,
}

type Handler = unsafe extern "C" fn();

/// libwayland-server's functions the test drives.
struct Lib {
    display_create: unsafe extern "C" fn() -> *mut c_void,
    display_destroy: unsafe extern "C" fn(*mut c_void),
    display_event_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    loop_dispatch: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    client_create: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void,
    client_destroy: unsafe extern "C" fn(*mut c_void),
    resource_create: unsafe extern "C" fn(*mut c_void, *const Interface, c_int, u32) -> *mut c_void,
    resource_set_implementation: unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void, *const c_void),
    resource_destroy: unsafe extern "C" fn(*mut c_void),
    /// `wl_resource_post_event(resource, opcode, ...)`: variadic.
    resource_post_event: unsafe extern "C" fn(*mut c_void, u32, ...),
    /// The `wl_buffer` interface the callback makes resources of (an address, so the struct is `Sync`).
    buffer_interface: usize,
}

static LIB: OnceLock<Lib> = OnceLock::new();

/// The C function `name` in the library `lib`, as the function type the caller's binding names.
fn sym<T: Copy>(lib: *mut c_void, name: &str) -> T {
    let c = CString::new(name).unwrap();
    // SAFETY: a symbol lookup in a library handle the test opened.
    let p = unsafe { libc::dlsym(lib, c.as_ptr()) };
    assert!(!p.is_null(), "libwayland-server has no {name}");
    // SAFETY: `T` is the function type of the C declaration (a pointer-sized function pointer), as the field it is bound to says.
    unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) }
}

/// What the driver's `create_buffer` does: a `wl_buffer` resource of its own under the client's id, and the fd closed.
unsafe extern "C" fn create_buffer(
    client: *mut c_void,
    _resource: *mut c_void,
    id: u32,
    _w: i32,
    _h: i32,
    _stride: u32,
    _format: u32,
    _hi: u32,
    _lo: u32,
    fd: c_int,
) {
    let lib = LIB.get().unwrap();
    // SAFETY: a live client; the interface and implementation outlive the resource (the test keeps them to its end).
    unsafe {
        let buffer = (lib.resource_create)(client, lib.buffer_interface as *const Interface, 1, id);
        (lib.resource_set_implementation)(buffer, BUFFER_IMPL.as_ptr() as *const c_void, std::ptr::null_mut(), std::ptr::null());
        libc::close(fd);
    }
}

unsafe extern "C" fn auth(_client: *mut c_void, _resource: *mut c_void, _magic: u32) {}

/// `wl_buffer.destroy`: the resource goes.
unsafe extern "C" fn buffer_destroy(_client: *mut c_void, resource: *mut c_void) {
    // SAFETY: the resource this request arrived on.
    unsafe { (LIB.get().unwrap().resource_destroy)(resource) };
}

static MALI_IMPL: [Handler; 2] = [
    // SAFETY: the pointers are reinterpreted as the C handlers they are, with the signatures the interface tables below declare.
    unsafe { std::mem::transmute::<unsafe extern "C" fn(*mut c_void, *mut c_void, u32, i32, i32, u32, u32, u32, u32, c_int), Handler>(create_buffer) },
    // SAFETY: as above.
    unsafe { std::mem::transmute::<unsafe extern "C" fn(*mut c_void, *mut c_void, u32), Handler>(auth) },
];
static BUFFER_IMPL: [Handler; 1] = [
    // SAFETY: as above.
    unsafe { std::mem::transmute::<unsafe extern "C" fn(*mut c_void, *mut c_void), Handler>(buffer_destroy) },
];

fn memfd(len: i64) -> OwnedFd {
    // SAFETY: a memfd for the test, sized below.
    let fd = unsafe { libc::memfd_create(c"tap-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    // SAFETY: the descriptor is ours.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: sizing the memfd.
    assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), len) }, 0);
    fd
}

/// One wire message to the server, with a descriptor along when `fd` is given.
fn send(socket: c_int, words: &[u32], fd: Option<c_int>) {
    let mut iov = libc::iovec { iov_base: words.as_ptr() as *mut c_void, iov_len: words.len() * 4 };
    let mut control = [0u64; 8];
    // SAFETY: a zeroed msghdr with the fields below filled in; the control buffer is aligned and large enough for one descriptor.
    unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        if let Some(fd) = fd {
            msg.msg_control = control.as_mut_ptr() as *mut c_void;
            msg.msg_controllen = libc::CMSG_SPACE(4) as _;
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(4) as _;
            *(libc::CMSG_DATA(c) as *mut c_int) = fd;
        }
        assert!(libc::sendmsg(socket, &msg, 0) >= 0, "sendmsg: {}", std::io::Error::last_os_error());
    }
}

/// `mali_buffer_sharing.create_buffer(id, w, h, stride, format, modifier_hi, modifier_lo, fd)` on object `mali`.
fn create(socket: c_int, mali: u32, id: u32, size: (i32, i32, u32), fd: c_int) {
    let words = [mali, ((8 + 7 * 4) << 16), id, size.0 as u32, size.1 as u32, size.2, 0x3432_4241, 0x0800_0000, 0x151];
    send(socket, &words, Some(fd));
}

fn dispatch(lib: &Lib, event_loop: *mut c_void) {
    for _ in 0..3 {
        // SAFETY: the display's own loop, with a zero timeout.
        unsafe { (lib.loop_dispatch)(event_loop, 0) };
    }
}

#[test]
fn the_tap_keeps_what_the_driver_is_told_and_lets_go_with_the_buffer_or_the_client() {
    // SAFETY: opening the library by its soname; what is resident is shared, not loaded twice.
    let handle = unsafe { libc::dlopen(c"libwayland-server.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        eprintln!("skipped: no libwayland-server.so.0 on this machine");
        return;
    }
    let lib = LIB.get_or_init(|| {
        let buffer_name = CString::new("wl_buffer").unwrap().into_raw();
        let destroy = CString::new("destroy").unwrap().into_raw();
        let empty = CString::new("").unwrap().into_raw();
        let release = CString::new("release").unwrap().into_raw();
        let buffer_methods = Box::into_raw(Box::new([Message { name: destroy, signature: empty, types: std::ptr::null() }]));
        let buffer_events = Box::into_raw(Box::new([Message { name: release, signature: empty, types: std::ptr::null() }]));
        let buffer_interface = Box::into_raw(Box::new(Interface {
            name: buffer_name,
            version: 1,
            method_count: 1,
            methods: buffer_methods as *const Message,
            event_count: 1,
            events: buffer_events as *const Message,
        })) as usize;
        Lib {
            display_create: sym(handle, "wl_display_create"),
            display_destroy: sym(handle, "wl_display_destroy"),
            display_event_loop: sym(handle, "wl_display_get_event_loop"),
            loop_dispatch: sym(handle, "wl_event_loop_dispatch"),
            client_create: sym(handle, "wl_client_create"),
            client_destroy: sym(handle, "wl_client_destroy"),
            resource_create: sym(handle, "wl_resource_create"),
            resource_set_implementation: sym(handle, "wl_resource_set_implementation"),
            resource_destroy: sym(handle, "wl_resource_destroy"),
            resource_post_event: sym(handle, "wl_resource_post_event"),
            buffer_interface,
        }
    });

    // The driver's interface, as its own table has it (docs/research/arch/x2-libmali32-protocols.txt).
    let new_buffer_type: [*const Interface; 8] = [
        lib.buffer_interface as *const Interface,
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
        std::ptr::null(),
    ];
    let auth_type: [*const Interface; 1] = [std::ptr::null()];
    let (name, create_name, sig, auth_name, auth_sig, alloc_name, alloc_sig) = (
        CString::new("mali_buffer_sharing").unwrap(),
        CString::new("create_buffer").unwrap(),
        CString::new("niiuuuuh").unwrap(),
        CString::new("auth").unwrap(),
        CString::new("u").unwrap(),
        CString::new("alloc_device").unwrap(),
        CString::new("s").unwrap(),
    );
    let methods = [
        Message { name: create_name.as_ptr(), signature: sig.as_ptr(), types: new_buffer_type.as_ptr() },
        Message { name: auth_name.as_ptr(), signature: auth_sig.as_ptr(), types: auth_type.as_ptr() },
    ];
    let events = [Message { name: alloc_name.as_ptr(), signature: alloc_sig.as_ptr(), types: auth_type.as_ptr() }];
    let mali = Interface { name: name.as_ptr(), version: 5, method_count: 2, methods: methods.as_ptr(), event_count: 1, events: events.as_ptr() };

    // SAFETY: a display of its own; every pointer below is the library's or the test's own and outlives its use.
    unsafe {
        let display = (lib.display_create)();
        assert!(!display.is_null());
        let event_loop = (lib.display_event_loop)(display);
        let registry = Registry::default();
        let tap = Tap::install(Some(display), &registry);
        assert_eq!(tap.status(), Status::Active, "the host's libwayland has the protocol logger");

        let mut pair = [0 as c_int; 2];
        assert_eq!(libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0, pair.as_mut_ptr()), 0);
        let (server_end, client_end) = (pair[0], pair[1]);
        let client = (lib.client_create)(display, server_end);
        assert!(!client.is_null());
        let key = |id: u32| (client as usize, id);
        // The driver's global as the client has bound it: object 2 (the client's ids are handed out one after the other, the display is 1).
        let resource = (lib.resource_create)(client, &mali, 5, 2);
        assert!(!resource.is_null());
        (lib.resource_set_implementation)(resource, MALI_IMPL.as_ptr() as *const c_void, std::ptr::null_mut(), std::ptr::null());

        // The driver tells its client where to allocate from (the event the tap reads, `alloc_device(path)`, opcode 0).
        (lib.resource_post_event)(resource, 0, c"/dev/dma_buf_te".as_ptr());
        assert_eq!(registry.alloc_devices(), vec!["/dev/dma_buf_te".to_string()]);

        // A 1080p AB24 AFBC buffer, as LG's libmali asks for one.
        let dmabuf = memfd(8_986_624);
        create(client_end, 2, 3, (1920, 1080, 7680), dmabuf.as_raw_fd());
        dispatch(lib, event_loop);
        let d = registry.lookup(key(3)).expect("the descriptor is kept under the client and the new id");
        assert_eq!((d.width, d.height, d.stride, d.format), (1920, 1080, 7680, 0x3432_4241));
        assert_eq!(d.modifier, 0x0800_0000_0000_0151);
        assert_eq!(d.bytes(), Some(8_986_624), "the duplicate is the same dma-buf, and it is still open: the callback closed the original");
        assert_eq!(registry.stats().undecoded, 0);

        // A second buffer of the same client, and the first destroyed through the protocol.
        create(client_end, 2, 4, (256, 256, 1024), dmabuf.as_raw_fd());
        dispatch(lib, event_loop);
        assert_eq!(registry.stats().retained, 2);
        send(client_end, &[3, 8 << 16], None);
        dispatch(lib, event_loop);
        assert!(registry.lookup(key(3)).is_none(), "wl_buffer.destroy ends the entry");
        assert!(registry.lookup(key(4)).is_some(), "and only that buffer's");

        // A client that leaves with a buffer still alive: the entry goes with it.
        (lib.client_destroy)(client);
        let s = registry.stats();
        assert_eq!((s.retained, s.destroyed, s.with_client), (0, 1, 1), "{s:?}");
        libc::close(client_end);

        drop(tap);
        (lib.display_destroy)(display);
    }
}
