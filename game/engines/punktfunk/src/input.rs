use lumagame_core::{ids::SessionId, ipc::{Event, FromEngine, PadKey, PadStats}};
use lumatv_gfn_pad::{PadEvent, PadSink, PadState, RawReport, lab::Timing, latency, menu::{MenuKeys, MenuKey}};
use punktfunk_core::{client::NativeClient, config::GamepadPref, input::{GamepadSnapshot, InputEvent, InputKind, encode_gamepad_remove, gamepad}, quic::{HidOutput, HOST_CAP_GAMEPAD_STATE}};
use std::{sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc::Sender}, time::Instant};
use crate::{Notice, dualsense::DualSense};

#[derive(Default)]
struct Slot { seq:u8, state:PadState, menu:MenuKeys, usb:Option<DualSense>, connected:bool, name:String, timing:Timing, last_ns:u64 }
struct State { app:bool, slots:[Slot;4] }
pub struct Pads {
    client: Arc<NativeClient>,
    session: SessionId,
    out: Sender<Notice>,
    state: Mutex<State>,
    pub failed: AtomicBool,
    sampling: AtomicBool,
    presses: Mutex<Option<(u32,u32,u32)>>,
}
impl Pads {
    pub fn new(client: Arc<NativeClient>, session: SessionId, out: Sender<Notice>) -> Self {
        Self {client,session,out,state:Mutex::new(State {app:false,slots:std::array::from_fn(|_| Slot::default())}),failed:AtomicBool::new(false),presses:Mutex::new(None),sampling:AtomicBool::new(false)}
    }
    /// Presses timed from the kernel's stamp to `send_input` returning, over the pad crate's window: (count, median µs, p99 µs).
    pub fn presses(&self) -> Option<(u32,u32,u32)> { *self.presses.lock().unwrap() }
    pub fn sample(&self, on:bool) {
        if self.sampling.swap(on,Ordering::AcqRel) && !on {
            for pad in &mut self.state.lock().unwrap().slots {pad.timing=Timing::default();pad.last_ns=0;}
        }
    }
    pub fn timing(&self, now_ns:u64) -> Option<PadStats> {
        if !self.sampling.load(Ordering::Acquire) {return None;}
        let state = self.state.lock().unwrap();
        let pad = state.slots.iter().filter(|p| p.connected && p.last_ns>0).max_by_key(|p| p.last_ns)?;
        let s = pad.timing.summary(now_ns);
        Some(PadStats { name:pad.name.clone(), reports:s.reports, rate_hz:s.rate_hz, p99_us:s.p99_us,
            bursts:s.bursts.count, burst_gap_us:s.bursts.gap_us, age_ms:now_ns.saturating_sub(pad.last_ns)/1_000_000 })
    }
    fn send(&self, event: InputEvent) { if self.client.send_input(&event).is_err() { self.failed.store(true,Ordering::Relaxed); } }
    pub fn route(&self, app:bool) {
        let mut state = self.state.lock().unwrap();
        if state.app == app { return; }
        state.app = app;
        for (i,s) in state.slots.iter_mut().enumerate().filter(|(_,s)| s.connected) {
            let next = if app {PadState::default()} else {s.state};
            transmit(i as u8,s.seq,if app {s.state} else {PadState::default()},next,self.client.host_caps & HOST_CAP_GAMEPAD_STATE != 0, |e|self.send(e));
            s.seq=s.seq.wrapping_add(1); s.menu=MenuKeys::default();
            if app && s.usb.as_mut().is_some_and(|usb|usb.release().is_err()) {s.usb=None;}
        }
    }
    pub fn feedback(&self, output:HidOutput) {
        let pad = match &output { HidOutput::Led {pad,..}|HidOutput::PlayerLeds {pad,..}|HidOutput::Trigger {pad,..}|HidOutput::MicLed {pad,..}=>*pad, _=>return };
        let mut state = self.state.lock().unwrap();
        if state.app { return; }
        if let Some(slot) = state.slots.get_mut(usize::from(pad))
            && slot.usb.as_mut().is_some_and(|usb| usb.apply(&output).is_err()) { slot.usb = None; }
    }
}
impl PadSink for Pads {
    fn pad_event(&self, _bitmap:u16, event:PadEvent) {
        let mut state = self.state.lock().unwrap();
        let app = state.app;
        match event {
            PadEvent::Connected {slot,uniq,name} => {
                let Some(s) = state.slots.get_mut(usize::from(slot)) else {return};
                s.connected=true; s.usb=DualSense::find(&uniq); s.name=name;
                let kind = if s.usb.is_some() { GamepadPref::DualSense } else { GamepadPref::Xbox360 };
                self.send(InputEvent {kind:InputKind::GamepadArrival,_pad:[0;3],code:u32::from(kind.to_u8()),x:0,y:0,flags:u32::from(slot)});
            }
            PadEvent::Disconnected {slot} => {
                let Some(s) = state.slots.get_mut(usize::from(slot)) else {return};
                transmit(slot,s.seq,s.state,PadState::default(),self.client.host_caps & HOST_CAP_GAMEPAD_STATE != 0, |e| self.send(e));
                s.seq=s.seq.wrapping_add(1);
                self.send(InputEvent {kind:InputKind::GamepadRemove,_pad:[0;3],code:0,x:0,y:0,flags:encode_gamepad_remove(slot,s.seq)});
                *s=Slot::default();
            }
            PadEvent::Report {slot,state:next,press} => {
                let Some(s) = state.slots.get_mut(usize::from(slot)) else {return};
                if app {
                    s.menu.feed(&next,|key| {
                        let key = match key {MenuKey::Up=>PadKey::Up,MenuKey::Down=>PadKey::Down,MenuKey::Left=>PadKey::Left,MenuKey::Right=>PadKey::Right,MenuKey::Ok=>PadKey::Ok,MenuKey::Back=>PadKey::Back,MenuKey::Menu=>PadKey::Menu,MenuKey::Alt=>PadKey::Alt,MenuKey::PageLeft=>PadKey::PageLeft,MenuKey::PageRight=>PadKey::PageRight};
                        for down in [true,false] { let _ = self.out.send(Notice::Emit(FromEngine::told(Event::Key {key,down}))); }
                    });
                } else {
                    transmit(slot,s.seq,s.state,next,self.client.host_caps & HOST_CAP_GAMEPAD_STATE != 0, |e|self.send(e));
                    s.seq=s.seq.wrapping_add(1);
                    // The input is on its way once send_input returns: that is this transport's handoff.
                    if let Some(id)=press { let at=Instant::now(); latency::global().link(id,at); latency::global().handoff(at,latency::mono_ns()); }
                }
                s.state=next;
            }
            PadEvent::Guide {pressed:true,..}|PadEvent::ExitChord {..} => { let _ = self.out.send(Notice::Emit(FromEngine::told(Event::Overlay {session:self.session}))); }
            _=>{}
        }
    }
    fn latency(&self, summary:&latency::Summary) {
        *self.presses.lock().unwrap()=Some((u32::try_from(summary.samples).unwrap_or(u32::MAX),summary.handoff.p50,summary.handoff.p99));
    }
    fn raw(&self, slot:u8, report:&RawReport) {
        if !self.sampling.load(Ordering::Acquire) {return;}
        if let Some(pad) = self.state.lock().unwrap().slots.get_mut(usize::from(slot)) {
            pad.timing.push(report.kernel_ns); pad.last_ns=report.kernel_ns;
        }
    }
}
fn transmit(slot:u8,seq:u8,old:PadState,next:PadState,snapshot:bool,mut send:impl FnMut(InputEvent)) {
    if snapshot {
        send(GamepadSnapshot {pad:slot,seq,buttons:u32::from(next.buttons),left_trigger:next.left_trigger,right_trigger:next.right_trigger,
            ls_x:next.left_x,ls_y:next.left_y,rs_x:next.right_x,rs_y:next.right_y}.to_event());
        return;
    }
    for bit in 0..16 { let mask=1u16<<bit; if (old.buttons ^ next.buttons) & mask != 0 {
        send(InputEvent {kind:InputKind::GamepadButton,_pad:[0;3],code:u32::from(mask),x:i32::from(next.buttons & mask != 0),y:0,flags:u32::from(slot)});
    }}
    for (axis,a,b) in [(gamepad::AXIS_LS_X,i32::from(old.left_x),i32::from(next.left_x)),(gamepad::AXIS_LS_Y,i32::from(old.left_y),i32::from(next.left_y)),
        (gamepad::AXIS_RS_X,i32::from(old.right_x),i32::from(next.right_x)),(gamepad::AXIS_RS_Y,i32::from(old.right_y),i32::from(next.right_y)),
        (gamepad::AXIS_LT,i32::from(old.left_trigger),i32::from(next.left_trigger)),(gamepad::AXIS_RT,i32::from(old.right_trigger),i32::from(next.right_trigger))] {
        if a!=b {send(InputEvent {kind:InputKind::GamepadAxis,_pad:[0;3],code:axis,x:b,y:0,flags:u32::from(slot)});}
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn routing_to_overlay_releases_all_held_controls_on_both_host_protocols() {
        let held=PadState {buttons:0x1001,left_trigger:200,left_y:30000,..Default::default()};
        let mut events=Vec::new();
        transmit(2,7,held,PadState::default(),true,|e|events.push(e));
        let s=GamepadSnapshot::from_event(&events[0]).unwrap();
        assert_eq!((s.pad,s.seq,s.buttons,s.left_trigger,s.ls_y),(2,7,0,0,0));
        events.clear();
        transmit(2,7,held,PadState::default(),false,|e|events.push(e));
        assert_eq!(events.len(),4);
        assert!(events.iter().all(|e|e.x==0 && e.flags==2));
    }
}
