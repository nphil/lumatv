//! One tuned multiplex, several viewers. The set has one frontend and LG's stack keeps demux channel 0, so the helper has one demux
//! channel and one certified PVR download. Viewers on the same RF channel are not given taps of their own: the download is read once,
//! routed with the union of the viewers' PIDs, and each packet is handed to the viewers whose program it belongs to, in software (a
//! program is a few Mbit/s; the whole multiplex would be 19). A viewer's [`ViewTap`] is a [`Tap`] like any other, so a stream does not
//! know whether it is alone. The tuner is let go when the last [`ViewTap`] is dropped.

use crate::psi::{self, PACKET};
use crate::stream::{Failure, Tap};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

/// The most a slow viewer may have waiting (about 14 s of an HD program): older packets go, so one stalled client costs the others nothing.
const QUEUE_MOST: usize = 8 << 20;

/// How long a viewer waits for a table at a time while others stream: the multiplex is read between the tries, so the others' packets
/// are not left to overflow the download.
const SECTION_STEP: Duration = Duration::from_millis(100);

type Key = (u16, u8, u8);

struct View {
    id: u64,
    pids: Vec<u16>,
    queue: Vec<u8>,
    /// The sections this viewer waits for, and the ones that came for it.
    wants: Vec<Key>,
    got: Vec<(Key, Vec<u8>)>,
}

struct Inner {
    tap: Box<dyn Tap>,
    views: Vec<View>,
    next: u64,
    buf: Vec<u8>,
}

/// The tuned multiplex and the viewers reading it.
pub(crate) struct Hub {
    /// The RF channel it is tuned to, when known.
    pub(crate) rf: Option<u8>,
    inner: Mutex<Inner>,
}

impl Hub {
    pub(crate) fn new(tap: Box<dyn Tap>, rf: Option<u8>) -> Arc<Self> {
        Arc::new(Self {
            rf,
            inner: Mutex::new(Inner {
                tap,
                views: Vec::new(),
                next: 0,
                buf: Vec::new(),
            }),
        })
    }

    /// A viewer's tap on the multiplex.
    pub(crate) fn view(self: &Arc<Self>) -> Box<dyn Tap> {
        let mut g = self.inner.lock();
        let id = g.next;
        g.next += 1;
        g.views.push(View {
            id,
            pids: Vec::new(),
            queue: Vec::new(),
            wants: Vec::new(),
            got: Vec::new(),
        });
        Box::new(ViewTap { hub: self.clone(), id })
    }
}

impl Inner {
    fn at(&mut self, id: u64) -> &mut View {
        self.views.iter_mut().find(|v| v.id == id).expect("a viewer of the hub")
    }

    /// Every PID some viewer reads.
    fn union(&self) -> Vec<u16> {
        let mut all: Vec<u16> = self.views.iter().flat_map(|v| v.pids.iter().copied()).collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Reads the download once and hands each packet to the viewers that route its PID.
    fn pump(&mut self, wait: Duration) -> Result<(), Failure> {
        self.buf.clear();
        let mut buf = std::mem::take(&mut self.buf);
        let read = self.tap.read(&mut buf, wait);
        for p in buf.chunks_exact(PACKET) {
            let pid = psi::pid_of_packet(p);
            for v in self.views.iter_mut().filter(|v| v.pids.contains(&pid)) {
                v.queue.extend_from_slice(p);
            }
        }
        for v in &mut self.views {
            if v.queue.len() > QUEUE_MOST {
                let over = (v.queue.len() - QUEUE_MOST).div_ceil(PACKET) * PACKET;
                v.queue.drain(..over);
            }
        }
        self.buf = buf;
        read
    }
}

/// One viewer's tap: its own queue of its program's packets, and the sections it asked for.
struct ViewTap {
    hub: Arc<Hub>,
    id: u64,
}

impl Tap for ViewTap {
    fn section(&mut self, pid: u16, table_id: u8, number: u8, wait: Duration) -> Option<Vec<u8>> {
        let mut left = wait;
        loop {
            let mut g = self.hub.inner.lock();
            // alone, the wait is the tap's own; with others, in steps that read the download between them
            let step = if g.views.len() > 1 { left.min(SECTION_STEP) } else { left };
            if let Some(section) = g.tap.section(pid, table_id, number, step) {
                return Some(section);
            }
            left = left.saturating_sub(step);
            if left.is_zero() {
                return None;
            }
            drop(g);
            std::thread::yield_now();
        }
    }

    fn start(&mut self, pids: &[u16]) -> Result<(), Failure> {
        let mut g = self.hub.inner.lock();
        let v = g.at(self.id);
        v.pids = pids.to_vec();
        v.queue.clear();
        if g.views.len() == 1 {
            g.tap.start(pids)
        } else {
            let all = g.union();
            g.tap.route(&all)
        }
    }

    fn read(&mut self, out: &mut Vec<u8>, wait: Duration) -> Result<(), Failure> {
        let mut g = self.hub.inner.lock();
        if g.at(self.id).queue.is_empty() {
            g.pump(wait)?;
        }
        let queue = &mut g.at(self.id).queue;
        out.append(queue);
        Ok(())
    }

    fn want(&mut self, pid: u16, table_id: u8, number: u8) {
        let key = (pid, table_id, number);
        let mut g = self.hub.inner.lock();
        let v = g.at(self.id);
        if !v.wants.contains(&key) {
            v.wants.push(key);
        }
        g.tap.want(pid, table_id, number);
    }

    fn arrived(&mut self, pid: u16, table_id: u8, number: u8) -> Option<Vec<u8>> {
        let key = (pid, table_id, number);
        let mut g = self.hub.inner.lock();
        if let Some(section) = g.tap.arrived(pid, table_id, number) {
            // the tap gives a section once: every viewer that waits for it has it
            for v in g.views.iter_mut().filter(|v| v.wants.contains(&key)) {
                v.got.retain(|(k, _)| *k != key);
                v.got.push((key, section.clone()));
            }
        }
        let v = g.at(self.id);
        let at = v.got.iter().position(|(k, _)| *k == key)?;
        v.wants.retain(|k| *k != key);
        Some(v.got.remove(at).1)
    }

    fn forget(&mut self) {
        let mut g = self.hub.inner.lock();
        let v = g.at(self.id);
        v.wants.clear();
        v.got.clear();
        // the tap forgets everyone's: what the others still wait for is asked again
        g.tap.forget();
        let others: Vec<Key> = g.views.iter().flat_map(|v| v.wants.iter().copied()).collect();
        for (pid, table_id, number) in others {
            g.tap.want(pid, table_id, number);
        }
    }

    fn taken(&mut self) -> Option<String> {
        self.hub.inner.lock().tap.taken()
    }
}

impl Drop for ViewTap {
    fn drop(&mut self) {
        let mut g = self.hub.inner.lock();
        g.views.retain(|v| v.id != self.id);
        if !g.views.is_empty() {
            // what only this viewer read is no longer routed; the others' packets are not interrupted
            let all = g.union();
            let _ = g.tap.route(&all);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Seen {
        started: Vec<Vec<u16>>,
        routed: Vec<Vec<u16>>,
        reads: usize,
        wants: Vec<Key>,
        arrive: Vec<(Key, Vec<u8>)>,
        dropped: bool,
    }

    /// A tap that gives one chunk of packets a read and writes down what it was asked.
    struct Stub {
        seen: Arc<Mutex<Seen>>,
        chunks: VecDeque<Vec<u8>>,
    }

    fn packet(pid: u16, count: u8) -> Vec<u8> {
        let mut p = vec![0; PACKET];
        p[..4].copy_from_slice(&[0x47, (pid >> 8) as u8 & 0x1F, pid as u8, 0x10 | count]);
        p
    }

    fn hub(chunks: Vec<Vec<u8>>) -> (Arc<Hub>, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let stub = Stub { seen: seen.clone(), chunks: chunks.into() };
        (Hub::new(Box::new(stub), Some(32)), seen)
    }

    impl Tap for Stub {
        fn section(&mut self, _: u16, _: u8, _: u8, _: Duration) -> Option<Vec<u8>> {
            None
        }

        fn start(&mut self, pids: &[u16]) -> Result<(), Failure> {
            self.seen.lock().started.push(pids.to_vec());
            Ok(())
        }

        fn route(&mut self, pids: &[u16]) -> Result<(), Failure> {
            self.seen.lock().routed.push(pids.to_vec());
            Ok(())
        }

        fn read(&mut self, out: &mut Vec<u8>, _: Duration) -> Result<(), Failure> {
            self.seen.lock().reads += 1;
            out.extend(self.chunks.pop_front().into_iter().flatten());
            Ok(())
        }

        fn want(&mut self, pid: u16, table_id: u8, number: u8) {
            let mut seen = self.seen.lock();
            if !seen.wants.contains(&(pid, table_id, number)) {
                seen.wants.push((pid, table_id, number));
            }
        }

        fn arrived(&mut self, pid: u16, table_id: u8, number: u8) -> Option<Vec<u8>> {
            let mut seen = self.seen.lock();
            let key = (pid, table_id, number);
            let at = seen.arrive.iter().position(|(k, _)| *k == key)?;
            seen.wants.retain(|k| *k != key);
            Some(seen.arrive.remove(at).1)
        }

        fn forget(&mut self) {
            self.seen.lock().wants.clear();
        }

        fn taken(&mut self) -> Option<String> {
            None
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.seen.lock().dropped = true;
        }
    }

    fn pids(packets: &[u8]) -> Vec<u16> {
        packets.chunks_exact(PACKET).map(psi::pid_of_packet).collect()
    }

    #[test]
    fn two_viewers_read_the_multiplex_once_and_each_gets_its_own_program_without_the_other_being_routed_again() {
        let mut both = packet(0x31, 0);
        both.extend(packet(0x41, 0));
        both.extend(packet(0, 0));
        let (hub, seen) = hub(vec![both]);
        let (mut a, mut b) = (hub.view(), hub.view());
        a.start(&[0, 0x31]).unwrap();
        b.start(&[0, 0x41]).unwrap();
        let (mut got_a, mut got_b) = (Vec::new(), Vec::new());
        a.read(&mut got_a, Duration::ZERO).unwrap();
        b.read(&mut got_b, Duration::ZERO).unwrap();
        assert_eq!((pids(&got_a), pids(&got_b)), (vec![0x31, 0], vec![0x41, 0]));
        let seen = seen.lock();
        assert_eq!(seen.reads, 1, "one read of the download for both");
        assert_eq!(seen.routed, [vec![0, 0x31], vec![0, 0x31, 0x41]], "the second routing adds the second program's PIDs to the first's");
    }

    #[test]
    fn a_viewer_that_leaves_stops_its_pids_and_the_others_go_on_until_the_last_lets_go_of_the_tuner() {
        let (hub, seen) = hub(vec![packet(0x31, 0), packet(0x41, 1)]);
        let (mut a, mut b) = (hub.view(), hub.view());
        drop(hub);
        a.start(&[0x31]).unwrap();
        b.start(&[0x41]).unwrap();
        drop(a);
        assert_eq!(seen.lock().routed.last(), Some(&vec![0x41]), "what only the viewer that left read is no longer routed");
        let mut got = Vec::new();
        for _ in 0..2 {
            b.read(&mut got, Duration::ZERO).unwrap();
        }
        assert_eq!(pids(&got), [0x41]);
        assert!(!seen.lock().dropped, "the tuner is the other viewer's still");
        drop(b);
        assert!(seen.lock().dropped);
    }

    #[test]
    fn a_section_two_viewers_wait_for_comes_to_both_and_one_forgetting_does_not_forget_the_other() {
        let (hub, seen) = hub(vec![]);
        let (mut a, mut b) = (hub.view(), hub.view());
        a.want(0, 0, 0);
        b.want(0, 0, 0);
        b.want(0x1FFB, 0xC8, 0);
        seen.lock().arrive.push(((0, 0, 0), vec![1, 2, 3]));
        assert_eq!(a.arrived(0, 0, 0), Some(vec![1, 2, 3]));
        assert_eq!(b.arrived(0, 0, 0), Some(vec![1, 2, 3]), "the tap gives a section once; the other viewer still has it");
        assert_eq!(a.arrived(0, 0, 0), None);
        a.want(0x1FFB, 0xC8, 1);
        a.forget();
        assert_eq!(seen.lock().wants, [(0x1FFB, 0xC8, 0)], "the other viewer's wait stands");
    }

    #[test]
    fn a_viewer_that_does_not_read_keeps_no_more_than_its_queue_holds_and_the_newest_of_it() {
        let chunk: Vec<u8> = (0..(QUEUE_MOST / PACKET + 10)).flat_map(|n| packet(0x31, n as u8)).collect();
        let (hub, _) = hub(vec![chunk]);
        let (mut slow, mut quick) = (hub.view(), hub.view());
        slow.start(&[0x31]).unwrap();
        quick.start(&[0x41]).unwrap();
        let mut got = Vec::new();
        quick.read(&mut got, Duration::ZERO).unwrap();
        slow.read(&mut got, Duration::ZERO).unwrap();
        assert!(got.len() <= QUEUE_MOST && got.len() > QUEUE_MOST - PACKET, "no more than the queue holds, the newest");
    }
}
