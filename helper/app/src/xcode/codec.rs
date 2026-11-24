//! The Live TV Tuner's converter over the engine: a station's MPEG-2 pictures in, the same pictures as H.264 out.
//!
//! One conversion is one `Priority::Live` job of the engine, fed by the stream's pump (`push` and `pull` never wait). The engine's faults are
//! the tuner's own words: what is not for this picture or for now (an app has the video hardware, a size the child cannot make) is
//! `Unsupported`, which costs the converter nothing; what the child did wrong is `Failed` or `Stalled`, which rests it; a call into the
//! hardware that never came back is `Wedged`, which turns it off until the helper restarts.

use super::gate::Look;
use crate::prewarm::room::{self, Lease};
use lumatuner::mpeg2::Format;
use lumatuner::{Codec, Convert, Fault, Profile, Unit, Want};
use lumaxcode::{Au, Engine, H264Profile, JobSpec, Out, Output, Priority, PushError, Reason, Sink, Source, SourceCodec, StartError, Wait};

/// What the helper's converter lists in `/status.json` (`transcode.profiles`), best picture first: the profiles measured on the set (`heavy` and
/// `mobile`: the measurements are in `docs/research/hw-transcode.md`). The others are made when a request names them, and join this list when
/// they have been measured.
const MADE: [Profile; 2] = [Profile::Heavy, Profile::Mobile];

/// `CmaFree` the converter is given before it starts, MB: its chain at 1080 needs about 30 (bitstream buffers, four NV12 pictures, the encoder), at
/// 720 and below about 24, and a start that finds the contiguous memory short has wedged the set, so each has about a third in hand.
fn cma_needed_mb(source: &Format) -> u64 {
    if source.height > 720 { 40 } else { 32 }
}

/// The converter the tuner is given while the plugin's setting is on.
pub struct Converter {
    engine: Engine,
    look: Look,
}

impl Converter {
    pub fn new(engine: Engine, look: Look) -> Self {
        Self { engine, look }
    }
}

impl Codec for Converter {
    fn open(&self, source: &Format, want: &Want) -> Result<Box<dyn Convert>, Fault> {
        let output = output_of(source, want);
        // the media server is asked once before the hardware is: a decoder an app took a moment ago closes the gate here
        self.look.now();
        // the kept apps hold CMA the chain needs: they are closed for it, and stay closed while the stream runs
        let need = cma_needed_mb(source);
        let lease = room::open(need).map_err(|short| Fault::Unsupported(format!("{} MB of CMA is free and the converter needs {need}", short.free_mb)))?;
        let spec = JobSpec {
            owner: "tuner".to_owned(),
            priority: Priority::Live,
            source: Source::Units { codec: SourceCodec::Mpeg2, width: u32::from(source.width), height: u32::from(source.height) },
            output,
            wait: Wait::Fail,
            sink: Sink::Deliver,
        };
        match self.engine.start(spec) {
            Ok(job) => Ok(Box::new(Live { job, units: Vec::new(), _room: Some(lease) })),
            Err(e) => Err(fault_of_start(e)),
        }
    }

    fn profiles(&self) -> &'static [Profile] {
        &MADE
    }
}

/// What the child is asked to make for `want` from pictures of `source`: its size when that is smaller than the picture, every Nth picture of a
/// rate above the one wanted, a keyframe every two seconds, constant bit rate, and the shape of its pixels when they are not square: the child never
/// enlarges, so a 16:9 station in 720x480 comes out 720x480 and says its pixels are 32:27.
fn output_of(source: &Format, want: &Want) -> Output {
    let (num, den) = source.frame_rate();
    let from = num as f64 / den as f64;
    let to = f64::from(want.rate.0) / f64::from(want.rate.1.max(1));
    let keep_every = ((from / to).round() as u32).max(1);
    let fps = to.round().clamp(1.0, 60.0) as u32;
    // the child makes the smaller of the picture and what is wanted in each direction, and none of it when nothing is smaller
    let (width, height) = (want.width.min(source.width), want.height.min(source.height));
    let whole = width == source.width && height == source.height;
    Output {
        width: if whole { 0 } else { u32::from(width) },
        height: if whole { 0 } else { u32::from(height) },
        keep_every,
        fps,
        kbps: want.kbps,
        cbr: true,
        gop: fps * 2,
        profile: H264Profile::High,
        sar: pixel_aspect(source, width, height),
    }
}

/// The shape of one pixel of a `width` x `height` picture made from `source`, which is meant to be shown in the shape it signals (4:3, 16:9, 2.21:1):
/// `None` when the pixels are square, or within 2% of it, which no eye tells and which a stream that says nothing gives every player. A 16:9 station
/// in 704x480 has 40:33 pixels, a 4:3 one in 704x480 has 10:11, a 16:9 one in 1440x1080 has 4:3.
fn pixel_aspect(source: &Format, width: u16, height: u16) -> Option<(u16, u16)> {
    let (across, down) = source.display_aspect();
    let (mut num, mut den) = (u64::from(across) * u64::from(height), u64::from(down) * u64::from(width));
    if num == 0 || den == 0 {
        return None;
    }
    let divisor = gcd(num, den);
    (num, den) = (num / divisor, den / divisor);
    if num.abs_diff(den) * 50 < num.min(den) {
        return None;
    }
    // the SPS has 16 bits for each
    while num > u64::from(u16::MAX) || den > u64::from(u16::MAX) {
        (num, den) = (num.div_ceil(2), den.div_ceil(2));
    }
    Some((num as u16, den as u16))
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The engine's faults in the tuner's words.
fn fault_of(fault: &lumaxcode::Fault) -> Fault {
    match fault {
        lumaxcode::Fault::Unsupported(why) => Fault::Unsupported(why.clone()),
        lumaxcode::Fault::Failed(why) => Fault::Failed(why.clone()),
        lumaxcode::Fault::Stalled(why) => Fault::Stalled(why.clone()),
        lumaxcode::Fault::Wedged(why) => Fault::Wedged(why.clone()),
    }
}

fn fault_of_start(error: StartError) -> Fault {
    match error {
        StartError::Wedged(why) => Fault::Wedged(why),
        StartError::Setup(fault) => fault_of(&fault),
        // not now, not for this: the stream goes out as the station sends it, and the next request asks again
        StartError::Held(why) | StartError::Busy(why) | StartError::Unsupported(why) => Fault::Unsupported(why),
        StartError::Cooling(rest) => Fault::Failed(format!("the child failed repeatedly and is resting for {} s", rest.as_secs())),
        StartError::TooMany => Fault::Failed("too many conversions at once".to_owned()),
    }
}

/// Why a job that is not running is not, in the tuner's words: a fault of the child is its fault, anything else is the engine giving the
/// hardware up (an app needs it, another viewer took it, the helper is stopping), which is nothing wrong.
fn fault_of_reason(reason: &Reason) -> Fault {
    match reason {
        Reason::Fault(fault) => fault_of(fault),
        other => Fault::Unsupported(other.to_string()),
    }
}

/// Whether the MPEG-2 picture `es` can start a decode: it has a sequence or a group header ahead of its picture, or it is an I picture.
fn starts_a_decode(es: &[u8]) -> bool {
    let mut rest = es;
    while let Some(zeros) = rest.windows(3).position(|w| w == [0, 0, 1]) {
        let after = &rest[zeros + 3..];
        match after.first() {
            // sequence header, group of pictures
            Some(0xB3 | 0xB8) => return true,
            // picture header: temporal reference (10 bits), then the coding type (3 bits): 1 is an I picture
            Some(0x00) => return after.get(2).is_some_and(|b| (b >> 3) & 7 == 1),
            Some(_) => rest = after,
            None => return false,
        }
    }
    false
}

/// The job's two sides, as the conversion uses them.
pub trait Session: Send {
    fn push(&self, au: Au) -> Result<(), PushError>;
    fn pull(&self, out: &mut Vec<Out>) -> Result<(), Reason>;
}

impl Session for lumaxcode::Job {
    fn push(&self, au: Au) -> Result<(), PushError> {
        lumaxcode::Job::push(self, au)
    }

    fn pull(&self, out: &mut Vec<Out>) -> Result<(), Reason> {
        lumaxcode::Job::pull(self, out)
    }
}

/// One stream's conversion.
struct Live<S: Session> {
    job: S,
    /// The units taken from the job, before they are the tuner's.
    units: Vec<Out>,
    /// The memory held for the stream: the kept apps come back when it is dropped.
    _room: Option<Lease>,
}

impl<S: Session> Convert for Live<S> {
    fn push(&mut self, picture: &[u8], pts: u64) -> Result<(), Fault> {
        let au = Au { pts, key: starts_a_decode(picture), data: picture.to_vec() };
        match self.job.push(au) {
            // a stream that began between keyframes: its pictures wait for the first one, which the child could not decode them without
            Ok(()) | Err(PushError::WantKey) => Ok(()),
            Err(PushError::Full) => Err(Fault::Stalled("the converter is not keeping up with the pictures".to_owned())),
            Err(PushError::Paused(reason) | PushError::Ended(reason)) => Err(fault_of_reason(&reason)),
        }
    }

    fn pull(&mut self, out: &mut Vec<Unit>) -> Result<(), Fault> {
        self.units.clear();
        let ended = self.job.pull(&mut self.units);
        out.extend(self.units.drain(..).map(|unit| Unit { pts: unit.pts, data: unit.data }));
        ended.map_err(|reason| fault_of_reason(&reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::VecDeque;
    use std::sync::Arc;

    fn station(width: u16, height: u16, rate: u8, aspect: u8) -> Format {
        Format { width, height, rate, aspect, progressive: false }
    }

    fn sized(want: &Output) -> (u32, u32) {
        (want.width, want.height)
    }

    #[test]
    fn the_profiles_ask_for_what_the_child_makes() {
        let hd = station(1920, 1080, 4, 3);
        let heavy = output_of(&hd, &Profile::Heavy.want(&hd));
        assert_eq!(
            (sized(&heavy), heavy.keep_every, heavy.fps, heavy.kbps, heavy.gop, heavy.sar),
            ((0, 0), 1, 30, 9000, 60, None),
            "1080i is made whole, as progressive 29.97 with square pixels and a keyframe every two seconds"
        );

        let mobile = output_of(&hd, &Profile::Mobile.want(&hd));
        assert_eq!((sized(&mobile), mobile.keep_every, mobile.fps, mobile.kbps), ((1280, 720), 1, 30, 3500));
        assert!(mobile.cbr && mobile.profile == H264Profile::High);

        let p60 = station(1280, 720, 7, 3);
        let mobile = output_of(&p60, &Profile::Mobile.want(&p60));
        assert_eq!((sized(&mobile), mobile.keep_every, mobile.fps), ((0, 0), 2, 30), "720p60 is every other picture, unscaled");
        let heavy = output_of(&p60, &Profile::Heavy.want(&p60));
        assert_eq!((sized(&heavy), heavy.keep_every, heavy.fps, heavy.gop, heavy.kbps), ((0, 0), 1, 60, 120, 7500), "heavy keeps all 59.94 pictures");

        let small = output_of(&hd, &Profile::Internet240.want(&hd));
        assert_eq!((sized(&small), small.kbps), ((432, 240), 400));
        let film = station(1920, 1080, 1, 3);
        assert_eq!(output_of(&film, &Profile::Mobile.want(&film)).fps, 24);
    }

    #[test]
    fn a_picture_that_is_not_made_in_its_shape_says_what_shape_its_pixels_are() {
        let sar = |source: &Format, profile: Profile| output_of(source, &profile.want(source)).sar;
        // 480i at its own size, in the two frames the stations use, 16:9 and 4:3
        assert_eq!(sar(&station(704, 480, 4, 3), Profile::Heavy), Some((40, 33)));
        assert_eq!(sar(&station(720, 480, 4, 3), Profile::Heavy), Some((32, 27)));
        assert_eq!(sar(&station(704, 480, 4, 2), Profile::Heavy), Some((10, 11)));
        assert_eq!(sar(&station(720, 480, 4, 2), Profile::Heavy), Some((8, 9)));
        assert_eq!(sar(&station(640, 480, 4, 2), Profile::Heavy), None, "4:3 in 640x480 is square");
        // what is not enlarged keeps its pixels' shape, what is scaled to the profile's box is square again
        let wide_sd = station(704, 480, 4, 3);
        assert_eq!(sar(&wide_sd, Profile::Mobile), Some((40, 33)));
        assert_eq!(sar(&wide_sd, Profile::Internet480), Some((40, 33)), "848x480 would be an enlargement");
        assert_eq!(sar(&wide_sd, Profile::Internet360), None);
        assert_eq!(sar(&station(704, 480, 4, 2), Profile::Mobile), None, "4:3 in 704x480 is scaled to 640x480");
        // 16:9 in 1440x1080, a frame some stations use
        let anamorphic_hd = station(1440, 1080, 4, 3);
        assert_eq!(sar(&anamorphic_hd, Profile::Heavy), Some((4, 3)));
        assert_eq!(sar(&anamorphic_hd, Profile::Mobile), None, "scaled down to 1280x720 it is square");
        // square pixels say nothing, and so does a shape the picture only misses by what no eye tells
        for square in [station(1920, 1080, 4, 3), station(1280, 720, 7, 3), station(704, 480, 4, 1)] {
            assert_eq!(sar(&square, Profile::Heavy), None, "{square:?}");
        }
        assert_eq!(sar(&station(1920, 1080, 4, 3), Profile::Internet240), None, "432x240 is 1.2% from 16:9");
        assert_eq!(sar(&station(1920, 1080, 4, 4), Profile::Heavy), Some((1989, 1600)), "2.21:1");
    }

    #[test]
    fn a_ratio_the_sps_cannot_hold_is_scaled_down_with_its_shape() {
        // coprime sizes: the reduced ratio is about 1.95 million to 1.1 million, which 16 bits cannot hold
        let source = Format { width: 1913, height: 1087, rate: 4, aspect: 1, progressive: false };
        let (num, den) = pixel_aspect(&source, 1019, 1021).unwrap();
        let (exact, held) = (f64::from(1913 * 1021) / f64::from(1087 * 1019), f64::from(num) / f64::from(den));
        assert!(num > 1000 && den > 1000 && (held / exact - 1.0).abs() < 1e-3, "{num}:{den} is {held}, not {exact}");
        assert_eq!(pixel_aspect(&source, 0, 480), None, "a picture of no width has no pixels to describe");
    }

    #[test]
    fn the_profiles_listed_are_in_the_tuners_order() {
        let at = |p: &Profile| Profile::ALL.iter().position(|a| a == p).expect("a profile of the tuner");
        assert!(MADE.windows(2).all(|pair| at(&pair[0]) < at(&pair[1])), "{MADE:?}: best picture first, each once");
    }

    #[test]
    fn the_engines_words_are_the_tuners() {
        let held = fault_of_start(StartError::Held("com.webos.app.home: netflix is in front".to_owned()));
        assert_eq!(held, Fault::Unsupported("com.webos.app.home: netflix is in front".to_owned()));
        assert!(matches!(fault_of_start(StartError::Wedged("a call did not return".to_owned())), Fault::Wedged(_)));
        assert!(matches!(fault_of_start(StartError::Setup(lumaxcode::Fault::Failed("no decoder".to_owned()))), Fault::Failed(_)));
        assert!(matches!(fault_of_start(StartError::Setup(lumaxcode::Fault::Stalled("no ready".to_owned()))), Fault::Stalled(_)));
        assert!(matches!(fault_of_start(StartError::Cooling(std::time::Duration::from_secs(20))), Fault::Failed(_)));
        assert!(matches!(fault_of_reason(&Reason::Held("an app".to_owned())), Fault::Unsupported(_)));
        assert!(matches!(fault_of_reason(&Reason::Replaced), Fault::Unsupported(_)));
        assert!(matches!(fault_of_reason(&Reason::Fault(lumaxcode::Fault::Wedged("x".to_owned()))), Fault::Wedged(_)));
    }

    #[test]
    fn a_picture_starts_a_decode_by_its_sequence_header_its_group_or_its_type() {
        let picture = |coding: u8| [0, 0, 1, 0x00, 0x00, coding << 3, 0xFF, 0xF8];
        assert!(starts_a_decode(&[0, 0, 1, 0xB3, 0x78, 0x04, 0x38, 0x34, 0xFF, 0xFF]), "a sequence header");
        assert!(starts_a_decode(&[0xFF, 0, 0, 1, 0xB8, 0, 8, 0, 0x40]), "a group of pictures");
        assert!(starts_a_decode(&picture(1)), "an I picture");
        assert!(!starts_a_decode(&picture(2)), "a P picture");
        assert!(!starts_a_decode(&picture(3)), "a B picture");
        assert!(!starts_a_decode(&[0, 0, 1, 0x00]) && !starts_a_decode(&[0, 0, 1]) && !starts_a_decode(&[]));
        assert!(!starts_a_decode(&[0, 0, 1, 0x01, 0xAA, 0, 0, 1, 0x00, 0, 0x10]), "a slice start, then a P picture: the first start code that says is the picture's");
    }

    /// A job that answers as scripted.
    #[derive(Clone, Default)]
    struct Script(Arc<Mutex<Scripted>>);

    #[derive(Default)]
    struct Scripted {
        pushed: Vec<Au>,
        push_answer: Option<PushError>,
        ready: VecDeque<Out>,
        ended: Option<Reason>,
    }

    impl Session for Script {
        fn push(&self, au: Au) -> Result<(), PushError> {
            let mut s = self.0.lock();
            s.pushed.push(au);
            s.push_answer.take().map_or(Ok(()), Err)
        }

        fn pull(&self, out: &mut Vec<Out>) -> Result<(), Reason> {
            let mut s = self.0.lock();
            out.extend(s.ready.drain(..));
            s.ended.clone().map_or(Ok(()), Err)
        }
    }

    fn unit(pts: u64) -> Out {
        Out { pts, key: false, discontinuity: false, encode_tenths_ms: 90, data: vec![0, 0, 0, 1, 0x65] }
    }

    #[test]
    fn pictures_go_in_with_their_times_and_their_units_come_out_in_order() {
        let script = Script::default();
        let mut live = Live { job: script.clone(), units: Vec::new(), _room: None };
        live.push(&[0, 0, 1, 0xB3, 1, 2, 3], 90_000).unwrap();
        live.push(&[0, 0, 1, 0x00, 0x00, 0x18, 0xFF, 0xF8], 93_003).unwrap();
        {
            let s = script.0.lock();
            assert_eq!(s.pushed.iter().map(|a| (a.pts, a.key)).collect::<Vec<_>>(), [(90_000, true), (93_003, false)]);
            assert_eq!(s.pushed[0].data, [0, 0, 1, 0xB3, 1, 2, 3]);
        }
        script.0.lock().ready.extend([unit(90_000), unit(93_003)]);
        let mut out = vec![Unit { pts: 1, data: vec![] }];
        live.pull(&mut out).unwrap();
        assert_eq!(out.iter().map(|u| u.pts).collect::<Vec<_>>(), [1, 90_000, 93_003], "added to what is there");
        live.pull(&mut out).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn a_job_that_ends_gives_its_last_units_and_then_its_reason() {
        let script = Script::default();
        let mut live = Live { job: script.clone(), units: Vec::new(), _room: None };
        {
            let mut s = script.0.lock();
            s.ready.push_back(unit(7));
            s.ended = Some(Reason::Held("netflix is in front".to_owned()));
        }
        let mut out = Vec::new();
        assert!(matches!(live.pull(&mut out), Err(Fault::Unsupported(_))));
        assert_eq!(out.len(), 1, "what was ready was not lost");

        script.0.lock().push_answer = Some(PushError::WantKey);
        assert_eq!(live.push(&[0, 0, 1, 0x00, 0x00, 0x10], 1), Ok(()), "a picture before the first keyframe is dropped, not a failure");
        script.0.lock().push_answer = Some(PushError::Full);
        assert!(matches!(live.push(&[0, 0, 1, 0xB3], 2), Err(Fault::Stalled(_))));
        script.0.lock().push_answer = Some(PushError::Ended(Reason::Fault(lumaxcode::Fault::Failed("the encoder said no".to_owned()))));
        assert_eq!(live.push(&[0, 0, 1, 0xB3], 3), Err(Fault::Failed("the encoder said no".to_owned())));
    }
}
