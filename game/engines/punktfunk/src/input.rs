use lumagame_core::{ids::SessionId, ipc::{Event, FromEngine, PadKey}};
use lumatv_gfn_pad::{PadEvent, PadSink, PadState, menu::{MenuKeys, MenuKey}};
use punktfunk_core::{client::NativeClient, config::GamepadPref, input::{GamepadSnapshot, InputEvent, InputKind, encode_gamepad_remove, gamepad}, quic::{HidOutput, HOST_CAP_GAMEPAD_STATE}};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc::Sender};
use crate::{Notice, dualsense::DualSense};

#[derive(Default)]
struct Slot { seq:u8, state:PadState, menu:MenuKeys, usb:Option<DualSense>, connected:bool }
struct State { app:bool, slots:[Slot;4] }
pub struct Pads {
    client: Arc<NativeClient>,
    session: SessionId,
    out: Sender<Notice>,
    state: Mutex<State>,
    pub failed: AtomicBool,
}
impl Pads {
    pub fn new(client: Arc<NativeClient>, session: SessionId, out: Sender<Notice>) -> Self {
        Self {client,session,out,state:Mutex::new(State {app:false,slots:std::array::from_fn(|_| Slot::default())}),failed:AtomicBool::new(false)}
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
            PadEvent::Connected {slot,uniq,..} => {
                let Some(s) = state.slots.get_mut(usize::from(slot)) else {return};
                s.connected=true; s.usb=DualSense::find(&uniq);
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
            PadEvent::Report {slot,state:next,..} => {
                let Some(s) = state.slots.get_mut(usize::from(slot)) else {return};
                if app {
                    s.menu.feed(&next,|key| {
                        let key = match key {MenuKey::Up=>PadKey::Up,MenuKey::Down=>PadKey::Down,MenuKey::Left=>PadKey::Left,MenuKey::Right=>PadKey::Right,MenuKey::Ok=>PadKey::Ok,MenuKey::Back=>PadKey::Back};
                        for down in [true,false] { let _ = self.out.send(Notice::Emit(FromEngine::told(Event::Key {key,down}))); }
                    });
                } else {
                    transmit(slot,s.seq,s.state,next,self.client.host_caps & HOST_CAP_GAMEPAD_STATE != 0, |e|self.send(e));
                    s.seq=s.seq.wrapping_add(1);
                }
                s.state=next;
            }
            PadEvent::Guide {pressed:true,..}|PadEvent::ExitChord {..} => { let _ = self.out.send(Notice::Emit(FromEngine::told(Event::Overlay {session:self.session}))); }
            _=>{}
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
