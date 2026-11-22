//! Shared by the session tests: the real feeder (gfn/feeder, host build, `sh gfn/feeder/build.sh` first) with the NDL stand-in, and HEVC
//! access units whose parameter sets carry real colour tags.
#![allow(dead_code)] // each test binary uses part of it
use std::path::{Path, PathBuf};

use opennow_streamer_platform::{MediaColorQuality, MediaStreamConfig, MediaVideoCodec, StreamShortcutBindings};

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..").canonicalize().unwrap()
}

/// Points the runtime at the real feeder and the NDL stand-in, with the pad directories under `tmp`. Returns the stand-in's log.
pub fn feeder_env(tmp: &Path) -> PathBuf {
    let lib_dir = std::env::var_os("LUMATV_GFN_TEST_FEEDER_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("gfn/feeder/build/host"));
    let feeder = lib_dir.join("lumatv-gfn-feeder");
    assert!(feeder.exists() && lib_dir.join("libNDL_directmedia.so.1").exists(), "build the feeder first (sh gfn/feeder/build.sh)");
    let ndl_log = tmp.join("ndl.log");
    unsafe {
        std::env::set_var("LUMATV_GFN_FEEDER_SOCKET", tmp.join("feed.sock"));
        std::env::set_var("LUMATV_GFN_FEEDER_SPAWN", &feeder);
        std::env::set_var("LUMATV_GFN_FEEDER_ARGS", format!("--once --window-id _Window_Id_42 -L {}", lib_dir.join("libNDL_directmedia.so.1").display()));
        std::env::set_var("LD_LIBRARY_PATH", &lib_dir);
        std::env::set_var("FAKE_NDL_LOG", &ndl_log);
        std::env::set_var("FAKE_NDL_VERBOSE", "1");
        std::env::set_var("LUMATV_GFN_PAD_DEV", tmp.join("dev/input"));
        std::env::set_var("LUMATV_GFN_PAD_SYS", tmp.join("sys/class/input"));
    }
    ndl_log
}

/// A 4K 120 fps 10-bit HEVC stream whose session answered `hdr`.
pub fn stream(hdr: bool) -> MediaStreamConfig {
    MediaStreamConfig {
        codec: MediaVideoCodec::H265,
        color_quality: MediaColorQuality::TenBit420,
        hdr,
        width: 3840,
        height: 2160,
        fps: 120,
        bitrate_bps: 80_000_000,
        cloud_gsync: false,
        shortcuts: StreamShortcutBindings::default(),
    }
}

/// x265 parameter sets, as the tests of the platform's `colour.rs` read them. The SPS differs only in its VUI: primaries, transfer
/// characteristic and matrix, as H.273 code points.
const VPS: &str = "40010c01ffff02200000030090000003000003003c959809";
const PPS: &str = "4401c172b46240";
/// PQ: 9, 16, 9.
pub const SPS_PQ: &str = "42010102200000030090000003000003003ca00a080b9f6d96566924caf016a122012080000003008000000f04";
/// HLG: 9, 18, 9.
pub const SPS_HLG: &str = "42010102200000030090000003000003003ca00a080b9f6d96566924caf016a122412080000003008000000f04";
/// BT.709: 1, 1, 1.
pub const SPS_BT709: &str = "42010101600000030090000003000003003ca00a080b9f796566924caf016a02020208000003000800000300f040";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// One access unit of `size` bytes. A keyframe carries the parameter sets, and `sps` is the one it carries.
pub fn hevc_unit(key: bool, size: usize, sps: &str) -> Vec<u8> {
    let mut slice = vec![0, 0, 0, 1, if key { 19 << 1 } else { 1 << 1 }, 1, 0x80];
    slice.resize(size, 0x5a);
    if !key {
        return slice;
    }
    let mut unit: Vec<u8> = [VPS, sps, PPS].iter().flat_map(|ps| [vec![0, 0, 0, 1], hex(ps)].concat()).collect();
    unit.extend_from_slice(&slice);
    unit
}
