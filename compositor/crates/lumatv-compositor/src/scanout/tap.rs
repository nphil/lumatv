//! The protocol tap: a libwayland protocol logger on the compositor's display that copies the descriptor out of `mali_buffer_sharing`'s
//! `create_buffer` on its way to the driver (`docs/research/scanout-research.md` route A).
//!
//! With `--buffers driver` the Mali blob serves that global itself, on the same C `wl_display` (`eglBindWaylandDisplayWL`), and answers
//! its clients' requests in its own callbacks: the Rust side never sees a driver-created buffer's size, stride, format or modifier, and
//! cannot read its fd. A logger registered with `wl_display_add_protocol_logger` is called for every message before it is dispatched, with
//! the request's arguments, fds included, still owned by libwayland's closure. The tap duplicates the fd and keeps the descriptor in the
//! [`Registry`] by `(wl_client, new buffer id)`; the request goes on to the driver untouched, which creates the resource, owns its copy of
//! the fd and imports the buffer as before. A `wl_buffer.destroy` removes the entry, and a destroy listener on the client removes what a
//! client that left without destroying its buffers held.
//!
//! libwayland-server is already loaded by `wayland-backend` when the display exists, so the entry points are looked up in it by name
//! (`dlopen` with `RTLD_NOLOAD`, which takes no second copy); a library without the logger leaves the tap unavailable, with the reason.
//! The [`Tap`] owns the registration: dropping it removes the logger and releases the state it was given.

use std::collections::HashSet;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::ManuallyDrop;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::meta::{Descriptor, Key, Registry};

/// The wire signature of `mali_buffer_sharing.create_buffer` (`id, width, height, stride, format, modifier_hi, modifier_lo, fd`), as the
/// driver's own table has it (`docs/research/arch/x2-libmali32-protocols.txt`). A message with another signature is not read.
pub const CREATE_BUFFER_SIGNATURE: &str = "niiuuuuh";

/// Whether the tap is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not asked for: nothing is retained.
    Off,
    Active,
    /// Asked for and not possible, and why.
    Unavailable(String),
}

impl Status {
    pub fn report(&self) -> Value {
        match self {
            Status::Off => json!({"state": "off"}),
            Status::Active => json!({"state": "active"}),
            Status::Unavailable(why) => json!({"state": "unavailable", "why": why}),
        }
    }

    pub fn is_active(&self) -> bool {
        *self == Status::Active
    }
}

/// The arguments of one `create_buffer`, as the wire carries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateBuffer {
    pub id: u32,
    pub width: i32,
    pub height: i32,
    pub stride: u32,
    pub format: u32,
    pub modifier_hi: u32,
    pub modifier_lo: u32,
    pub fd: i32,
}

/// The descriptor a request describes, with its own duplicate of the fd (the driver's handler owns the original and closes it when it has
/// done with it). `None` for arguments no buffer can have.
pub fn descriptor(a: &CreateBuffer) -> Option<Descriptor> {
    if a.width <= 0 || a.height <= 0 || a.stride == 0 || a.fd < 0 {
        return None;
    }
    // SAFETY: duplicating a descriptor libwayland still holds for the closure; the duplicate is ours, close-on-exec.
    let dup = unsafe { libc::fcntl(a.fd, libc::F_DUPFD_CLOEXEC, 3) };
    if dup < 0 {
        return None;
    }
    Some(Descriptor {
        width: a.width,
        height: a.height,
        stride: a.stride,
        format: a.format,
        modifier: (u64::from(a.modifier_hi) << 32) | u64::from(a.modifier_lo),
        // SAFETY: `dup` is a fresh descriptor nothing else owns.
        fd: unsafe { OwnedFd::from_raw_fd(dup) },
        serial: 0,
    })
}

#[repr(C)]
struct WlMessage {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const c_void,
}

/// `union wl_argument`: eight bytes on a 64-bit target, every member at offset 0.
#[repr(C)]
#[derive(Clone, Copy)]
union WlArgument {
    i: i32,
    u: u32,
    s: *const c_char,
    o: *mut c_void,
    a: *mut c_void,
    h: i32,
}

#[repr(C)]
struct LoggerMessage {
    resource: *mut c_void,
    message_opcode: c_int,
    message: *const WlMessage,
    arguments_count: c_int,
    arguments: *const WlArgument,
}

#[repr(C)]
struct WlList {
    prev: *mut WlList,
    next: *mut WlList,
}

/// `struct wl_listener` followed by what the tap needs to find its way back: libwayland touches only the first two fields. It holds one
/// count of the shared state, given back when it is notified.
#[repr(C)]
struct Listener {
    link: WlList,
    notify: unsafe extern "C" fn(*mut Listener, *mut c_void),
    client: usize,
    shared: *const Shared,
}

type LoggerFn = unsafe extern "C" fn(*mut c_void, c_int, *const LoggerMessage);

/// `WL_PROTOCOL_LOGGER_REQUEST`.
const LOGGER_REQUEST: c_int = 0;

/// `WL_PROTOCOL_LOGGER_EVENT`.
const LOGGER_EVENT: c_int = 1;

/// The entry points of libwayland-server the tap uses.
struct Sys {
    add_logger: unsafe extern "C" fn(*mut c_void, LoggerFn, *mut c_void) -> *mut c_void,
    logger_destroy: unsafe extern "C" fn(*mut c_void),
    resource_client: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    resource_id: unsafe extern "C" fn(*mut c_void) -> u32,
    resource_class: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    client_add_destroy_listener: unsafe extern "C" fn(*mut c_void, *mut Listener),
    list_remove: unsafe extern "C" fn(*mut WlList),
}

impl Sys {
    fn load() -> Result<Sys, String> {
        fn sym<T: Copy>(lib: *mut c_void, name: &CStr) -> Result<T, String> {
            // SAFETY: a symbol lookup in a library handle; the pointer is only reinterpreted as the C function it names.
            let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
            if p.is_null() {
                return Err(format!("libwayland-server has no {}", name.to_string_lossy()));
            }
            // SAFETY: the caller names the function type of the symbol.
            Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
        }
        let mut lib = std::ptr::null_mut();
        for name in [c"libwayland-server.so.0", c"libwayland-server.so"] {
            // SAFETY: RTLD_NOLOAD never loads anything, it only finds what is resident.
            lib = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_NOLOAD) };
            if !lib.is_null() {
                break;
            }
        }
        if lib.is_null() {
            lib = libc::RTLD_DEFAULT;
        }
        Ok(Sys {
            add_logger: sym(lib, c"wl_display_add_protocol_logger")?,
            logger_destroy: sym(lib, c"wl_protocol_logger_destroy")?,
            resource_client: sym(lib, c"wl_resource_get_client")?,
            resource_id: sym(lib, c"wl_resource_get_id")?,
            resource_class: sym(lib, c"wl_resource_get_class")?,
            client_add_destroy_listener: sym(lib, c"wl_client_add_destroy_listener")?,
            list_remove: sym(lib, c"wl_list_remove")?,
        })
    }
}

static SYS: LazyLock<Result<Sys, String>> = LazyLock::new(Sys::load);

fn sys() -> Result<&'static Sys, String> {
    SYS.as_ref().map_err(Clone::clone)
}

/// The tap's state: the logger and the client listeners each hold a count of it.
struct Shared {
    sys: &'static Sys,
    registry: Registry,
    watched: Mutex<HashSet<usize>>,
}

impl Shared {
    /// # Safety
    /// `m` is a live logger message of a `create_buffer` request.
    unsafe fn created(this: &Arc<Shared>, m: &LoggerMessage) {
        // SAFETY: the message and its signature are libwayland's, valid for this call.
        let signature = unsafe { CStr::from_ptr((*m.message).signature) }.to_bytes();
        if signature != CREATE_BUFFER_SIGNATURE.as_bytes() || m.arguments_count != 8 {
            this.registry.note_undecoded();
            return;
        }
        // SAFETY: eight arguments, as counted above.
        let a = unsafe { std::slice::from_raw_parts(m.arguments, 8) };
        // SAFETY: the signature says which member each argument is.
        let args = unsafe {
            CreateBuffer { id: a[0].u, width: a[1].i, height: a[2].i, stride: a[3].u, format: a[4].u, modifier_hi: a[5].u, modifier_lo: a[6].u, fd: a[7].h }
        };
        let Some(d) = descriptor(&args) else {
            this.registry.note_undecoded();
            return;
        };
        // SAFETY: a live resource of this request.
        let client = unsafe { (this.sys.resource_client)(m.resource) } as usize;
        this.registry.insert((client, args.id), d);
        Shared::watch(this, client);
    }

    /// Asks to be told when `client` is destroyed, once per client.
    fn watch(this: &Arc<Shared>, client: usize) {
        if !this.watched.lock().insert(client) {
            return;
        }
        let listener = Box::into_raw(Box::new(Listener {
            link: WlList { prev: std::ptr::null_mut(), next: std::ptr::null_mut() },
            notify: client_destroyed,
            client,
            shared: Arc::into_raw(this.clone()),
        }));
        // SAFETY: a live client of the request being logged; the listener stays allocated until its own notify frees it.
        unsafe { (this.sys.client_add_destroy_listener)(client as *mut c_void, listener) };
    }
}

/// A client went away: what it still held goes with it.
///
/// # Safety
/// libwayland calls this with the listener the tap registered, from the client's destruction.
unsafe extern "C" fn client_destroyed(listener: *mut Listener, _data: *mut c_void) {
    // SAFETY: the listener is the Box the tap gave libwayland; it is unlinked first, so libwayland's safe iteration goes on to the next one,
    // and then freed, with the count of the shared state it held.
    unsafe {
        let shared = Arc::from_raw((*listener).shared);
        (shared.sys.list_remove)(&mut (*listener).link);
        let l = Box::from_raw(listener);
        shared.registry.client_gone(l.client);
        shared.watched.lock().remove(&l.client);
    }
}

/// libwayland's callback for every message: the requests and the one event the tap reads, nothing else. No allocation unless a buffer is made.
unsafe extern "C" fn log_message(user: *mut c_void, direction: c_int, message: *const LoggerMessage) {
    if (direction != LOGGER_REQUEST && direction != LOGGER_EVENT) || user.is_null() || message.is_null() {
        return;
    }
    // SAFETY: `user` is the pointer of the count the logger holds (`Tap::install`), borrowed here without taking it; `message` is
    // libwayland's, valid for this call.
    let (shared, m) = unsafe { (ManuallyDrop::new(Arc::from_raw(user as *const Shared)), &*message) };
    if m.resource.is_null() || m.message.is_null() {
        return;
    }
    // SAFETY: a live resource and its interface name, a live message and its name.
    let (class, name) = unsafe {
        let class = (shared.sys.resource_class)(m.resource);
        if class.is_null() || (*m.message).name.is_null() {
            return;
        }
        (CStr::from_ptr(class).to_bytes(), CStr::from_ptr((*m.message).name).to_bytes())
    };
    match (direction, class, name) {
        (LOGGER_REQUEST, b"mali_buffer_sharing", b"create_buffer") => {
            // SAFETY: the message is a create_buffer request.
            unsafe { Shared::created(&shared, m) }
        }
        (LOGGER_REQUEST, b"wl_buffer", b"destroy") => {
            // SAFETY: a live resource.
            let key = unsafe { ((shared.sys.resource_client)(m.resource) as usize, (shared.sys.resource_id)(m.resource)) };
            shared.registry.remove(key);
        }
        (LOGGER_EVENT, b"mali_buffer_sharing", b"alloc_device") if m.arguments_count == 1 && !m.arguments.is_null() => {
            // SAFETY: the event's one argument is a string (signature `s`), a NUL-terminated one when it is not null.
            let path = unsafe {
                let s = (*m.arguments).s;
                (!s.is_null()).then(|| CStr::from_ptr(s).to_string_lossy().into_owned())
            };
            if let Some(path) = path {
                shared.registry.note_alloc_device(path);
            }
        }
        _ => {}
    }
}

/// The key of a resource the C backend owns: its client and its object id, the pair the tap files descriptors under.
///
/// # Safety
/// `resource` is null or a live `wl_resource`.
pub unsafe fn resource_key(resource: *mut c_void) -> Option<Key> {
    if resource.is_null() {
        return None;
    }
    let sys = sys().ok()?;
    // SAFETY: a live wl_resource, by the caller's contract.
    Some(unsafe { ((sys.resource_client)(resource) as usize, (sys.resource_id)(resource)) })
}

/// The registration of the logger on a display, and what says whether it took.
pub struct Tap {
    status: Status,
    /// The logger and the count of the shared state it holds, while it is registered.
    registered: Option<(*mut c_void, *const Shared)>,
}

impl Tap {
    /// A tap that was not asked for.
    pub fn off() -> Tap {
        Tap { status: Status::Off, registered: None }
    }

    /// Puts the tap on `display` (the C `wl_display` the driver binds, `None` on a build without one). What descriptors it sees go to `registry`.
    pub fn install(display: Option<*mut c_void>, registry: &Registry) -> Tap {
        let unavailable = |why: String| Tap { status: Status::Unavailable(why), registered: None };
        let Some(display) = display.filter(|d| !d.is_null()) else {
            return unavailable("no C libwayland display (this build runs the Rust backend)".into());
        };
        let sys = match sys() {
            Ok(s) => s,
            Err(why) => return unavailable(why),
        };
        let shared = Arc::into_raw(Arc::new(Shared { sys, registry: registry.clone(), watched: Mutex::new(HashSet::new()) }));
        // SAFETY: a live wl_display, a callback with libwayland's signature and the count of the shared state it is given back.
        let logger = unsafe { (sys.add_logger)(display, log_message, shared as *mut c_void) };
        if logger.is_null() {
            // SAFETY: the count taken above, never handed over.
            drop(unsafe { Arc::from_raw(shared) });
            return unavailable("wl_display_add_protocol_logger refused the logger".into());
        }
        Tap { status: Status::Active, registered: Some((logger, shared)) }
    }

    pub fn status(&self) -> Status {
        self.status.clone()
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        if let Some((logger, shared)) = self.registered.take()
            && let Ok(sys) = sys()
        {
            // SAFETY: the logger this tap registered, removed once, and then the count it held.
            unsafe {
                (sys.logger_destroy)(logger);
                drop(Arc::from_raw(shared));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    fn memfd() -> OwnedFd {
        // SAFETY: a memfd for the test.
        let fd = unsafe { libc::memfd_create(c"scanout-tap".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        // SAFETY: the descriptor is ours.
        unsafe { OwnedFd::from_raw_fd(fd) }
    }

    fn request(fd: i32) -> CreateBuffer {
        CreateBuffer { id: 9, width: 1920, height: 1080, stride: 7680, format: 0x3432_4241, modifier_hi: 0x0800_0000, modifier_lo: 0x151, fd }
    }

    #[test]
    fn the_descriptor_is_the_requests_with_a_duplicate_of_its_fd() {
        let original = memfd();
        let d = descriptor(&request(original.as_raw_fd())).unwrap();
        assert_eq!((d.width, d.height, d.stride, d.format), (1920, 1080, 7680, 0x3432_4241));
        assert_eq!(d.modifier, 0x0800_0000_0000_0151, "the two words are the halves of the modifier");
        assert_ne!(d.fd.as_raw_fd(), original.as_raw_fd());
        drop(original);
        // SAFETY: fstat of the duplicate on a stack struct: it outlives the original, which is what the tap is for.
        let alive = unsafe {
            let mut st: libc::stat = std::mem::zeroed();
            libc::fstat(d.fd.as_raw_fd(), &mut st) == 0
        };
        assert!(alive, "the driver closes its fd, the tap's is still a dma-buf");
    }

    #[test]
    fn arguments_no_buffer_can_have_are_not_a_descriptor() {
        let fd = memfd();
        let ok = request(fd.as_raw_fd());
        assert!(descriptor(&CreateBuffer { width: 0, ..ok }).is_none());
        assert!(descriptor(&CreateBuffer { height: -4, ..ok }).is_none());
        assert!(descriptor(&CreateBuffer { stride: 0, ..ok }).is_none());
        assert!(descriptor(&CreateBuffer { fd: -1, ..ok }).is_none());
        assert!(descriptor(&CreateBuffer { fd: 987_654, ..ok }).is_none(), "a descriptor that is not open");
    }

    #[test]
    fn the_signature_is_the_drivers_table() {
        // docs/research/arch/x2-libmali32-protocols.txt: request[0] create_buffer sig=niiuuuuh
        assert_eq!(CREATE_BUFFER_SIGNATURE, "niiuuuuh");
        assert_eq!(CREATE_BUFFER_SIGNATURE.len(), 8, "the eight arguments the tap reads");
    }

    #[test]
    fn without_a_c_display_the_tap_says_why_it_is_not_on() {
        let r = Registry::default();
        assert!(matches!(Tap::install(None, &r).status(), Status::Unavailable(_)));
        assert!(matches!(Tap::install(Some(std::ptr::null_mut()), &r).status(), Status::Unavailable(_)));
        assert_eq!(Tap::off().status(), Status::Off);
        assert_eq!(Status::Off.report()["state"], "off");
        assert_eq!(Status::Unavailable("x".into()).report()["why"], "x");
        assert!(Status::Active.is_active() && !Status::Off.is_active());
    }
}
