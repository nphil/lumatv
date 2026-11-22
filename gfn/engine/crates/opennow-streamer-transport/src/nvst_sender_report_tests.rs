use super::*;

/// A sender report as the video socket sees SRTCP: the clear header and sender SSRC, then bytes
/// standing in for the encrypted rest.
fn srtcp_sender_report(ssrc: u32) -> Vec<u8> {
    let mut packet = vec![0x80, 200, 0, 6];
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.resize(28, 0xa5);
    packet
}

fn audio_track(ssrc: Option<u32>) -> NvstAudioTrack {
    NvstAudioTrack {
        payload_type: GFN_OPUS_PAYLOAD_TYPE,
        clock_rate_hz: 48_000,
        channels: 2,
        mid: "audio".to_owned(),
        ssrc,
    }
}

fn heard(json: &Value, track: &str) -> Option<u64> {
    json[track]["wire"]["count"].as_u64()
}

#[test]
fn a_report_belongs_to_the_handed_off_the_bound_or_the_audio_stream_and_to_no_other() {
    let feedback = NvstFeedbackState::default();
    let at = Instant::now();
    // Nothing expected or bound yet, so the sender is nobody's.
    feedback.observe_sender_report(None, None, 5, None, at);
    // The handoff names the video SSRC before any media has arrived.
    feedback.observe_sender_report(Some(9), None, 9, None, at);
    // The video path has bound the SSRC it authenticated.
    feedback.video_ssrc.store(5, Ordering::Release);
    feedback.observe_sender_report(None, None, 5, None, at);
    feedback.observe_sender_report(None, Some(48_000), 6, None, at);
    feedback.observe_sender_report(None, None, 7, None, at);

    let json = feedback.sender_reports_json(0);
    assert_eq!(
        (
            heard(&json, "video"),
            heard(&json, "audio"),
            heard(&json, "unmapped")
        ),
        (Some(2), Some(1), Some(2))
    );
    assert_eq!(json["video"]["clock_hz"], 90_000);
    assert_eq!(json["audio"]["clock_hz"], 48_000);
    assert_eq!(json["unmapped"]["ssrc"], 7);
}

#[test]
fn the_audio_clock_is_given_only_for_streams_of_the_audio_track() {
    let mut streams = NvstAudioStreams::default();
    streams.authenticated.push_back(30);
    streams.provisional = Some(31);
    let named = audio_track(Some(20));
    let unnamed = audio_track(None);
    assert_eq!(
        audio_clock_of(Some(&named), &streams, 20),
        Some(48_000),
        "named by the handoff"
    );
    assert_eq!(
        audio_clock_of(Some(&unnamed), &streams, 30),
        Some(48_000),
        "authenticated"
    );
    assert_eq!(
        audio_clock_of(Some(&unnamed), &streams, 31),
        Some(48_000),
        "admitted"
    );
    assert_eq!(audio_clock_of(Some(&unnamed), &streams, 32), None);
    assert_eq!(
        audio_clock_of(None, &streams, 30),
        None,
        "no audio track was negotiated"
    );
}

#[test]
fn a_sender_report_on_the_video_socket_is_heard_and_the_stream_goes_on() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    let client_address = client.local_addr().unwrap();
    let mut config = send_fault_config(&server);
    config.expected_ssrc = None;
    let feedback = config.feedback();
    let srtp = test_srtp(&config);
    let frame = |sequence: u16| {
        protect_for_test(
            &srtp,
            build_plaintext_rtp(
                sequence,
                FLAG_SOF | FLAG_EOF | FLAG_CONTAINS_PIC_DATA,
                41 + u32::from(sequence),
                &[0, 0, 1, 0x65],
            ),
            0,
        )
    };
    // The first frame binds the video SSRC. The reports sit between it and the second frame, so
    // that frame arriving shows the receiver went on past them.
    for datagram in [
        frame(1),
        srtcp_sender_report(0x1122_3344),
        srtcp_sender_report(0x0a0b_0c0d),
        frame(2),
    ] {
        server.send_to(&datagram, client_address).unwrap();
    }
    let (media_consumer, media_receiver) = mpsc::sync_channel(4);
    let (event_sender, _events) = mpsc::channel();
    let session = spawn_video_with_send_faults(
        client,
        config,
        media_consumer,
        event_sender,
        |socket, packet, peer| socket.send_to(packet, peer),
        |socket, packet, peer| socket.send_to(packet, peer),
    );
    for index in [42, 43] {
        let frame = media_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("video continues past the reports");
        assert_eq!(frame.frame_index, Some(index));
    }

    let json = feedback.sender_reports_json(0);
    assert_eq!(
        (heard(&json, "video"), heard(&json, "unmapped")),
        (Some(1), Some(1))
    );
    assert_eq!(
        (
            json["video"]["ssrc"].as_u64(),
            json["unmapped"]["ssrc"].as_u64()
        ),
        (Some(0x1122_3344), Some(0x0a0b_0c0d))
    );
    assert_eq!(json["video"]["read"]["count"], 0, "the times are encrypted");
    session.stop();
}
