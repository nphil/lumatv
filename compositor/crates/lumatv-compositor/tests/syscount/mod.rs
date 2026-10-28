//! A test hook that counts the system calls which take the address space's lock (LPC 2026: `mmap_sem` is what a late frame waits
//! behind): `mmap`, `munmap`, `mprotect`, `mremap` and `brk`, made by one watched thread. A seccomp filter traps those five
//! with SIGSYS for every thread that is made after it is installed (and the installing one); the handler counts the call if the
//! thread is the watched one and then makes it itself, from a stub whose address the filter lets through, so the program runs as
//! it would without the hook, only slower. x86_64 Linux: elsewhere, or where the container refuses seccomp, `install` says so and
//! the test skips.

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

const NRS: [i64; 5] = [libc::SYS_mmap, libc::SYS_munmap, libc::SYS_mprotect, libc::SYS_mremap, libc::SYS_brk];
const NAMES: [&str; 5] = ["mmap", "munmap", "mprotect", "mremap", "brk"];

static COUNTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static WATCHED: AtomicI64 = AtomicI64::new(0);

/// What the watched thread asked the kernel for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts(pub [u64; 5]);

impl Counts {
    pub fn total(&self) -> u64 {
        self.0.iter().sum()
    }
}

impl std::fmt::Display for Counts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = NAMES.iter().zip(self.0).filter(|(_, n)| *n > 0).map(|(name, n)| format!("{name} x{n}")).collect();
        if parts.is_empty() { write!(f, "no mmap, munmap, mprotect, mremap or brk") } else { write!(f, "{}", parts.join(", ")) }
    }
}

/// Counts from now on for the thread `tid` (0: nobody), from zero.
pub fn watch(tid: i64) {
    for c in &COUNTS {
        c.store(0, Ordering::SeqCst);
    }
    WATCHED.store(tid, Ordering::SeqCst);
}

/// The calling thread's id.
pub fn gettid() -> i64 {
    // SAFETY: gettid takes nothing.
    unsafe { libc::syscall(libc::SYS_gettid) }
}

/// Stops counting and returns what was counted.
pub fn stop() -> Counts {
    WATCHED.store(0, Ordering::SeqCst);
    Counts(std::array::from_fn(|i| COUNTS[i].load(Ordering::SeqCst)))
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
mod x86 {
    use super::*;

    core::arch::global_asm!(
        ".globl lumatv_raw_syscall",
        "lumatv_raw_syscall:",
        "mov rax, rdi",
        "mov rdi, rsi",
        "mov rsi, rdx",
        "mov rdx, rcx",
        "mov r10, r8",
        "mov r8, r9",
        "mov r9, [rsp+8]",
        "syscall",
        ".globl lumatv_raw_syscall_end",
        "lumatv_raw_syscall_end:",
        "ret",
    );

    unsafe extern "C" {
        fn lumatv_raw_syscall(nr: i64, a0: i64, a1: i64, a2: i64, a3: i64, a4: i64, a5: i64) -> i64;
        fn lumatv_raw_syscall_end();
    }

    extern "C" fn on_sigsys(_sig: libc::c_int, _info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        // SAFETY: the kernel passes the interrupted thread's context; the registers are rewritten as the call's result.
        unsafe {
            let g = &mut (*(ctx as *mut libc::ucontext_t)).uc_mcontext.gregs;
            let nr = g[libc::REG_RAX as usize];
            let watched = WATCHED.load(Ordering::Relaxed);
            if watched != 0
                && libc::syscall(libc::SYS_gettid) == watched
                && let Some(i) = NRS.iter().position(|&n| n == nr)
            {
                COUNTS[i].fetch_add(1, Ordering::Relaxed);
            }
            g[libc::REG_RAX as usize] = lumatv_raw_syscall(
                nr,
                g[libc::REG_RDI as usize],
                g[libc::REG_RSI as usize],
                g[libc::REG_RDX as usize],
                g[libc::REG_R10 as usize],
                g[libc::REG_R8 as usize],
                g[libc::REG_R9 as usize],
            );
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Insn {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct Prog {
        len: u16,
        filter: *const Insn,
    }

    pub fn install() -> Result<(), String> {
        const LD: u16 = 0x20;
        const JEQ: u16 = 0x15;
        const RET: u16 = 0x06;
        const ALLOW: u32 = 0x7fff_0000;
        const TRAP: u32 = 0x0003_0000;
        let ip = lumatv_raw_syscall_end as *const () as usize as u64;
        // 0 arch, 1 x86_64 or allow, 2 nr, 3-7 one of ours: to 9, 8 allow, 9-12 the call came from the stub: allow, 13 trap, 14 allow.
        let mut f = vec![Insn { code: LD, jt: 0, jf: 0, k: 4 }, Insn { code: JEQ, jt: 0, jf: 12, k: 0xc000_003e }, Insn { code: LD, jt: 0, jf: 0, k: 0 }];
        for (i, nr) in NRS.iter().enumerate() {
            f.push(Insn { code: JEQ, jt: (5 - i) as u8, jf: 0, k: *nr as u32 });
        }
        f.extend([
            Insn { code: RET, jt: 0, jf: 0, k: ALLOW },
            Insn { code: LD, jt: 0, jf: 0, k: 8 },
            Insn { code: JEQ, jt: 0, jf: 2, k: (ip & 0xffff_ffff) as u32 },
            Insn { code: LD, jt: 0, jf: 0, k: 12 },
            Insn { code: JEQ, jt: 1, jf: 0, k: (ip >> 32) as u32 },
            Insn { code: RET, jt: 0, jf: 0, k: TRAP },
            Insn { code: RET, jt: 0, jf: 0, k: ALLOW },
        ]);
        // SAFETY: a stack sigaction and filter program handed to the kernel; the handler is `extern "C"` and lives for the process.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_sigsys as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;
            libc::sigemptyset(&mut sa.sa_mask);
            if libc::sigaction(libc::SIGSYS, &sa, std::ptr::null_mut()) != 0 {
                return Err(format!("sigaction: {}", std::io::Error::last_os_error()));
            }
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
            }
            let prog = Prog { len: f.len() as u16, filter: f.as_ptr() };
            if libc::syscall(libc::SYS_seccomp, 1, 0, &prog as *const Prog) != 0 {
                return Err(format!("seccomp: {}", std::io::Error::last_os_error()));
            }
        }
        Ok(())
    }
}

/// Installs the filter on the calling thread and every thread it makes from now on. `Err` says why it cannot be (not x86_64 Linux,
/// or the container's own filter refuses another).
pub fn install() -> Result<(), String> {
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        x86::install()
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
    {
        Err("the hook is x86_64 Linux's".to_string())
    }
}
