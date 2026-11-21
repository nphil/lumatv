//! The commit thread: every atomic commit of the output (a frame's page flip, a buffer's test-only commit) is made here, blocking, and
//! never on the frame loop.
//!
//! Why a thread of its own: a non-blocking commit returns at once and leaves the flip to the kernel's `commit_work`, an ordinary-priority
//! worker that waits for the frame's fence and then programs the planes. On the 120 Hz gates of 2026-11-21 that worker lost the refresh
//! with the GPU done 1.92 ms and 0.81 ms before it: the UI's threads (nice -19) held its CPU in one, the loop's own FIFO pass in the
//! other. A blocking commit does that wait and that programming in the thread that made the ioctl, so they run at this thread's priority.
//! It also holds the display's locks until the flip is on the screen, which is why no call that takes them is left on the loop.
//!
//! Priority: SCHED_FIFO one above the loop's (21 for the default 20). The moment that matters is the fence's signal, and the thread must
//! take the CPU then from everything that held up `commit_work`: the time-shared UI, and the loop itself. Below LG's input (RR 99) and
//! the HDMI interrupt thread (FIFO 50). Its CPU is bounded by the work: it sleeps in `recv` between jobs and in the kernel during the
//! fence and vblank waits, and it is given at most one frame at a time (`Kms` sends the next only after this one is answered), tens of
//! microseconds of register writes per refresh; the kernel's real-time cap (`sched_rt_runtime_us`) is the backstop, as for the loop.
//!
//! Ownership and order: the loop owns the `Committer` and is the only sender; jobs run in the order sent; a commit sent while one is
//! outstanding waits in the channel behind it. A commit's answer comes back with a write to the wake descriptor, which the loop polls with
//! the card's events (`Kms::event_fd`). A test-only commit is answered at once on the same channel and only while no commit is
//! outstanding: then the loop waits for the ioctl as it did when it made it itself, and never behind a flip.
//!
//! Left elsewhere: the pointer's thread (`pointer`) makes its cursor calls on its own duplicate of the card, already off the loop, and its
//! legacy route does not wait for a vblank. Framebuffer removal stays on the loop: `RMFB` takes the locks only for a framebuffer the
//! kernel still holds, and the output removes only those the display has let go of (`scanout`'s holds, the page ring turning one ahead of
//! the flip), after the thread has answered for the commit that replaced them (`Kms::dispatch_events`). Property reads happen once, in
//! `Kms::open`, before the thread exists.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::Instant;

use drm::control::{AtomicCommitFlags, Device as ControlDevice, atomic::AtomicModeReq};

use super::Card;

/// The thread's name, as `ps` and the scheduler trace show it.
pub const THREAD_NAME: &str = "lumatv-commit";

/// The calls that take the display's locks. The output's is the card; the tests' record the thread that makes them.
pub trait Submit: Send + 'static {
    type Req: Send + 'static;
    /// A blocking commit with a page-flip event, the mode with it when `modeset`: returns once the flip is on the screen. Errno on refusal.
    fn commit(&mut self, req: Self::Req, modeset: bool) -> Result<(), i32>;
    /// A test-only commit: whether the driver would take the state.
    fn test(&mut self, req: Self::Req) -> Result<(), i32>;
}

/// The card as the thread's [`Submit`]: a duplicate of the output's descriptor, the same DRM master.
pub struct CardSubmit(pub Card);

fn errno(e: io::Error) -> i32 {
    e.raw_os_error().unwrap_or(libc::EIO)
}

impl Submit for CardSubmit {
    type Req = AtomicModeReq;

    fn commit(&mut self, req: AtomicModeReq, modeset: bool) -> Result<(), i32> {
        let mut flags = AtomicCommitFlags::PAGE_FLIP_EVENT;
        if modeset {
            flags |= AtomicCommitFlags::ALLOW_MODESET;
        }
        self.0.atomic_commit(flags, req).map_err(errno)
    }

    fn test(&mut self, req: AtomicModeReq) -> Result<(), i32> {
        self.0.atomic_commit(AtomicCommitFlags::TEST_ONLY, req).map_err(errno)
    }
}

enum Job<R> {
    /// The in-fence's descriptor rides along: the request names it by number, so it stays open until the ioctl has taken the fence.
    Commit { req: R, modeset: bool, _fence: Option<OwnedFd> },
    Test(R),
}

/// A commit's answer.
#[derive(Debug, PartialEq, Eq)]
pub struct Done {
    pub result: Result<(), i32>,
    /// How long the ioctl took: the fence's wait and the flip's.
    pub took_ns: u64,
}

enum Reply {
    Committed(Done),
    Tested(Result<(), i32>),
}

pub struct Committer<S: Submit> {
    jobs: Option<Sender<Job<S::Req>>>,
    replies: Receiver<Reply>,
    wake: OwnedFd,
    /// Commits sent and not yet answered through [`Committer::take_done`].
    outstanding: usize,
    /// Answers made here for commits the thread could not take (it has stopped).
    undelivered: Vec<Done>,
    worker: Option<JoinHandle<()>>,
}

impl<S: Submit> Committer<S> {
    /// Starts the thread; `priority` is its SCHED_FIFO priority, 0 for time-shared.
    pub fn start(mut submit: S, priority: i32) -> io::Result<Committer<S>> {
        let wake = crate::shell::eventfd()?;
        let thread_wake = wake.try_clone()?;
        let (jobs, inbox) = channel::<Job<S::Req>>();
        let (outbox, replies) = channel();
        let worker = std::thread::Builder::new().name(THREAD_NAME.into()).spawn(move || {
            if priority > 0 {
                match crate::sched::fifo(priority) {
                    Ok(()) => log::info!("kms: commit thread: {}", crate::sched::describe()),
                    Err(e) => log::warn!("kms: commit thread: SCHED_FIFO {priority} refused ({e}); {}", crate::sched::describe()),
                }
            }
            // Ends when the loop drops its sender, after every job sent before.
            for job in inbox {
                let reply = match job {
                    Job::Commit { req, modeset, _fence } => {
                        let t = Instant::now();
                        let result = submit.commit(req, modeset);
                        Reply::Committed(Done { result, took_ns: t.elapsed().as_nanos() as u64 })
                    }
                    Job::Test(req) => Reply::Tested(submit.test(req)),
                };
                let committed = matches!(reply, Reply::Committed(_));
                if outbox.send(reply).is_err() {
                    break;
                }
                // A test's answer is waited for on the channel; a commit's is polled for with the card's events.
                if committed {
                    crate::shell::wake(&thread_wake);
                }
            }
        })?;
        Ok(Committer { jobs: Some(jobs), replies, wake, outstanding: 0, undelivered: Vec::new(), worker: Some(worker) })
    }

    /// The descriptor that turns readable when a commit has been answered.
    pub fn wake_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// Whether a commit sent has not been answered yet.
    pub fn busy(&self) -> bool {
        self.outstanding > 0
    }

    /// Sends a commit; its answer comes from [`Committer::take_done`]. One sent while another is outstanding runs after it.
    pub fn commit(&mut self, req: S::Req, modeset: bool, fence: Option<OwnedFd>) {
        self.outstanding += 1;
        let sent = self.jobs.as_ref().is_some_and(|j| j.send(Job::Commit { req, modeset, _fence: fence }).is_ok());
        if !sent {
            self.undelivered.push(Done { result: Err(libc::ENODEV), took_ns: 0 });
            crate::shell::wake(&self.wake);
        }
    }

    /// A test-only commit, answered now; `None` while a commit is outstanding (the answer would wait for its flip).
    pub fn test(&mut self, req: S::Req) -> Option<Result<(), i32>> {
        if self.busy() {
            return None;
        }
        self.jobs.as_ref()?.send(Job::Test(req)).ok()?;
        match self.replies.recv().ok()? {
            Reply::Tested(r) => Some(r),
            // Every commit's answer was taken before this one (`outstanding` was 0), so the next on the channel is the test's.
            Reply::Committed(_) => unreachable!("a commit answered while none was outstanding"),
        }
    }

    /// The commits answered since the last call, in the order sent. Never waits.
    pub fn take_done(&mut self) -> Vec<Done> {
        // The wake is cleared before the channel is read: an answer that lands in between leaves it set and is read on the next pass.
        crate::shell::drain(&self.wake);
        let mut done = Vec::new();
        loop {
            match self.replies.try_recv() {
                Ok(Reply::Committed(d)) => done.push(d),
                Ok(Reply::Tested(_)) => unreachable!("a test's answer is taken by `test`"),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        // (refused after the stop, so after every commit the thread answered)
        done.append(&mut self.undelivered);
        self.outstanding = self.outstanding.saturating_sub(done.len());
        done
    }

    /// Ends the thread after the jobs already sent, and waits for it: nothing is committed once this returns.
    pub fn stop(&mut self) {
        self.jobs = None;
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl<S: Submit> Drop for Committer<S> {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One descriptor for the loop that is readable while any of `fds` is (a level-triggered epoll of them).
pub fn watch(fds: &[RawFd]) -> io::Result<OwnedFd> {
    // SAFETY: epoll_create1 returns a new descriptor or -1.
    let ep = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if ep < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor is new and ours.
    let ep = unsafe { OwnedFd::from_raw_fd(ep) };
    for &fd in fds {
        let mut ev = libc::epoll_event { events: libc::EPOLLIN as u32, u64: fd as u64 };
        // SAFETY: registering a live descriptor with a stack event.
        if unsafe { libc::epoll_ctl(ep.as_raw_fd(), libc::EPOLL_CTL_ADD, fd, &mut ev) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(ep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread::ThreadId;

    /// Each call: the thread it ran on, that thread's name, which call, the request.
    type Calls = Arc<Mutex<Vec<(ThreadId, Option<String>, &'static str, u32)>>>;

    /// Records each call with the thread it ran on; a commit waits for a permit from the test, so the test decides when it returns.
    struct Fake {
        calls: Calls,
        permits: Receiver<()>,
    }

    impl Fake {
        fn note(&self, what: &'static str, req: u32) {
            let t = std::thread::current();
            self.calls.lock().unwrap().push((t.id(), t.name().map(String::from), what, req));
        }
    }

    impl Submit for Fake {
        type Req = u32;
        fn commit(&mut self, req: u32, _: bool) -> Result<(), i32> {
            let _ = self.permits.recv();
            self.note("commit", req);
            if req == 2 { Err(libc::EINVAL) } else { Ok(()) }
        }
        fn test(&mut self, req: u32) -> Result<(), i32> {
            self.note("test", req);
            Ok(())
        }
    }

    fn start() -> (Committer<Fake>, Calls, Sender<()>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (permit, permits) = channel();
        (Committer::start(Fake { calls: calls.clone(), permits }, 0).unwrap(), calls, permit)
    }

    /// Waits for the wake descriptor, as the loop's epoll does.
    fn readable(fd: RawFd) {
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd on a live descriptor.
        assert_eq!(unsafe { libc::poll(&mut p, 1, 5000) }, 1, "the answer woke the loop");
    }

    fn take(c: &mut Committer<Fake>, n: usize) -> Vec<Result<(), i32>> {
        let mut got = Vec::new();
        while got.len() < n {
            readable(c.wake_fd());
            got.extend(c.take_done().into_iter().map(|d| d.result));
        }
        got
    }

    #[test]
    fn every_call_that_takes_the_display_locks_runs_on_the_commit_thread() {
        let (mut c, calls, permit) = start();
        assert_eq!(c.test(7), Some(Ok(())));
        permit.send(()).unwrap();
        c.commit(1, true, None);
        assert_eq!(take(&mut c, 1), vec![Ok(())]);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let me = std::thread::current().id();
        for (thread, name, what, _) in calls.iter() {
            assert_ne!(*thread, me, "{what} ran on the caller's thread");
            assert_eq!(name.as_deref(), Some(THREAD_NAME));
        }
    }

    #[test]
    fn a_commit_sent_while_one_is_outstanding_is_queued_and_answered_in_order() {
        let (mut c, calls, permit) = start();
        c.commit(1, false, None);
        c.commit(2, false, None);
        c.commit(3, false, None);
        assert!(c.busy());
        assert_eq!(c.test(9), None, "a test while a commit is outstanding is not waited for");
        for _ in 0..3 {
            permit.send(()).unwrap();
        }
        assert_eq!(take(&mut c, 3), vec![Ok(()), Err(libc::EINVAL), Ok(())], "each answer is its own commit's");
        assert!(!c.busy());
        let order: Vec<u32> = calls.lock().unwrap().iter().map(|(_, _, _, r)| *r).collect();
        assert_eq!(order, vec![1, 2, 3]);
    }

    #[test]
    fn stopping_runs_what_was_sent_and_nothing_after() {
        let (mut c, calls, permit) = start();
        permit.send(()).unwrap();
        c.commit(1, false, None);
        c.stop();
        assert_eq!(calls.lock().unwrap().len(), 1, "the commit sent before the stop went out before it returned");
        c.commit(2, false, None);
        assert_eq!(c.test(3), None);
        assert_eq!(take(&mut c, 2), vec![Ok(()), Err(libc::ENODEV)], "a commit after the stop is refused here, not lost");
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}
