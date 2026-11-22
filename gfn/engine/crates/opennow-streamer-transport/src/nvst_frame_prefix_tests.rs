use super::*;

fn start_of_picture(frame_index: u32) -> NvVideoPacket {
    NvVideoPacket {
        stream_packet_index: frame_index,
        frame_index,
        flags: FLAG_SOF | FLAG_EOF | FLAG_CONTAINS_PIC_DATA,
        fec_current_block: 0,
        fec_last_block: 0,
        is_fec: false,
    }
}

/// The prefix the TV's stream sends: `81 08 01 02`, a capture time, an extension of one `0x202`
/// record holding the access-unit length.
fn tv_prefix(capture: u32, access_unit: u32) -> Vec<u8> {
    let mut prefix = vec![0x81, 8, 1, 2];
    prefix.extend(capture.to_le_bytes());
    prefix.extend([8, 0, 0, 0, 2, 2, 4, 0]);
    prefix.extend(access_unit.to_le_bytes());
    prefix
}

/// The same prefix with a `0x200` sender-paced record added to its extension (25 bytes).
fn paced_prefix(capture: u32, access_unit: u32) -> Vec<u8> {
    let mut prefix = vec![0x81, 8, 1, 2];
    prefix.extend(capture.to_le_bytes());
    prefix.extend([13, 0, 0, 0, 2, 2, 4, 0]);
    prefix.extend(access_unit.to_le_bytes());
    prefix.extend([0, 2, 1, 0, 1]);
    prefix
}

fn with_codec_data(mut prefix: Vec<u8>, codec: &[u8]) -> Vec<u8> {
    prefix.extend(codec);
    prefix
}

fn recorded(feedback: &NvstFeedbackState) -> (Value, Vec<Value>) {
    let json = feedback.frame_prefixes_json(0);
    let rows = json["rows"].as_array().unwrap().clone();
    (json, rows)
}

fn column(json: &Value, name: &str) -> usize {
    json["fields"]
        .as_array()
        .unwrap()
        .iter()
        .position(|field| field == name)
        .unwrap()
}

fn assembled(assembler: &mut VideoAccessUnitAssembler, frame_index: u32, payload: &[u8]) -> String {
    let unit = assembler
        .push(start_of_picture(frame_index), 90_000, payload)
        .map(|unit| unit.map(|unit| (unit.bytes, unit.keyframe, unit.frame_index)));
    format!("{unit:?}")
}

#[test]
fn recording_the_prefix_changes_nothing_the_assembler_returns() {
    let annex_b = [0, 0, 0, 1, 0x67, 0xaa];
    let av1 = [0x12, 0x34, 0x56];
    let feedback = NvstFeedbackState::default();
    feedback.record_frame_prefixes();
    let mut plain_h264 = VideoAccessUnitAssembler::new(NvstVideoCodec::H264, 4096);
    let mut recording_h264 = VideoAccessUnitAssembler::new(NvstVideoCodec::H264, 4096)
        .recording_prefixes(Arc::clone(&feedback.frame_prefixes));
    let mut plain_av1 = VideoAccessUnitAssembler::new(NvstVideoCodec::Av1, 4096);
    let mut recording_av1 = VideoAccessUnitAssembler::new(NvstVideoCodec::Av1, 4096)
        .recording_prefixes(Arc::clone(&feedback.frame_prefixes));

    let h264_pictures = [
        with_codec_data(tv_prefix(1_000, 6), &annex_b),
        with_codec_data(paced_prefix(2_000, 6), &annex_b),
        [&[0x01, 0, 0, 2, 0, 0, 0, 0][..], &[0, 0, 1, 0x67, 0xaa][..]].concat(),
        vec![0x81; 76],
    ];
    for (index, payload) in (1..).zip(&h264_pictures) {
        assert_eq!(
            assembled(&mut recording_h264, index, payload),
            assembled(&mut plain_h264, index, payload),
            "H.264 picture {index}"
        );
    }
    let av1_picture = with_codec_data(tv_prefix(3_000, 3), &av1);
    assert_eq!(
        assembled(&mut recording_av1, 5, &av1_picture),
        assembled(&mut plain_av1, 5, &av1_picture),
        "AV1 picture"
    );

    let (json, rows) = recorded(&feedback);
    let counts = &json["counts"];
    assert_eq!(
        (
            counts["seen"].clone(),
            counts["read"].clone(),
            counts["absent"].clone(),
            counts["malformed"].clone()
        ),
        (json!(5), json!(3), json!(1), json!(1))
    );
    assert_eq!(rows.len(), 5);
}

#[test]
fn a_row_carries_the_frame_the_timestamp_and_what_the_receiver_strips() {
    let feedback = NvstFeedbackState::default();
    feedback.record_frame_prefixes();
    let mut assembler = VideoAccessUnitAssembler::new(NvstVideoCodec::H264, 4096)
        .recording_prefixes(Arc::clone(&feedback.frame_prefixes));
    let payload = with_codec_data(tv_prefix(0x1122_3344, 6), &[0, 0, 0, 1, 0x67, 0xaa]);
    assembler
        .push(start_of_picture(77), 123_456, &payload)
        .expect("a picture");

    let (json, rows) = recorded(&feedback);
    let row = &rows[0];
    for (name, expected) in [
        ("frame_index", json!(77)),
        ("rtp_timestamp", json!(123_456)),
        ("status", json!("read")),
        ("capture_time_us", json!(0x1122_3344)),
        ("access_unit_length", json!(6)),
        ("sender_paced", Value::Null),
        ("presentation_time_us", Value::Null),
        ("prefix_bytes", json!(20)),
        ("stripped_bytes", json!(20)),
    ] {
        assert_eq!(row[column(&json, name)], expected, "{name}");
    }
}

#[test]
fn a_prefix_longer_than_the_av1_skip_shows_as_two_different_lengths() {
    let feedback = NvstFeedbackState::default();
    feedback.record_frame_prefixes();
    let log = Arc::clone(&feedback.frame_prefixes);
    let mut av1 = VideoAccessUnitAssembler::new(NvstVideoCodec::Av1, 4096)
        .recording_prefixes(Arc::clone(&log));
    let mut h265 =
        VideoAccessUnitAssembler::new(NvstVideoCodec::H265, 4096).recording_prefixes(log);
    // The pinned receiver skips a fixed 20 bytes of an AV1 `0x81` prefix; the prefix declares 25.
    let _ = av1.push(
        start_of_picture(1),
        90_000,
        &with_codec_data(paced_prefix(1, 3), &[0x12, 0x34, 0x56]),
    );
    // For H.265 it looks for the start code, which finds the end of the declared prefix.
    h265.push(
        start_of_picture(2),
        90_000,
        &with_codec_data(paced_prefix(2, 3), &[0, 0, 0, 1, 19 << 1, 1, 0xaa]),
    )
    .expect("an H.265 picture");

    let (json, rows) = recorded(&feedback);
    let paced = column(&json, "sender_paced");
    let declared = column(&json, "prefix_bytes");
    let stripped = column(&json, "stripped_bytes");
    assert_eq!(
        (
            rows[0][paced].clone(),
            rows[0][declared].clone(),
            rows[0][stripped].clone()
        ),
        (json!(1), json!(25), json!(20)),
        "AV1"
    );
    assert_eq!(
        (
            rows[1][paced].clone(),
            rows[1][declared].clone(),
            rows[1][stripped].clone()
        ),
        (json!(1), json!(25), json!(25)),
        "H.265"
    );
}

#[test]
fn a_start_packet_without_codec_data_is_recorded_before_it_is_refused() {
    let feedback = NvstFeedbackState::default();
    feedback.record_frame_prefixes();
    let mut assembler = VideoAccessUnitAssembler::new(NvstVideoCodec::H264, 4096)
        .recording_prefixes(Arc::clone(&feedback.frame_prefixes));
    let payload = with_codec_data(tv_prefix(9, 3), &[5, 5, 5]);
    assert!(matches!(
        assembler.push(start_of_picture(3), 90_000, &payload),
        Err(NvstDropReason::MissingAnnexBStartCode)
    ));

    let (json, rows) = recorded(&feedback);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][column(&json, "status")], "read");
    assert_eq!(rows[0][column(&json, "capture_time_us")], 9);
    assert_eq!(rows[0][column(&json, "stripped_bytes")], Value::Null);
}

#[test]
fn only_the_first_packet_of_a_picture_is_recorded() {
    let feedback = NvstFeedbackState::default();
    feedback.record_frame_prefixes();
    let mut assembler = VideoAccessUnitAssembler::new(NvstVideoCodec::H264, 4096)
        .recording_prefixes(Arc::clone(&feedback.frame_prefixes));
    let first = NvVideoPacket {
        flags: FLAG_SOF | FLAG_CONTAINS_PIC_DATA,
        ..start_of_picture(8)
    };
    let last = NvVideoPacket {
        stream_packet_index: 9,
        flags: FLAG_EOF | FLAG_CONTAINS_PIC_DATA,
        ..start_of_picture(8)
    };
    let payload = with_codec_data(tv_prefix(1, 6), &[0, 0, 0, 1, 0x67]);
    assert!(assembler.push(first, 90_000, &payload).unwrap().is_none());
    assert!(assembler.push(last, 90_000, &[0xaa]).unwrap().is_some());
    assert_eq!(recorded(&feedback).1.len(), 1);
}

#[test]
fn the_receiver_records_into_the_feedback_it_was_configured_with_once_armed() {
    let config = config();
    let feedback = config.feedback();
    let crypto = test_srtp(&config);
    let media = with_codec_data(tv_prefix(0xabcd, 5), &[0, 0, 0, 1, 0x65]);
    let packet = |sequence, frame_index| {
        protect_for_test(
            &crypto,
            build_plaintext_rtp(
                sequence,
                FLAG_SOF | FLAG_EOF | FLAG_CONTAINS_PIC_DATA,
                frame_index,
                &media,
            ),
            0,
        )
    };
    let mut receiver = NvstVideoReceiver::new(config);
    let origin = Instant::now();

    let events = receiver.process_datagram(peer(), &packet(10, 9), origin);
    assert!(matches!(events[..], [NvstReceiveEvent::Frame(_)]));
    assert_eq!(
        feedback.frame_prefixes_json(0)["counts"]["seen"],
        0,
        "not armed yet"
    );

    feedback.record_frame_prefixes();
    let events = receiver.process_datagram(peer(), &packet(11, 10), origin);
    assert!(matches!(events[..], [NvstReceiveEvent::Frame(_)]));
    let (json, rows) = recorded(&feedback);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][column(&json, "frame_index")], 10);
    assert_eq!(rows[0][column(&json, "rtp_timestamp")], 0x0102_0304);
    assert_eq!(rows[0][column(&json, "capture_time_us")], 0xabcd);
}
