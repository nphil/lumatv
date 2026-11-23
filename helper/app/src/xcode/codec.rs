//! The Live TV Tuner's converter over the engine: a station's MPEG-2 pictures in, the same pictures as H.264 out.
//!
//! One conversion is one `Priority::Live` job of the engine, fed by the stream's pump (`push` and `pull` never wait). The engine's faults are
//! the tuner's own words: what is not for this picture or for now (an app has the video hardware, a size the child cannot make) is
//! `Unsupported`, which costs the converter nothing; what the child did wrong is `Failed` or `Stalled`, which rests it; a call into the
//! hardware that never came back is `Wedged`, which turns it off until the helper restarts.

use super::gate::Look;
use lumatuner::mpeg2::Format;
use lumatuner::{Codec, Convert, Fault, Unit, Want};
use lumaxcode::{Au, Engine, H264Profile, JobSpec, Out, Output, Priority, PushError, Reason, Sink, Source, SourceCodec, StartError, Wait};

/// How far the shape of the picture the child will make may be from the shape the station signals before it is not made: an anamorphic
/// picture comes out with square pixels at its coded size (the encoder's stream carries no pixel aspect and the child never enlarges), so
/// 16:9 in 704x480 would show 17% too narrow, where 4:3 in 704x480 shows 10% too wide.
const SHAPE_TOLERANCE: f32 = 0.15;

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
        let output = output_of(source, want)?;
        // the media server is asked once before the hardware is: a decoder an app took a moment ago closes the gate here
        self.look.now();
        let spec = JobSpec {
            owner: "tuner".to_owned(),
            priority: Priority::Live,
            source: Source::Units { codec: SourceCodec::Mpeg2, width: u32::from(source.width), height: u32::from(source.height) },
            output,
            wait: Wait::Fail,
            sink: Sink::Deliver,
        };
        match self.engine.start(spec) {
            Ok(job) => Ok(Box::new(Live { job, units: Vec::new() })),
            Err(e) => Err(fault_of_start(e)),
        }
    }
}

/// What the child is asked to make for `want` from pictures of `source`: its size when that is smaller than the picture, every Nth picture
/// of a rate above the one wanted, a keyframe every two seconds, constant bit rate. Refused (`Unsupported`) when the child would not make the
/// shape the station signals.
fn output_of(source: &Format, want: &Want) -> Result<Output, Fault> {
    let (num, den) = source.frame_rate();
    let from = num as f64 / den as f64;
    let to = f64::from(want.rate.0) / f64::from(want.rate.1.max(1));
    let keep_every = ((from / to).round() as u32).max(1);
    let fps = to.round().clamp(1.0, 60.0) as u32;
    // the child makes the smaller of the picture and what is wanted in each direction, and none of it when nothing is smaller
    let (width, height) = (want.width.min(source.width), want.height.min(source.height));
    let shape = f32::from(width) / f32::from(height.max(1));
    let signalled = source.display_width() / f32::from(source.height.max(1));
    if (shape - signalled).abs() > SHAPE_TOLERANCE * signalled {
        return Err(Fault::Unsupported(format!(
            "a {}x{} picture would come out {width}x{height}, in the wrong shape: the converter does not enlarge or signal a pixel aspect",
            source.width, source.height
        )));
    }
    let whole = width == source.width && height == source.height;
    Ok(Output {
        width: if whole { 0 } else { u32::from(width) },
        height: if whole { 0 } else { u32::from(height) },
        keep_every,
        fps,
        kbps: want.kbps,
        cbr: true,
        gop: fps * 2,
        profile: H264Profile::High,
    })
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
    use lumatuner::Profile;
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
        let heavy = output_of(&hd, &Profile::Heavy.want(&hd)).unwrap();
        assert_eq!((sized(&heavy), heavy.keep_every, heavy.fps, heavy.kbps, heavy.gop), ((0, 0), 1, 30, 9000, 60), "1080 is made whole, with no scale");

        let mobile = output_of(&hd, &Profile::Mobile.want(&hd)).unwrap();
        assert_eq!((sized(&mobile), mobile.keep_every, mobile.fps, mobile.kbps), ((1280, 720), 1, 30, 3500));
        assert!(mobile.cbr && mobile.profile == H264Profile::High);

        let p60 = station(1280, 720, 7, 3);
        let mobile = output_of(&p60, &Profile::Mobile.want(&p60)).unwrap();
        assert_eq!((sized(&mobile), mobile.keep_every, mobile.fps), ((0, 0), 2, 30), "720p60 is every other picture, unscaled");
        let heavy = output_of(&p60, &Profile::Heavy.want(&p60)).unwrap();
        assert_eq!((heavy.keep_every, heavy.fps, heavy.gop), (1, 60, 120));

        let small = output_of(&hd, &Profile::Internet240.want(&hd)).unwrap();
        assert_eq!((sized(&small), small.kbps), ((432, 240), 400));
        let film = station(1920, 1080, 1, 3);
        assert_eq!(output_of(&film, &Profile::Mobile.want(&film)).unwrap().fps, 24);
    }

    #[test]
    fn a_shape_the_child_would_get_wrong_is_not_made() {
        // 16:9 in the two standard-definition frames, which are both close to 3:2
        for wide_sd in [station(704, 480, 4, 3), station(720, 480, 4, 3)] {
            for profile in [Profile::Heavy, Profile::Mobile, Profile::Internet480] {
                let Err(Fault::Unsupported(why)) = output_of(&wide_sd, &profile.want(&wide_sd)) else {
                    panic!("{profile:?}: 16:9 in {}x480 needs enlarging", wide_sd.width);
                };
                assert!(why.contains("wrong shape"), "{why}");
            }
        }
        let anamorphic_hd = station(1440, 1080, 4, 3);
        assert!(output_of(&anamorphic_hd, &Profile::Heavy.want(&anamorphic_hd)).is_err());
        assert_eq!(sized(&output_of(&anamorphic_hd, &Profile::Mobile.want(&anamorphic_hd)).unwrap()), (1280, 720), "scaled down it is square");
        let sd = station(704, 480, 4, 2);
        assert!(output_of(&sd, &Profile::Heavy.want(&sd)).is_ok(), "4:3 in 704x480 is 10% wide, which is let through");
        assert_eq!(sized(&output_of(&sd, &Profile::Mobile.want(&sd)).unwrap()), (640, 480));
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
        let mut live = Live { job: script.clone(), units: Vec::new() };
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
        let mut live = Live { job: script.clone(), units: Vec::new() };
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
