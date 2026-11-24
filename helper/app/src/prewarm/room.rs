//! Room in CMA for a requester: the converter's decoder and encoder take contiguous memory, which the apps kept ready hold on to
//! while they are hidden. A requester asks for N MB of `CmaFree`; the kept apps are closed in the owner's eviction order (the last of
//! the prewarm list first: Live TV, Apple TV, Prime Video, Netflix, Marquee last) until it is there, and nothing is launched again
//! while the [`Lease`] lives. When it is dropped the policy brings the closed apps back as its memory rules allow.

use super::{KEEP, Platform};
use parking_lot::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// After an app is closed, `CmaFree` is read this often, this many times, for the memory to come back.
const SETTLE_POLLS: u32 = 8;
const SETTLE_STEP: Duration = Duration::from_millis(250);

/// Leases alive: the policy launches nothing while any is.
static HELD: AtomicUsize = AtomicUsize::new(0);
/// The app on screen as of the last prewarm step, which is never closed for a requester.
static FRONT: Mutex<Option<String>> = Mutex::new(None);
static SYS: OnceLock<Box<dyn Platform + Send + Sync>> = OnceLock::new();

pub fn held() -> bool {
    HELD.load(Ordering::Acquire) > 0
}

pub(super) fn install(plat: Box<dyn Platform + Send + Sync>) {
    let _ = SYS.set(plat);
}

pub(super) fn set_front(front: Option<&str>) {
    *FRONT.lock() = front.map(str::to_owned);
}

/// Why there is no room: what `CmaFree` is, and what was closed on the way.
#[derive(Debug, PartialEq, Eq)]
pub struct Short {
    pub free_mb: u64,
    pub closed: Vec<String>,
}

/// Memory held for a requester; the kept apps stay closed until it is dropped.
#[derive(Debug)]
pub struct Lease {
    pub closed: Vec<String>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        HELD.fetch_sub(1, Ordering::AcqRel);
    }
}

/// `need_mb` of `CmaFree`, got by closing kept apps. Takes a few seconds when apps have to be closed (a closed app's memory comes back over
/// about a second), nothing when the room is there.
pub fn open(need_mb: u64) -> Result<Lease, Short> {
    // counted before anything is closed, so the policy does not bring an app back while it is being closed; an `Err` drops the lease
    HELD.fetch_add(1, Ordering::AcqRel);
    let mut lease = Lease { closed: Vec::new() };
    let Some(plat) = SYS.get() else { return Ok(lease) };
    let front = FRONT.lock().clone();
    lease.closed = clear(plat.as_ref(), &KEEP.order(), front.as_deref(), need_mb, &|| std::thread::sleep(SETTLE_STEP))?;
    Ok(lease)
}

/// Closes the kept apps that are out of sight and silent, last of `order` first, until `need_mb` of CMA is free. `pause` is how a closed app is
/// given time to let go. `Ok` with what was closed, or the shortfall.
pub fn clear(plat: &dyn Platform, order: &[String], front: Option<&str>, need_mb: u64, pause: &dyn Fn()) -> Result<Vec<String>, Short> {
    let mut closed = Vec::new();
    loop {
        // not readable: not a set this can judge, so nothing is held back
        let Some(free) = plat.cma_free_mb() else { return Ok(closed) };
        if free >= need_mb {
            return Ok(closed);
        }
        let running = plat.running();
        let playing = plat.playing();
        let victim = order.iter().rev().find(|app| {
            running.as_ref().is_some_and(|r| r.contains(app)) && front != Some(app.as_str()) && playing.as_ref().is_some_and(|p| !p.contains(*app))
        });
        let Some(app) = victim else {
            crate::log_info!("prewarm: nothing to close for {need_mb} MB of CMA ({free} MB free): kept {order:?}, running {running:?}, playing {playing:?}, front {front:?}");
            return Err(Short { free_mb: free, closed });
        };
        crate::log_info!("prewarm: closing {app} for {need_mb} MB of CMA ({free} MB free)");
        if !plat.close(app) {
            return Err(Short { free_mb: free, closed });
        }
        closed.push(app.clone());
        for _ in 0..SETTLE_POLLS {
            pause();
            if plat.cma_free_mb().is_some_and(|now| now > free) {
                break;
            }
        }
    }
}
