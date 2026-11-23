//! The child process behind two traits: the engine runs against `lumatv-venc xcode` on the TV and against scripted children in tests.

use crate::proto::Cfg;
use crate::types::JobId;
use std::io::{self, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};

/// The two ways the engine stops a child: asking, then not asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sig {
    Term,
    Kill,
}

/// How a child ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// What a child is started for: the job, and the configuration the engine sends it first on stdin.
#[derive(Clone, Debug)]
pub struct ChildCfg {
    pub job: JobId,
    pub cfg: Cfg,
}

pub trait Spawn: Send + Sync {
    /// Starts `child xcode` with all three standard streams piped. A real child reads its configuration from stdin; `cfg` is for scripted ones.
    fn spawn(&self, child: &Path, cfg: &ChildCfg) -> io::Result<Box<dyn Proc>>;
}

/// A running child. The engine takes each stream once, on its own thread, and never waits on the process: it asks `try_wait`.
pub trait Proc: Send {
    fn pid(&self) -> u32;
    fn stdin(&mut self) -> Option<Box<dyn Write + Send>>;
    fn stdout(&mut self) -> Option<Box<dyn Read + Send>>;
    fn stderr(&mut self) -> Option<Box<dyn Read + Send>>;
    /// Best effort: a child that is already gone is not an error.
    fn signal(&mut self, sig: Sig);
    /// `Some` once, when the child has ended and been reaped.
    fn try_wait(&mut self) -> Option<Exit>;
}

/// Real processes.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsSpawn;

impl Spawn for OsSpawn {
    fn spawn(&self, child: &Path, _cfg: &ChildCfg) -> io::Result<Box<dyn Proc>> {
        let mut command = Command::new(child);
        command.arg("xcode").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // SAFETY: the closure runs between fork and exec and only calls setrlimit, which is async-signal-safe. A crash of the child must not leave a
        // multi-hundred-megabyte core dump on the TV's small flash.
        unsafe {
            command.pre_exec(|| {
                let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
                if libc::setrlimit(libc::RLIMIT_CORE, &none) == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
            });
        }
        Ok(Box::new(OsProc(command.spawn()?)))
    }
}

struct OsProc(Child);

fn exit_of(status: ExitStatus) -> Exit {
    match (status.code(), status.signal()) {
        (Some(code), _) => Exit::Code(code),
        (None, Some(sig)) => Exit::Signal(sig),
        (None, None) => Exit::Code(-1),
    }
}

impl Proc for OsProc {
    fn pid(&self) -> u32 {
        self.0.id()
    }

    fn stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.0.stdin.take().map(|s| Box::new(s) as Box<dyn Write + Send>)
    }

    fn stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.0.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>)
    }

    fn stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.0.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)
    }

    fn signal(&mut self, sig: Sig) {
        match sig {
            Sig::Term => {
                // SAFETY: the child has not been reaped (try_wait has not returned its status), so the pid still names it and cannot have been reused.
                // A pid that does not fit in pid_t is never signalled: kill(0) would signal the helper's whole process group.
                if let Ok(pid) = libc::pid_t::try_from(self.0.id()) {
                    unsafe { libc::kill(pid, libc::SIGTERM) };
                }
            }
            Sig::Kill => {
                let _ = self.0.kill();
            }
        }
    }

    fn try_wait(&mut self) -> Option<Exit> {
        match self.0.try_wait() {
            Ok(status) => status.map(exit_of),
            Err(e) => {
                // ECHILD: something else reaped it (SIGCHLD ignored); it is gone either way, only its status is lost
                log::warn!("lumaxcode: waiting for pid {} failed: {e}", self.0.id());
                Some(Exit::Code(-1))
            }
        }
    }
}
