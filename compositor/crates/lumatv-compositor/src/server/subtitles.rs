//! Optional, surface-owned subtitle geometry hints.

use lumatv_wl_protocols::lumatv_subtitles_v1::server::{
    lumatv_subtitle_manager_v1::{self, LumatvSubtitleManagerV1},
    lumatv_subtitle_region_v1::{self, LumatvSubtitleRegionV1},
};
use wayland_server::backend::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use super::compositor::surface_id;
use crate::surface::{SubtitleBounds, Surface, SurfaceId};
use crate::{Rect, State, plain_global};

plain_global!(LumatvSubtitleManagerV1);

pub struct SubtitleRegionData {
    surface: SurfaceId,
}

fn bounds(canvas_width: i32, canvas_height: i32, x: i32, y: i32, width: i32, height: i32) -> Option<SubtitleBounds> {
    if !(1..=16384).contains(&canvas_width)
        || !(1..=16384).contains(&canvas_height)
        || x < 0 || y < 0 || width < 0 || height < 0
        || (width == 0) != (height == 0)
        || x.checked_add(width).is_none_or(|right| right > canvas_width)
        || y.checked_add(height).is_none_or(|bottom| bottom > canvas_height)
    {
        return None;
    }
    Some(SubtitleBounds { canvas_width, canvas_height, rect: Rect::new(x, y, width, height) })
}

fn clear_region(surface: &mut Surface) {
    surface.subtitle_region_live = false;
    surface.current.subtitle_bounds = None;
    surface.pending.subtitle_bounds = None;
    if let Some(cached) = &mut surface.cached {
        cached.subtitle_bounds = None;
    }
}

impl Dispatch<LumatvSubtitleManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &LumatvSubtitleManagerV1,
        request: lumatv_subtitle_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            lumatv_subtitle_manager_v1::Request::Destroy => {}
            lumatv_subtitle_manager_v1::Request::GetRegion { id, surface } => {
                if surface.client().is_none_or(|owner| owner.id() != client.id()) {
                    resource.post_error(lumatv_subtitle_manager_v1::Error::InvalidSurface, "surface belongs to another client");
                    return;
                }
                let Some(surface) = surface_id(&surface).and_then(|id| state.surfaces.get_mut(id)) else {
                    resource.post_error(lumatv_subtitle_manager_v1::Error::InvalidSurface, "unknown surface");
                    return;
                };
                if surface.subtitle_region_live {
                    resource.post_error(lumatv_subtitle_manager_v1::Error::DuplicateRegion, "surface already has a subtitle region");
                    return;
                }
                surface.subtitle_region_live = true;
                data_init.init(id, SubtitleRegionData { surface: surface.id });
            }
            _ => {}
        }
    }
}

impl Dispatch<LumatvSubtitleRegionV1, SubtitleRegionData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &LumatvSubtitleRegionV1,
        request: lumatv_subtitle_region_v1::Request,
        data: &SubtitleRegionData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            lumatv_subtitle_region_v1::Request::Destroy => {}
            lumatv_subtitle_region_v1::Request::SetBounds { canvas_width, canvas_height, x, y, width, height } => {
                let Some(bounds) = bounds(canvas_width, canvas_height, x, y, width, height) else {
                    resource.post_error(lumatv_subtitle_region_v1::Error::InvalidBounds, "invalid subtitle bounds");
                    return;
                };
                if let Some(surface) = state.surfaces.get_mut(data.surface) {
                    surface.pending.subtitle_bounds = Some(bounds);
                }
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &LumatvSubtitleRegionV1, data: &SubtitleRegionData) {
        if let Some(surface) = state.surfaces.get_mut(data.surface) {
            clear_region(surface);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{Role, Surfaces};

    #[test]
    fn bounds_are_checked_and_empty_is_known() {
        assert_eq!(bounds(1920, 1080, 0, 0, 0, 0).unwrap().rect, Rect::new(0, 0, 0, 0));
        assert!(bounds(16384, 16384, 16383, 16383, 1, 1).is_some());
        for args in [
            (0, 100, 0, 0, 0, 0), (16385, 100, 0, 0, 1, 1),
            (100, 100, -1, 0, 1, 1), (100, 100, 0, 0, 0, 1),
            (100, 100, 99, 0, 2, 1), (100, 100, i32::MAX, 0, i32::MAX, 1),
        ] {
            assert!(bounds(args.0, args.1, args.2, args.3, args.4, args.5).is_none());
        }
    }

    #[test]
    fn subtitle_bounds_follow_surface_commits_and_cached_subsurfaces() {
        let mut surfaces = Surfaces::default();
        let parent = surfaces.create();
        let child = surfaces.create();
        assert!(surfaces.add_subsurface(child, parent));
        assert!(matches!(surfaces.get(child).unwrap().role, Role::Subsurface { .. }));
        let first = bounds(100, 100, 5, 6, 10, 12).unwrap();
        let second = bounds(100, 100, 0, 0, 0, 0).unwrap();
        surfaces.get_mut(child).unwrap().pending.subtitle_bounds = Some(first);
        assert_eq!(surfaces.get(child).unwrap().current.subtitle_bounds, None);
        surfaces.commit(child, |_| None);
        assert_eq!(surfaces.get(child).unwrap().current.subtitle_bounds, None);
        surfaces.get_mut(child).unwrap().pending.subtitle_bounds = Some(second);
        surfaces.commit(child, |_| None);
        surfaces.commit(parent, |_| None);
        assert_eq!(surfaces.get(child).unwrap().current.subtitle_bounds, Some(second));
    }

    #[test]
    fn destroying_region_clears_pending_cached_and_current() {
        let mut surface = Surface::new(SurfaceId(1));
        let value = bounds(100, 100, 1, 2, 3, 4).unwrap();
        surface.subtitle_region_live = true;
        surface.pending.subtitle_bounds = Some(value);
        surface.cached = Some(Default::default());
        surface.cached.as_mut().unwrap().subtitle_bounds = Some(value);
        surface.current.subtitle_bounds = Some(value);
        clear_region(&mut surface);
        assert!(!surface.subtitle_region_live);
        assert_eq!(surface.pending.subtitle_bounds, None);
        assert_eq!(surface.cached.unwrap().subtitle_bounds, None);
        assert_eq!(surface.current.subtitle_bounds, None);
    }
}
