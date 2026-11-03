//! Subtitle hints over a real Wayland connection.
mod harness;

use harness::{Client, Server};
use lumatv_compositor::Rect;
use lumatv_compositor::surface::SubtitleBounds;
use lumatv_wl_protocols::lumatv_subtitles_v1::client::lumatv_subtitle_manager_v1::LumatvSubtitleManagerV1;

fn manager(client: &Client) -> LumatvSubtitleManagerV1 {
    client.globals.bind(&client.qh, 1..=1, ()).unwrap()
}

#[test]
fn hint_changes_only_on_surface_commit_and_is_cleared_on_unmap_or_destroy() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let surface = client.compositor.create_surface(&client.qh, ());
    let manager = manager(&client);
    let region = manager.get_region(&surface, &client.qh, ());
    let buffer = client.shm_buffer(64, 64, 0xff000000);
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    client.roundtrip();
    let current = |server: &Server| server.eval(|s| s.surfaces.iter().next().unwrap().current.subtitle_bounds);
    assert_eq!(current(&server), None);
    region.set_bounds(1920, 1080, 100, 900, 800, 100);
    client.roundtrip();
    assert_eq!(current(&server), None);
    surface.commit();
    client.roundtrip();
    assert_eq!(current(&server), Some(SubtitleBounds { canvas_width: 1920, canvas_height: 1080, rect: Rect::new(100, 900, 800, 100) }));
    region.set_bounds(1920, 1080, 0, 0, 0, 0);
    surface.commit();
    client.roundtrip();
    assert_eq!(current(&server).unwrap().rect, Rect::new(0, 0, 0, 0));
    surface.attach(None, 0, 0);
    surface.commit();
    client.roundtrip();
    assert_eq!(current(&server), None);
    region.set_bounds(1920, 1080, 10, 10, 20, 20);
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    client.roundtrip();
    region.destroy();
    client.roundtrip();
    assert_eq!(current(&server), None);
    let _replacement = manager.get_region(&surface, &client.qh, ());
    client.roundtrip();
}

#[test]
fn duplicate_region_is_a_protocol_error() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let surface = client.compositor.create_surface(&client.qh, ());
    let manager = manager(&client);
    let _first = manager.get_region(&surface, &client.qh, ());
    let _second = manager.get_region(&surface, &client.qh, ());
    assert!(client.queue.roundtrip(&mut client.state).is_err());
}

#[test]
fn invalid_bounds_are_a_protocol_error() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let surface = client.compositor.create_surface(&client.qh, ());
    let manager = manager(&client);
    let region = manager.get_region(&surface, &client.qh, ());
    region.set_bounds(100, 100, 99, 0, 2, 1);
    assert!(client.queue.roundtrip(&mut client.state).is_err());
}
