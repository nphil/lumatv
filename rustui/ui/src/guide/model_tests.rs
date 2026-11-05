//! The guide model's behaviour: what it asks for and when, how the keys move the focus and the view, how a programme is picked, and that a
//! list of hundreds of channels and a day of listings cost a few calls and a few steps, not a rebuild.

use super::model::*;
use crate::host::channels as ch;
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

/// 2026-11-04 04:20 UTC, 23:20 on the evening of the 3rd in the zone the tests give the TV (UTC minus five hours).
fn t(stamp: &str) -> i64 {
    ch::parse_stamp(stamp, 0).unwrap()
}
const OFFSET: i32 = -5 * 3600;

fn now() -> u64 {
    t("2026,11,04,04,20,00") as u64
}

fn channel(i: usize, internet: bool) -> Value {
    if internet {
        json!({"channelId": format!("11_3_{}_65535_0_0_0", 100 + i), "channelNumber": (100 + i).to_string(), "channelName": format!("Net {i}"), "imgUrl": ""})
    } else {
        json!({"channelId": format!("1_32_{}_1_0_0_0", 2 + i), "channelNumber": format!("{}-1", 2 + i), "channelName": format!("Ch {i}"), "channelMode": "Terrestrial",
               "imgUrl": "", "skipped": false, "Invisible": false})
    }
}

fn list(n: usize, internet: bool) -> Value {
    json!({"returnValue": true, "channelList": (0..n).map(|i| channel(i, internet)).collect::<Vec<_>>()})
}

fn id(i: usize) -> String {
    format!("1_32_{}_1_0_0_0", 2 + i)
}

/// A channel and its programmes: start, end, title.
type Listing<'a> = (String, Vec<(&'a str, &'a str, &'a str)>);

/// A `getEventList` answer: the programmes (start, end, title) of each channel.
fn events(rows: &[Listing]) -> Value {
    let mut out = Vec::new();
    for (channel, list) in rows {
        for (s, e, title) in list {
            out.push(json!({"channelId": channel, "programId": format!("{channel}@{s}"), "programName": title, "description": format!("About {title}"), "startTime": s, "endTime": e}));
        }
    }
    json!({"returnValue": true, "programListCount": out.len(), "programList": out})
}

struct Rig {
    g: Guide,
    fx: Vec<Effect>,
    now: u64,
}

impl Rig {
    fn open() -> Self {
        let mut g = Guide::new();
        let now = now();
        let fx = g.open(now);
        g.set_offset(OFFSET);
        Rig { g, fx, now }
    }

    /// Opened, with `n` antenna channels listed and the last channel the TV was on.
    fn with(n: usize, last: Option<usize>) -> Self {
        let mut r = Rig::open();
        r.feed(TAG_LIST, list(n, false));
        r.feed(TAG_LAST, json!({"returnValue": true, "channelId": last.map_or(String::new(), id)}));
        r
    }

    fn feed(&mut self, tag: u64, payload: Value) {
        self.fx = self.g.update(&Input::Luna { tag, payload }, self.now);
    }

    fn key(&mut self, name: &str) {
        self.fx = self.g.key(name, true, false, self.now);
    }

    fn keys(&mut self, name: &str, n: usize) {
        for _ in 0..n {
            self.key(name);
        }
    }

    fn tick(&mut self, ms: u64) {
        self.now += ms;
        self.fx = self.g.tick(self.now);
    }

    /// The luna calls the last step asked for: tag, uri, payload.
    fn calls(&self) -> Vec<(u64, String, Value)> {
        self.fx
            .iter()
            .filter_map(|e| match e {
                Effect::Luna { tag, uri, payload, .. } => Some((*tag, uri.clone(), payload.clone())),
                _ => None,
            })
            .collect()
    }

    /// Lets the focus settle and returns the programme calls that went out.
    fn settle(&mut self) -> Vec<(u64, String, Value)> {
        self.tick(FETCH_DEBOUNCE_MS + 1);
        self.calls().into_iter().filter(|c| c.1 == ch::EVENTS_URI).collect()
    }

    fn channels_of(call: &Value) -> Vec<String> {
        call["channelId"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect()
    }
}

#[test]
fn it_opens_asking_for_the_antennas_channels_and_the_one_the_tv_was_on_and_nothing_else() {
    let r = Rig::open();
    let calls = r.calls();
    assert_eq!(calls.iter().map(|c| (c.0, c.1.as_str())).collect::<Vec<_>>(), [(TAG_LIST, ch::LIST_URI), (TAG_LAST, ch::LAST_URI)]);
    assert_eq!(calls[0].2["channelMode"], json!(["Tuner"]), "the internet's 150 KB wait until they are wanted");
    assert_eq!(calls[1].2, json!({"sourceType": "INPUTSOURCE_RF", "subscribe": false}));
    assert!(r.g.is_open() && r.g.wants());
}

#[test]
fn the_list_fills_the_rows_and_the_focus_is_on_the_last_channel_whichever_answer_comes_first() {
    let mut r = Rig::open();
    r.feed(TAG_LAST, json!({"returnValue": true, "channelId": id(5)}));
    assert_eq!(r.g.rows(), 0);
    r.feed(TAG_LIST, list(12, false));
    assert_eq!((r.g.rows(), r.g.row()), (12, 5));
    assert_eq!(r.g.channel(5).map(|c| c.number.as_str()), Some("7-1"));
    assert!(r.g.top() <= 5 && 5 < r.g.top() + r.g.page().rows, "on the screen");
    // the other order
    let r = Rig::with(12, Some(7));
    assert_eq!(r.g.row(), 7);
    // a last channel the list does not have leaves the focus on the first
    let r = Rig::with(12, None);
    assert_eq!(r.g.row(), 0);
}

#[test]
fn the_focus_stays_where_the_person_put_it_when_a_late_answer_comes() {
    let mut r = Rig::open();
    r.feed(TAG_LIST, list(12, false));
    r.key("down");
    r.feed(TAG_LAST, json!({"returnValue": true, "channelId": id(9)}));
    assert_eq!(r.g.row(), 1);
}

#[test]
fn programmes_are_asked_for_once_the_focus_is_still_for_the_rows_on_the_screen_a_few_channels_at_a_time() {
    let mut r = Rig::with(30, Some(10));
    assert!(r.calls().iter().all(|c| c.1 != ch::EVENTS_URI), "nothing yet: the list's own answer is the first the model hears");
    let calls = r.settle();
    assert!(calls.len() <= IN_FLIGHT && !calls.is_empty(), "{calls:?}");
    let first = Rig::channels_of(&calls[0].2);
    assert_eq!(first[0], id(10), "the row in focus first");
    assert!(first.len() <= BATCH);
    // 6 hours at most, from the half hour before what the focus needs, on the broadcast's clock
    assert_eq!(calls[0].2["localStartTime"], json!("2026,11,03,22,00,00"));
    assert_eq!(calls[0].2["localEndTime"], json!("2026,11,04,04,00,00"));
    assert_eq!(calls[0].2["dataType"], json!(0));
    // two calls are all that fly: nothing more until one comes back
    r.tick(500);
    assert!(r.calls().is_empty());
}

#[test]
fn an_answer_is_kept_and_an_empty_one_is_no_information_and_not_a_call_to_make_again() {
    let mut r = Rig::with(12, Some(0));
    let calls = r.settle();
    let (tag, call) = (calls[0].0, calls[0].2.clone());
    let chans = Rig::channels_of(&call);
    // the first channel has two programmes, the others none in the window
    let rows = vec![(chans[0].clone(), vec![("2026,11,04,04,00,00", "2026,11,04,05,00,00", "News"), ("2026,11,04,05,00,00", "2026,11,04,06,00,00", "Film")])];
    r.feed(tag, events(&rows));
    assert_eq!(r.g.programmes(&chans[0]).len(), 2);
    let s = r.g.span_at(&chans[0], t("2026,11,04,04,30,00"));
    assert_eq!(s.programme.map(|p| p.title.as_str()), Some("News"));
    assert_eq!(r.g.programmes(&chans[1]).len(), 0);
    let gap = r.g.span_at(&chans[1], t("2026,11,04,04,30,00"));
    assert!(gap.programme.is_none() && !gap.loading, "asked for and nothing came: no information, not loading");
    // a stretch that was not asked for is loading
    let later = r.g.span_at(&chans[1], t("2026,11,07,04,30,00"));
    assert!(later.loading);
    // asked for again? the window is held; the next call is for what is missing
    r.tick(FETCH_DEBOUNCE_MS + 1);
    for c in r.calls().iter().filter(|c| c.1 == ch::EVENTS_URI) {
        let ids = Rig::channels_of(&c.2);
        assert!(!(ids.contains(&chans[0]) && c.2["localStartTime"] == call["localStartTime"] && c.2["localEndTime"] == call["localEndTime"]), "{c:?}");
    }
}

#[test]
fn a_call_that_failed_is_left_alone_for_a_while_and_then_tried_again() {
    let mut r = Rig::with(3, Some(0));
    let calls = r.settle();
    let tag = calls[0].0;
    let ids = Rig::channels_of(&calls[0].2);
    r.feed(tag, json!({"returnValue": false, "errorCode": -1, "errorText": "not running"}));
    assert!(r.g.data_fault().is_some());
    r.tick(FETCH_DEBOUNCE_MS + 1);
    let again: Vec<_> = r.calls().into_iter().filter(|c| c.1 == ch::EVENTS_URI).flat_map(|c| Rig::channels_of(&c.2)).collect();
    assert!(!again.iter().any(|c| ids.contains(c)) || again.is_empty(), "left alone: {again:?}");
    r.tick(RETRY_MS + 1);
    let later: Vec<_> = r.calls().into_iter().filter(|c| c.1 == ch::EVENTS_URI).flat_map(|c| Rig::channels_of(&c.2)).collect();
    assert!(ids.iter().any(|c| later.contains(c)), "tried again: {later:?}");
}

/// The helper's online listings of `channel`: start, end, title.
fn online(channel: &str, list: &[(&str, &str, &str)]) -> Value {
    let programmes: Vec<Value> =
        list.iter().map(|(s, e, title)| json!({"id": format!("on@{s}"), "channel": channel, "title": title, "start": t(s), "end": t(e)})).collect();
    json!({"ok": true, "on": true, "programmes": programmes})
}

fn fetches(r: &Rig) -> Vec<(u64, String)> {
    r.fx.iter().filter_map(|e| if let Effect::Fetch { tag, path } = e { Some((*tag, path.clone())) } else { None }).collect()
}

#[test]
fn the_online_listings_go_with_each_call_and_lie_over_lgs_and_a_plugin_that_is_off_is_asked_nothing_more() {
    let mut r = Rig::with(12, Some(0));
    let calls = r.settle();
    let asked = fetches(&r);
    assert_eq!(asked.len(), calls.len(), "one online call with each of LG's");
    let chans = Rig::channels_of(&calls[0].2);
    assert!(asked[0].1.starts_with(ONLINE_PATH) && asked[0].1.contains(&format!("&channel={}", chans[0])), "{}", asked[0].1);
    let lg = vec![(chans[0].clone(), vec![("2026,11,04,04,00,00", "2026,11,04,05,00,00", "News"), ("2026,11,04,05,00,00", "2026,11,04,06,00,00", "Film")])];
    r.feed(calls[0].0, events(&lg));
    let answer = online(&chans[0], &[("2026,11,04,04,00,00", "2026,11,04,05,00,00", "The News Hour")]);
    r.fx = r.g.update(&Input::Fetched { tag: asked[0].0, status: 200, data: answer }, r.now);
    assert!(r.fx.contains(&Effect::Redraw));
    let titles: Vec<&str> = r.g.programmes(&chans[0]).iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["The News Hour", "Film"], "the online programme wins its hour, LG's fill the rest");

    // an older helper, or the plugin off: nothing more is asked of it while the guide is open
    let mut off = Rig::with(12, Some(0));
    let calls = off.settle();
    let asked = fetches(&off);
    off.fx = off.g.update(&Input::Fetched { tag: asked[0].0, status: 404, data: json!({"error": {"message": "no route"}}) }, off.now);
    for c in &calls {
        off.feed(c.0, events(&[]));
    }
    off.keys("right", 12);
    assert!(!off.settle().is_empty(), "LG is asked for the hours the focus went to");
    assert!(fetches(&off).is_empty(), "{:?}", off.fx);
}

/// Channel 0 has a day of listings round now: 03:30-04:15, 04:15-05:00 (on the air), 05:00-06:00, 06:00-06:30, 06:30-08:00.
fn with_listings() -> Rig {
    let mut r = Rig::with(12, Some(0));
    let calls = r.settle();
    let rows = vec![(
        id(0),
        vec![
            ("2026,11,04,03,30,00", "2026,11,04,04,15,00", "Earlier"),
            ("2026,11,04,04,15,00", "2026,11,04,05,00,00", "On now"),
            ("2026,11,04,05,00,00", "2026,11,04,06,00,00", "Next"),
            ("2026,11,04,06,00,00", "2026,11,04,06,30,00", "Short"),
            ("2026,11,04,06,30,00", "2026,11,04,08,00,00", "Long"),
        ],
    )];
    r.feed(calls[0].0, events(&rows));
    r
}

#[test]
fn a_gap_is_the_whole_stretch_between_two_programmes_whatever_part_of_it_is_asked_for() {
    let r = with_listings();
    let c = id(0);
    let before = r.g.spans(&c, t("2026,11,04,01,00,00"), t("2026,11,04,01,10,00"));
    assert_eq!(before.len(), 1);
    assert_eq!((before[0].start, before[0].end), (r.g.origin(), t("2026,11,04,03,30,00")), "from the start of the grid to the first programme");
    let after = r.g.spans(&c, t("2026,11,04,09,00,00"), t("2026,11,04,09,10,00"));
    assert_eq!((after[0].start, after[0].end), (t("2026,11,04,08,00,00"), r.g.horizon()), "from the last programme to the end of the grid");
    let across = r.g.spans(&c, t("2026,11,04,02,00,00"), t("2026,11,04,05,30,00"));
    assert_eq!(across.iter().map(|s| s.programme.map(|p| p.title.as_str())).collect::<Vec<_>>(), [None, Some("Earlier"), Some("On now"), Some("Next")]);
    assert_eq!(across[0].start, r.g.origin());
}

fn title(r: &Rig) -> String {
    r.g.focus_span().and_then(|s| s.programme.map(|p| p.title.clone())).unwrap_or_default()
}

#[test]
fn it_opens_on_the_programme_on_the_air_with_the_view_at_the_half_hour() {
    let r = with_listings();
    assert_eq!(title(&r), "On now");
    assert_eq!(r.g.view(), t("2026,11,04,04,00,00"), "the half hour before now; the programme that began before it is the surface's to show");
    assert_eq!(r.g.at(), now() as i64);
}

#[test]
fn right_and_left_go_cell_by_cell_and_the_view_moves_only_as_far_as_the_focus_needs() {
    let mut r = with_listings();
    let view = r.g.view();
    r.key("right");
    assert_eq!(title(&r), "Next");
    assert_eq!(r.g.view(), view, "it fits: nothing moves");
    r.key("right");
    assert_eq!(title(&r), "Short");
    r.key("right");
    assert_eq!(title(&r), "Long");
    let p = r.g.page();
    let (_, long_end) = r.g.focus_extent().unwrap();
    assert!(r.g.view() > view && r.g.view() + p.span >= long_end, "a cell past the right edge brings its end in: view {} span {}", r.g.view(), p.span);
    assert!(r.g.view() < t("2026,11,04,06,30,00"), "and only so far");
    r.keys("left", 4);
    assert_eq!(title(&r), "Earlier");
    assert!(r.g.view() <= t("2026,11,04,03,30,00"), "a cell past the left edge brings its start in");
    assert!(r.g.view() >= t("2026,11,04,03,25,00"), "and only so far: {}", r.g.view());
    r.key("left");
    assert!(r.g.focus_span().is_some_and(|s| s.programme.is_none()), "before the first programme is a gap, an hour at a time");
}

#[test]
fn up_and_down_keep_the_instant_and_do_not_move_the_view() {
    let mut r = with_listings();
    r.key("right");
    r.key("right");
    let (view, at) = (r.g.view(), r.g.at());
    r.key("down");
    assert_eq!((r.g.row(), r.g.at(), r.g.view()), (1, at, view));
    assert!(r.g.focus_span().is_some_and(|s| s.programme.is_none()), "a row with nothing in it is a gap");
    r.key("up");
    assert_eq!(title(&r), "Short");
}

#[test]
fn a_gap_is_gone_through_an_hour_at_a_time() {
    let mut r = with_listings();
    r.key("down");
    let at = r.g.at();
    r.key("right");
    assert_eq!(r.g.at() - at.div_euclid(HOUR) * HOUR, HOUR, "the next hour");
    r.key("left");
    r.key("left");
    assert!(r.g.at() < at, "and back");
}

#[test]
fn the_view_scrolls_by_rows_only_as_far_as_needed_and_ch_pages_by_a_screen_less_one() {
    let mut r = Rig::with(40, Some(0));
    r.g.set_page(Page { rows: 8, span: 3 * HOUR });
    for i in 1..=7 {
        r.key("down");
        assert_eq!((r.g.row(), r.g.top()), (i, 0), "still on the screen");
    }
    r.key("down");
    assert_eq!((r.g.row(), r.g.top()), (8, 1), "one row more: one row of scroll");
    r.key("chdown");
    assert_eq!((r.g.row(), r.g.top()), (15, 8));
    r.key("chup");
    assert_eq!(r.g.row(), 8);
    assert!(r.g.top() <= 8 && 8 < r.g.top() + 8);
    r.keys("chdown", 20);
    assert_eq!((r.g.row(), r.g.top()), (39, 32), "the last page ends at the last row");
    r.keys("chup", 20);
    assert_eq!((r.g.row(), r.g.top()), (0, 0));
}

#[test]
fn a_notch_of_the_wheel_is_one_row_and_stops_at_the_ends_and_the_chips_and_the_sheet_do_not_hear_it() {
    let mut r = Rig::with(40, Some(0));
    r.g.set_page(Page { rows: 8, span: 3 * HOUR });
    r.key("wheelup");
    assert_eq!((r.g.row(), r.g.region()), (0, Region::Grid), "the first row is the end: the chips are for the arrow");
    r.keys("wheeldown", 9);
    assert_eq!((r.g.row(), r.g.top()), (9, 2), "a row a notch, the view only as far as it must");
    r.keys("wheelup", 3);
    assert_eq!(r.g.row(), 6);
    r.keys("wheeldown", 60);
    assert_eq!(r.g.row(), 39, "the last row is the end");
    r.key("info");
    r.key("wheelup");
    assert!(r.g.sheet_open() && r.g.row() == 39, "the sheet has the keys");
    r.key("back");
    r.keys("wheelup", 39);
    r.key("up");
    assert_eq!(r.g.region(), Region::Chips);
    r.key("wheeldown");
    assert_eq!(r.g.region(), Region::Chips, "the chips are the arrows'");
}

#[test]
fn up_from_the_first_row_is_the_chips_and_down_is_the_grid_where_it_was() {
    let mut r = with_listings();
    r.key("right");
    let (row, at) = (r.g.row(), r.g.at());
    r.key("up");
    assert_eq!(r.g.region(), Region::Chips);
    assert_eq!(r.g.chips()[r.g.chip_index()], Chip::Filter(Filter::Antenna), "on the chip of the filter that is on");
    r.key("down");
    assert_eq!((r.g.region(), r.g.row(), r.g.at()), (Region::Grid, row, at));
}

#[test]
fn the_filters_pick_the_rows_and_the_internet_list_is_asked_for_once_when_it_is_wanted() {
    let mut r = Rig::with(6, Some(0));
    assert_eq!(r.g.chips(), [Chip::Filter(Filter::All), Chip::Filter(Filter::Antenna), Chip::Filter(Filter::Internet), Chip::Now]);
    r.key("up");
    r.key("left");
    r.key("ok");
    assert_eq!(r.g.filter(), Filter::All);
    assert_eq!(r.calls().iter().filter(|c| c.0 == TAG_LIST_NET).count(), 1);
    assert!(r.g.loading_list(), "the grid says so while it waits");
    assert_eq!(r.g.rows(), 6, "what it has until the rest comes");
    r.feed(TAG_LIST_NET, list(459, true));
    assert_eq!(r.g.rows(), 6 + 459);
    assert!(!r.g.loading_list());
    assert_eq!(r.g.channel(r.g.row()).map(|c| c.number.as_str()), Some("2-1"), "the focus is on the channel it was on");
    r.keys("right", 2);
    r.key("ok");
    assert_eq!(r.g.filter(), Filter::Internet);
    assert_eq!(r.g.rows(), 459);
    r.keys("left", 2);
    r.key("ok");
    assert!(r.calls().iter().all(|c| c.0 != TAG_LIST_NET), "never twice");
    assert_eq!((r.g.filter(), r.g.rows()), (Filter::All, 465));
}

#[test]
fn a_set_with_no_antenna_channels_opens_on_the_internets() {
    let mut r = Rig::open();
    r.feed(TAG_LIST, json!({"returnValue": true, "channelList": []}));
    assert_eq!(r.g.filter(), Filter::Internet);
    assert_eq!(r.calls().iter().filter(|c| c.0 == TAG_LIST_NET).count(), 1);
    assert!(!r.g.chips().contains(&Chip::Filter(Filter::Antenna)));
    r.feed(TAG_LIST_NET, list(3, true));
    assert_eq!(r.g.rows(), 3);
}

#[test]
fn a_service_that_does_not_answer_is_said_and_nothing_else_happens() {
    let mut r = Rig::open();
    r.feed(TAG_LIST, json!({"returnValue": false, "errorCode": -1, "errorText": "not running"}));
    assert!(r.g.list_fault().is_some() && r.g.rows() == 0);
    r.key("down");
    r.key("ok");
    assert!(r.g.is_open(), "OK with nothing to tune does nothing");
    assert!(r.calls().is_empty());
    r.key("back");
    assert!(!r.g.is_open());
}

#[test]
fn ok_on_a_programme_on_the_air_tunes_as_lgs_guide_does_and_leaves_when_lg_has_answered() {
    let mut r = with_listings();
    r.key("ok");
    let tune: Vec<_> = r.calls().into_iter().filter(|c| c.0 == TAG_TUNE).collect();
    assert_eq!(tune.len(), 1);
    assert_eq!(tune[0].1, ch::LAUNCH_URI);
    assert_eq!(tune[0].2, ch::tune_request(&id(0)));
    assert!(r.g.is_open() && r.g.tuning(), "it leaves when the switch is answered, not before");
    r.key("ok");
    r.key("right");
    assert!(r.calls().is_empty() && r.g.at() == now() as i64, "a second OK and the arrows are not heard while the channel is being switched");
    r.feed(TAG_TUNE, json!({"returnValue": true, "appId": "com.webos.app.livetv"}));
    assert!(!r.g.is_open());
}

#[test]
fn a_switch_lg_refuses_is_said_and_the_guide_stays_and_one_it_does_not_answer_leaves_by_itself() {
    let mut r = with_listings();
    r.key("ok");
    r.feed(TAG_TUNE, json!({"returnValue": false, "errorText": "no such app"}));
    assert!(r.g.is_open() && !r.g.tuning() && r.g.tune_fault());
    r.key("right");
    r.key("left");
    assert_eq!(title(&r), "On now", "the arrows work again");
    r.key("ok");
    assert!(r.g.tuning() && !r.g.tune_fault());
    r.tick(TUNE_WAIT_MS - 100);
    assert!(r.g.is_open());
    r.tick(200);
    assert!(!r.g.is_open(), "the app switch is the answer when LG says nothing");
    let mut r = with_listings();
    r.key("ok");
    r.key("back");
    assert!(!r.g.is_open(), "Back leaves at once");
}

#[test]
fn ok_on_a_programme_that_is_not_on_the_air_opens_the_sheet_and_ok_there_tunes() {
    let mut r = with_listings();
    r.keys("right", 2);
    r.key("ok");
    assert!(r.g.sheet_open() && r.g.is_open());
    assert!(r.calls().is_empty());
    let s = r.g.sheet().unwrap();
    assert_eq!((s.main.map(|p| p.title.as_str()), s.when), (Some("Short"), "Later"));
    assert_eq!(s.after.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["Long"]);
    r.key("ok");
    assert!(r.calls().iter().any(|c| c.0 == TAG_TUNE) && !r.g.sheet_open());
    r.feed(TAG_TUNE, json!({"returnValue": true}));
    assert!(!r.g.is_open());
}

#[test]
fn the_sheet_says_how_the_programme_stands_to_now_and_back_closes_it_before_the_guide() {
    let mut r = with_listings();
    r.key("info");
    let s = r.g.sheet().unwrap();
    assert_eq!((s.main.map(|p| p.title.as_str()), s.when), (Some("On now"), "On now"));
    assert_eq!(s.after.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["Next", "Short"]);
    r.key("down");
    assert_eq!(r.g.row(), 0, "the sheet has the keys");
    r.key("back");
    assert!(!r.g.sheet_open() && r.g.is_open());
    r.key("right");
    r.key("info");
    assert_eq!(r.g.sheet().unwrap().when, "Next");
    r.key("info");
    assert!(!r.g.sheet_open(), "INFO again closes it");
    r.keys("left", 2);
    r.key("info");
    assert_eq!(r.g.sheet().unwrap().when, "Earlier");
    r.key("back");
    r.key("back");
    assert!(!r.g.is_open());
}

#[test]
fn the_now_chip_goes_back_to_the_programmes_on_the_air_wherever_the_view_is() {
    let mut r = with_listings();
    r.keys("right", 40);
    assert!(r.g.at() > now() as i64 + DAY, "a day and more along the row");
    r.key("up");
    assert_eq!(r.g.region(), Region::Chips);
    r.keys("right", 2);
    assert_eq!(r.g.chips()[r.g.chip_index()], Chip::Now);
    r.key("ok");
    assert_eq!((r.g.region(), r.g.at(), r.g.view()), (Region::Grid, now() as i64, t("2026,11,04,04,00,00")));
    assert_eq!(title(&r), "On now");
    // the pointer gets there in one click
    r.keys("right", 40);
    r.fx = r.g.click(Target::Chip(3), r.now);
    assert_eq!((r.g.region(), r.g.at(), r.g.view()), (Region::Grid, now() as i64, t("2026,11,04,04,00,00")));
}

#[test]
fn a_click_focuses_and_a_second_click_acts_and_the_pointer_does_not_move_the_focus_by_itself() {
    let mut r = with_listings();
    let (row, at) = (0, t("2026,11,04,05,30,00"));
    r.fx = r.g.click(Target::Cell { row, at }, r.now);
    assert_eq!(title(&r), "Next");
    assert!(r.g.is_open() && r.calls().is_empty(), "the first click only focuses");
    r.fx = r.g.click(Target::Cell { row, at }, r.now);
    assert!(r.g.sheet_open(), "a programme that is not on the air opens its sheet");
    r.fx = r.g.click(Target::Watch, r.now);
    assert!(r.calls().iter().any(|c| c.0 == TAG_TUNE) && r.g.tuning(), "the switch is on its way");
    // a chip is chosen by one click
    let mut r = Rig::with(6, Some(0));
    r.fx = r.g.click(Target::Chip(0), r.now);
    assert_eq!((r.g.filter(), r.g.region()), (Filter::All, Region::Grid));
}

#[test]
fn it_closes_on_the_home_key_and_by_itself_when_nobody_touches_it_and_gives_back_what_is_in_flight() {
    let mut r = with_listings();
    r.tick(FETCH_DEBOUNCE_MS + 1);
    let flying = r.g.state()["inFlight"].as_u64().unwrap();
    r.fx = r.g.update(&Input::Topic { topic: "homekey".into(), data: json!({}) }, r.now);
    assert!(!r.g.is_open());
    assert_eq!(r.fx.iter().filter(|e| matches!(e, Effect::Cancel { .. })).count() as u64, flying);
    let mut r = with_listings();
    r.tick(IDLE_MS - 1000);
    assert!(r.g.is_open());
    r.key("right");
    r.tick(IDLE_MS - 1000);
    assert!(r.g.is_open(), "a key is a touch");
    r.tick(1001);
    assert!(!r.g.is_open());
}

#[test]
fn the_minute_moves_the_now_marker_and_nothing_else_asks_for_a_frame_at_rest() {
    let mut r = with_listings();
    let due = r.g.due_at().unwrap();
    assert!(due as i64 <= (now() as i64 / MIN + 1) * MIN + 1, "the next thing due is at most the minute's turn: {due}");
    r.now = ((now() as i64 / MIN + 1) * MIN) as u64;
    r.fx = r.g.tick(r.now);
    assert!(r.fx.contains(&Effect::Redraw));
    assert_eq!(r.g.now(), r.now as i64);
    r.fx = r.g.tick(r.now + 1);
    assert!(r.fx.is_empty(), "within the minute nothing changes");
}

#[test]
fn hundreds_of_channels_and_days_of_listings_cost_a_few_calls_and_the_rows_are_not_walked() {
    let mut r = Rig::with(600, Some(300));
    assert_eq!(r.g.rows(), 600);
    let calls = r.settle();
    assert!(calls.len() <= IN_FLIGHT);
    let asked: Vec<String> = calls.iter().flat_map(|c| Rig::channels_of(&c.2)).collect();
    assert!(asked.len() <= IN_FLIGHT * BATCH && asked.iter().all(|a| (290..=310).any(|i| *a == id(i))), "only the rows round the focus: {asked:?}");
    // days of listings for one channel, in one answer
    let mut prog = Vec::new();
    for h in 0..120 {
        let s = ch::stamp(t("2026,11,04,00,00,00") + h * HOUR, 0);
        let e = ch::stamp(t("2026,11,04,00,00,00") + (h + 1) * HOUR, 0);
        prog.push((s, e));
    }
    let rows: Vec<Listing> = vec![(id(300), prog.iter().map(|(s, e)| (s.as_str(), e.as_str(), "Hourly")).collect())];
    r.feed(calls[0].0, events(&rows));
    assert_eq!(r.g.programmes(&id(300)).len(), 120);
    // a hundred steps along the row and a hundred down it make no call that is not for what is shown
    let mut asked = 0;
    for _ in 0..100 {
        r.key("right");
        asked += r.calls().iter().filter(|c| c.1 == ch::EVENTS_URI).count();
    }
    assert_eq!(asked, 0, "a key asks nothing by itself: the focus has to be still");
    r.keys("chdown", 30);
    r.tick(FETCH_DEBOUNCE_MS + 1);
    assert!(r.calls().len() <= IN_FLIGHT);
}

#[test]
fn a_guide_that_is_not_open_has_nothing_due_whatever_its_surface_has_set() {
    let mut g = Guide::new();
    // the surface sets its page when it is built, with the model's clock at 0, whether or not the guide ever opens
    assert!(g.set_page(Page { rows: 5, span: 2 * HOUR }));
    assert_eq!(g.due_at(), None, "a closed guide's tick does nothing, so it must not ask for one");
    g.open(now());
    assert!(g.due_at().is_some(), "an open one has its fetch, its minute and its idle close");
}
