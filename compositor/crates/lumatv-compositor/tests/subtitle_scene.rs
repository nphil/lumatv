mod harness;

use harness::{Client, Server};
use lumatv_wl_protocols::lumatv_subtitles_v1::client::lumatv_subtitle_manager_v1::LumatvSubtitleManagerV1;
use lumatv_compositor::{Rect, output::{Image, Output, Presented, RenderStats}, scene::Plan, surface::Surfaces};
use std::{io, sync::{Arc, Mutex}, sync::atomic::Ordering};
use wayland_server::protocol::wl_buffer::WlBuffer;

struct ExclusionOutput(Arc<Mutex<Vec<Option<Rect>>>>);

impl Output for ExclusionOutput {
    fn name(&self) -> &'static str { "exclusion-test" }
    fn size(&self) -> (u32, u32) { (1920, 1080) }
    fn query_buffer(&mut self, _: &WlBuffer) -> Option<((i32, i32), bool)> { None }
    fn forget_buffer(&mut self, _: &WlBuffer) {}
    fn ui_set_exclusion(&mut self, area: Option<Rect>) { self.0.lock().unwrap().push(area); }
    fn ui_needs_frame(&mut self) -> io::Result<bool> { Ok(true) }
    fn render(&mut self, _: &Plan, _: &Surfaces) -> io::Result<RenderStats> { Ok(RenderStats::default()) }
    fn present(&mut self) -> io::Result<Presented> { Ok(Presented::default()) }
    fn capture(&mut self) -> io::Result<Image> { Err(io::Error::other("nothing drawn")) }
}

#[test]
fn only_the_mapped_foreground_surface_contributes() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let manager: LumatvSubtitleManagerV1 = client.globals.bind(&client.qh, 1..=1, ()).unwrap();
    let first = client.card("_WEBOS_WINDOW_TYPE_CARD", "first");
    let region = manager.get_region(&first.surface, &client.qh, ());
    let buffer = client.shm_buffer(100, 100, 0xff000000);
    region.set_bounds(100, 100, 10, 20, 30, 40);
    client.present(&first.surface, &buffer);
    client.roundtrip();
    let report = |server: &Server| server.eval(|s| {
        s.scene.plan(&s.surfaces);
        s.scene.subtitle_bounds(&s.surfaces).map(|(app, surface, rect)| (app.to_owned(), surface, rect))
    });
    let original = report(&server).expect("mapped first card reports bounds");
    assert_eq!(original.0, "org.test.first");
    assert!(original.2.is_some());

    let child = client.compositor.create_surface(&client.qh, ());
    let sub = client.subcompositor.get_subsurface(&child, &first.surface, &client.qh, ());
    let child_region = manager.get_region(&child, &client.qh, ());
    child_region.set_bounds(100, 100, 10, 10, 20, 20);
    client.present(&child, &buffer);
    first.surface.commit();
    client.roundtrip();
    assert_eq!(report(&server), None, "multiple visible reports have no authoritative single surface");
    child_region.destroy();
    sub.destroy();
    child.destroy();
    first.surface.commit();
    client.roundtrip();
    assert_eq!(report(&server), Some(original.clone()));

    let second = client.card("_WEBOS_WINDOW_TYPE_CARD", "second");
    client.present(&second.surface, &buffer);
    client.roundtrip();
    assert_eq!(report(&server), None, "a background hint must not leak into the foreground");
    let second_region = manager.get_region(&second.surface, &client.qh, ());
    second_region.set_bounds(100, 100, 0, 0, 0, 0);
    second.surface.commit();
    client.roundtrip();
    let empty = report(&server).expect("explicit empty is known");
    assert_eq!(empty.0, "org.test.second");
    assert_ne!(empty.1, original.1);
    assert_eq!(empty.2, None);
    second.surface.set_buffer_scale(2);
    second.surface.commit();
    client.roundtrip();
    assert_eq!(report(&server), None, "unsupported buffer scale is unknown");
    second.surface.set_buffer_scale(1);
    second.surface.commit();
    client.roundtrip();
    assert_eq!(report(&server), Some(empty));
    second.surface.attach(None, 0, 0);
    second.surface.commit();
    client.roundtrip();
    assert_eq!(report(&server), Some(original));
    region.destroy();
    client.roundtrip();
    assert_eq!(report(&server), None);
}

#[test]
fn current_plan_sets_the_exclusion_before_ui_and_clears_it() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let output_calls = calls.clone();
    let mut server = Server::start_with(Box::new(move || Box::new(ExclusionOutput(output_calls))));
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let manager: LumatvSubtitleManagerV1 = client.globals.bind(&client.qh, 1..=1, ()).unwrap();
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "player");
    let region = manager.get_region(&card.surface, &client.qh, ());
    let buffer = client.shm_buffer(100, 100, 0xff000000);
    client.present(&card.surface, &buffer);
    client.roundtrip();
    server.eval(|s| { s.tick(); });
    assert_eq!(calls.lock().unwrap().last().copied(), Some(Some(Rect::new(0, 905, 1920, 105))));

    region.set_bounds(100, 100, 10, 20, 30, 40);
    card.surface.commit();
    client.roundtrip();
    let expected = server.eval(|s| {
        s.tick();
        s.scene.subtitle_bounds(&s.surfaces).unwrap().2.unwrap()
    });
    assert_eq!(calls.lock().unwrap().last().copied(), Some(Some(expected)));

    region.set_bounds(100, 100, 0, 0, 0, 0);
    card.surface.commit();
    client.roundtrip();
    server.eval(|s| { s.tick(); });
    assert_eq!(calls.lock().unwrap().last().copied(), Some(None));

    card.surface.attach(None, 0, 0);
    card.surface.commit();
    client.roundtrip();
    server.eval(|s| { s.tick(); });
    assert_eq!(calls.lock().unwrap().last().copied(), Some(None));
}
