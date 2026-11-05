//! What the tuner needs of an MPEG transport stream: the PAT, the PMT and ATSC's terrestrial virtual channel table read from whole
//! sections (the demux hands sections over whole), the one section it writes (a PAT that names a single program), and the 4-byte
//! timestamps the PVR download puts before every packet, taken off.

/// A transport stream packet.
pub const PACKET: usize = 188;
/// A PVR download unit: a 4-byte big-endian timestamp (27 MHz), then the packet. Measured on the set: the sync byte is at offset 4.
pub const UNIT: usize = 192;
pub const PID_PAT: u16 = 0x0000;
/// ATSC PSIP's base PID: the virtual channel table, which names the channels of a multiplex.
pub const PID_PSIP: u16 = 0x1FFB;
pub const TID_PAT: u8 = 0x00;
pub const TID_PMT: u8 = 0x02;
pub const TID_TVCT: u8 = 0xC8;
const SYNC: u8 = 0x47;

/// CRC-32/MPEG-2 (polynomial 0x04C11DB7, no reflection, no final xor). Over a whole section, its own CRC included, it is 0.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// A long-form section whose table id is `table_id`, whose length field fits and whose CRC holds: the section itself, trimmed to its length.
pub(crate) fn checked(section: &[u8], table_id: u8) -> Option<&[u8]> {
    if section.len() < 12 || section[0] != table_id {
        return None;
    }
    let len = (usize::from(section[1] & 0x0F) << 8) | usize::from(section[2]);
    let whole = section.get(..3 + len)?;
    (len >= 9 && crc32(whole) == 0).then_some(whole)
}

fn pid_of(hi: u8, lo: u8) -> u16 {
    u16::from_be_bytes([hi & 0x1F, lo])
}

/// The tags of a descriptor loop.
fn tags(mut d: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while d.len() >= 2 {
        out.push(d[0]);
        d = d.get(2 + usize::from(d[1])..).unwrap_or(&[]);
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pat {
    pub tsid: u16,
    /// (program number, PMT PID); the network PID (program 0) is not a program.
    pub programs: Vec<(u16, u16)>,
}

pub fn parse_pat(section: &[u8]) -> Option<Pat> {
    let s = checked(section, TID_PAT)?;
    let programs = s[8..s.len() - 4]
        .chunks_exact(4)
        .map(|e| (u16::from_be_bytes([e[0], e[1]]), pid_of(e[2], e[3])))
        .filter(|&(program, _)| program != 0)
        .collect();
    Some(Pat {
        tsid: u16::from_be_bytes([s[3], s[4]]),
        programs,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Es {
    pub stream_type: u8,
    pub pid: u16,
    pub tags: Vec<u8>,
    /// The descriptor loop as sent (a language, an AC-3 descriptor): what a PMT made for the same stream carries on.
    pub descriptors: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pmt {
    pub program: u16,
    pub pcr_pid: u16,
    /// The tags of the program-level descriptors.
    pub tags: Vec<u8>,
    /// The program-level descriptor loop as sent.
    pub descriptors: Vec<u8>,
    pub streams: Vec<Es>,
}

pub fn parse_pmt(section: &[u8]) -> Option<Pmt> {
    let s = checked(section, TID_PMT)?;
    let info = usize::from(u16::from_be_bytes([s[10] & 0x0F, s[11]]));
    let program_info = s.get(12..12 + info)?;
    let mut rest = s.get(12 + info..s.len() - 4)?;
    let mut streams = Vec::new();
    while rest.len() >= 5 {
        let n = usize::from(u16::from_be_bytes([rest[3] & 0x0F, rest[4]]));
        let d = rest.get(5..5 + n)?;
        streams.push(Es {
            stream_type: rest[0],
            pid: pid_of(rest[1], rest[2]),
            tags: tags(d),
            descriptors: d.to_vec(),
        });
        rest = &rest[5 + n..];
    }
    Some(Pmt {
        program: u16::from_be_bytes([s[3], s[4]]),
        pcr_pid: pid_of(s[8], s[9]),
        tags: tags(program_info),
        descriptors: program_info.to_vec(),
        streams,
    })
}

/// A PMT section of `version`: the program, its clock's PID, its descriptors, and its streams as (type, PID, descriptors).
pub fn build_pmt(program: u16, version: u8, pcr_pid: u16, descriptors: &[u8], streams: &[(u8, u16, &[u8])]) -> Vec<u8> {
    let mut s = vec![TID_PMT, 0xB0, 0, (program >> 8) as u8, program as u8, 0xC1 | ((version & 0x1F) << 1), 0, 0];
    s.extend_from_slice(&(0xE000 | pcr_pid).to_be_bytes());
    s.extend_from_slice(&(0xF000 | descriptors.len() as u16).to_be_bytes());
    s.extend_from_slice(descriptors);
    for (stream_type, pid, d) in streams {
        s.push(*stream_type);
        s.extend_from_slice(&(0xE000 | pid).to_be_bytes());
        s.extend_from_slice(&(0xF000 | d.len() as u16).to_be_bytes());
        s.extend_from_slice(d);
    }
    let len = s.len() - 3 + 4;
    s[1] = 0xB0 | (len >> 8) as u8;
    s[2] = len as u8;
    let crc = crc32(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

/// One virtual channel of a terrestrial virtual channel table: `2.4` is major 2, minor 4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    pub major: u16,
    pub minor: u16,
    pub program: u16,
    /// The channel's events may be scrambled.
    pub access_controlled: bool,
    pub hidden: bool,
    pub service_type: u8,
}

pub fn parse_tvct(section: &[u8]) -> Option<Vec<Channel>> {
    let s = checked(section, TID_TVCT)?;
    let n = usize::from(*s.get(9)?);
    let mut at = 10;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let e = s.get(at..at + 32)?;
        let name: Vec<u16> = e[..14]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        out.push(Channel {
            name: String::from_utf16_lossy(&name),
            major: (u16::from(e[14] & 0x0F) << 6) | u16::from(e[15] >> 2),
            minor: (u16::from(e[15] & 0x03) << 8) | u16::from(e[16]),
            program: u16::from_be_bytes([e[24], e[25]]),
            access_controlled: e[26] & 0x20 != 0,
            hidden: e[26] & 0x10 != 0,
            service_type: e[27] & 0x3F,
        });
        at += 32 + ((usize::from(e[30] & 0x03) << 8) | usize::from(e[31]));
    }
    Some(out)
}

/// Whether a program asks not to be passed on or is not in the clear: a conditional access descriptor (0x09) or ATSC's redistribution
/// control descriptor (0xAA), on the program or on one of its streams, or the channel's access-controlled bit. The tuner refuses it.
pub fn protected(pmt: &Pmt, channel: Option<&Channel>) -> bool {
    let flagged = |tags: &[u8]| tags.iter().any(|t| matches!(t, 0x09 | 0xAA));
    flagged(&pmt.tags)
        || pmt.streams.iter().any(|es| flagged(&es.tags))
        || channel.is_some_and(|c| c.access_controlled)
}

/// A PAT section that names one program (what an HDHomeRun's stream carries: the multiplex's other programs are not in it).
pub fn build_pat(tsid: u16, program: u16, pmt_pid: u16) -> Vec<u8> {
    let mut s = vec![TID_PAT, 0xB0, 13, (tsid >> 8) as u8, tsid as u8, 0xC1, 0, 0];
    s.extend_from_slice(&program.to_be_bytes());
    s.extend_from_slice(&(0xE000 | pmt_pid).to_be_bytes());
    let crc = crc32(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

/// The PID of a transport stream packet.
pub fn pid_of_packet(packet: &[u8]) -> u16 {
    pid_of(packet[1], packet[2])
}

/// Whether a packet of a picture stream holds the start of a picture that decodes on its own: one the multiplexer marks random access, or
/// one whose payload has an MPEG-2 sequence header (stream type 0x02), an H.264 SPS or IDR slice (0x1B), or an HEVC parameter set or IDR
/// slice (0x24). A start code split between two packets is not seen; the next picture's is.
pub fn access_point(packet: &[u8], stream_type: u8) -> bool {
    let control = (packet[3] >> 4) & 3;
    let mut at = 4;
    if control & 2 != 0 {
        let len = usize::from(packet[4]);
        if len > 0 && packet[5] & 0x40 != 0 {
            return true;
        }
        at += 1 + len;
    }
    if control & 1 == 0 || at >= packet.len() {
        return false;
    }
    packet[at..].windows(4).any(|w| {
        w[..3] == [0, 0, 1]
            && match stream_type {
                0x02 => w[3] == 0xB3,
                0x1B => matches!(w[3] & 0x1F, 5 | 7),
                0x24 => matches!((w[3] >> 1) & 0x3F, 19 | 20 | 32 | 33),
                _ => false,
            }
    })
}

/// Writes the sections of one PID as packets and keeps that PID's continuity count, for the packets it writes and for the ones it is
/// handed of the same PID.
#[derive(Debug, Default)]
pub struct SectionPackets {
    cc: u8,
}

impl SectionPackets {
    /// Appends the packets that carry `section`: the first starts it after a pointer field, the rest go on with it, the last is padded.
    pub fn write(&mut self, pid: u16, section: &[u8], out: &mut Vec<u8>) {
        let mut rest = section;
        let mut first = true;
        while first || !rest.is_empty() {
            let mut p = [0xFF; PACKET];
            p[0] = SYNC;
            p[1] = if first { 0x40 } else { 0 } | (pid >> 8) as u8;
            p[2] = pid as u8;
            p[3] = 0x10 | self.next();
            let at = if first {
                p[4] = 0;
                5
            } else {
                4
            };
            let n = rest.len().min(PACKET - at);
            p[at..at + n].copy_from_slice(&rest[..n]);
            rest = &rest[n..];
            first = false;
            out.extend_from_slice(&p);
        }
    }

    /// Gives a packet of this PID that came from elsewhere the next count, so the PID's packets read as one unbroken sequence (a packet
    /// with no payload does not count).
    pub fn renumber(&mut self, packet: &mut [u8]) {
        if packet[3] & 0x10 != 0 {
            packet[3] = (packet[3] & 0xF0) | self.next();
        }
    }

    fn next(&mut self) -> u8 {
        let cc = self.cc;
        self.cc = (cc + 1) & 0x0F;
        cc
    }
}

/// Takes the timestamps off the 192-byte units in `units` (whole units only; a partial one at the end is left out of `out` and is the
/// count that is returned beside it) and appends the packets to `out`. A unit whose sync byte is not where it belongs ends the copy: the
/// stream is not what was expected, and what follows cannot be trusted to be aligned.
pub fn strip_stamps(units: &[u8], out: &mut Vec<u8>) -> Result<usize, usize> {
    let mut taken = 0;
    for u in units.chunks_exact(UNIT) {
        if u[4] != SYNC {
            return Err(taken);
        }
        out.extend_from_slice(&u[4..]);
        taken += 1;
    }
    Ok(taken)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A section with a correct CRC.
    fn section(table_id: u8, tail: &[u8]) -> Vec<u8> {
        let len = tail.len() + 4;
        let mut s = vec![table_id, 0xB0 | ((len >> 8) as u8), len as u8];
        s.extend_from_slice(tail);
        let crc = crc32(&s);
        s.extend_from_slice(&crc.to_be_bytes());
        s
    }

    pub fn pat(tsid: u16, programs: &[(u16, u16)]) -> Vec<u8> {
        let mut tail = vec![(tsid >> 8) as u8, tsid as u8, 0xC1, 0, 0];
        for (n, pid) in programs {
            tail.extend_from_slice(&n.to_be_bytes());
            tail.extend_from_slice(&(0xE000 | pid).to_be_bytes());
        }
        section(TID_PAT, &tail)
    }

    pub fn pmt(
        program: u16,
        pcr: u16,
        program_tags: &[u8],
        streams: &[(u8, u16, &[u8])],
    ) -> Vec<u8> {
        let desc = |tags: &[u8]| tags.iter().flat_map(|t| [*t, 0]).collect::<Vec<u8>>();
        let pd = desc(program_tags);
        let mut tail = vec![
            (program >> 8) as u8,
            program as u8,
            0xC1,
            0,
            0,
            0xE0 | (pcr >> 8) as u8,
            pcr as u8,
            0xF0 | (pd.len() >> 8) as u8,
            pd.len() as u8,
        ];
        tail.extend(pd);
        for (ty, pid, tags) in streams {
            let d = desc(tags);
            tail.extend([
                *ty,
                0xE0 | (pid >> 8) as u8,
                *pid as u8,
                0xF0 | (d.len() >> 8) as u8,
                d.len() as u8,
            ]);
            tail.extend(d);
        }
        section(TID_PMT, &tail)
    }

    /// A terrestrial virtual channel table of one section: (name, major, minor, program, access_controlled).
    pub fn tvct(tsid: u16, channels: &[(&str, u16, u16, u16, bool)]) -> Vec<u8> {
        tvct_part(tsid, 0, 0, channels)
    }

    /// Section `number` of a table whose last section is `last`.
    pub fn tvct_part(
        tsid: u16,
        number: u8,
        last: u8,
        channels: &[(&str, u16, u16, u16, bool)],
    ) -> Vec<u8> {
        let mut tail = vec![
            (tsid >> 8) as u8,
            tsid as u8,
            0xC1,
            number,
            last,
            0xC0,
            channels.len() as u8,
        ];
        for (name, major, minor, program, ac) in channels {
            let mut n: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
            n.resize(14, 0);
            tail.extend(n);
            tail.extend([
                0xF0 | (major >> 6) as u8,
                ((major & 0x3F) << 2) as u8 | (minor >> 8) as u8,
                *minor as u8,
                0x04,
            ]);
            tail.extend([0, 0, 0, 0]);
            tail.extend(tsid.to_be_bytes());
            tail.extend(program.to_be_bytes());
            tail.extend([if *ac { 0x20 } else { 0 }, 0x02]);
            tail.extend([0, 0, 0xFC, 0]);
        }
        tail.extend([0xFC, 0]);
        section(TID_TVCT, &tail)
    }

    #[test]
    fn the_crc_of_a_whole_section_is_zero_and_a_changed_byte_shows() {
        let mut s = pat(739, &[(1, 0x30)]);
        assert_eq!(crc32(&s), 0);
        s[9] ^= 1;
        assert_eq!(parse_pat(&s), None);
    }

    #[test]
    fn a_pat_names_its_programs_but_not_the_network() {
        let got = parse_pat(&pat(739, &[(0, 0x10), (1, 0x30), (4, 0x60)])).unwrap();
        assert_eq!(
            got,
            Pat {
                tsid: 739,
                programs: vec![(1, 0x30), (4, 0x60)]
            }
        );
    }

    #[test]
    fn a_pmt_lists_streams_and_the_descriptors_that_matter() {
        let s = pmt(
            4,
            0x61,
            &[0x05, 0x87],
            &[
                (0x02, 0x61, &[0x11, 0x86]),
                (0x81, 0x64, &[0x05, 0x81, 0x0A]),
            ],
        );
        let got = parse_pmt(&s).unwrap();
        assert_eq!((got.program, got.pcr_pid), (4, 0x61));
        assert_eq!(got.tags, [0x05, 0x87]);
        assert_eq!(
            got.streams
                .iter()
                .map(|e| (e.stream_type, e.pid))
                .collect::<Vec<_>>(),
            [(0x02, 0x61), (0x81, 0x64)]
        );
        assert_eq!(got.streams[1].tags, [0x05, 0x81, 0x0A]);
        assert!(!protected(&got, None));
    }

    #[test]
    fn a_built_pmt_parses_back_with_the_descriptors_it_was_given() {
        let language = [0x0A, 4, b'e', b'n', b'g', 0];
        let registration = [0x05, 4, b'G', b'A', b'9', b'4'];
        let s = build_pmt(4, 3, 0x101, &registration, &[(0x02, 0x101, &[]), (0x81, 0x102, &language)]);
        assert_eq!(crc32(&s), 0);
        let got = parse_pmt(&s).unwrap();
        assert_eq!((got.program, got.pcr_pid), (4, 0x101));
        assert_eq!(got.descriptors, registration);
        assert_eq!(got.streams[1].descriptors, language);
        assert_eq!(s[5] >> 1 & 0x1F, 3, "the version");
    }

    #[test]
    fn a_ca_or_redistribution_descriptor_or_an_access_controlled_channel_is_protected() {
        let plain = parse_pmt(&pmt(1, 0x31, &[], &[(0x02, 0x31, &[])])).unwrap();
        let ca = parse_pmt(&pmt(1, 0x31, &[0x09], &[(0x02, 0x31, &[])])).unwrap();
        let rc = parse_pmt(&pmt(1, 0x31, &[], &[(0x02, 0x31, &[0xAA])])).unwrap();
        let locked = parse_tvct(&tvct(1, &[("X", 2, 1, 1, true)]))
            .unwrap()
            .remove(0);
        assert!(!protected(&plain, None));
        assert!(protected(&ca, None) && protected(&rc, None));
        assert!(protected(&plain, Some(&locked)));
    }

    #[test]
    fn a_virtual_channel_table_gives_numbers_names_and_programs() {
        let got = parse_tvct(&tvct(
            739,
            &[
                ("WSB-HD", 2, 1, 1, false),
                ("COURT", 17, 3, 5, false),
                ("WEST", 2, 4, 4, false),
            ],
        ))
        .unwrap();
        assert_eq!(
            got.iter()
                .map(|c| (c.name.as_str(), c.major, c.minor, c.program))
                .collect::<Vec<_>>(),
            [("WSB-HD", 2, 1, 1), ("COURT", 17, 3, 5), ("WEST", 2, 4, 4)]
        );
        assert!(
            got.iter()
                .all(|c| !c.access_controlled && !c.hidden && c.service_type == 2)
        );
    }

    #[test]
    fn a_built_pat_parses_back_and_fits_one_packet() {
        let s = build_pat(739, 4, 0x60);
        assert_eq!(
            parse_pat(&s),
            Some(Pat {
                tsid: 739,
                programs: vec![(4, 0x60)]
            })
        );
        let mut p = SectionPackets::default();
        let mut out = Vec::new();
        p.write(PID_PAT, &s, &mut out);
        p.write(PID_PAT, &s, &mut out);
        let (a, b) = out.split_at(PACKET);
        assert_eq!(b.len(), PACKET);
        assert_eq!(&a[..5], &[0x47, 0x40, 0x00, 0x10, 0x00]);
        assert_eq!(b[3], 0x11, "the continuity counter goes on");
        assert_eq!(&a[5..5 + s.len()], &s[..]);
        assert!(a[5 + s.len()..].iter().all(|&x| x == 0xFF));
    }

    #[test]
    fn a_long_section_goes_on_in_the_next_packet_and_passed_on_packets_keep_the_count() {
        let tags: Vec<u8> = (0..90).map(|i| 0x50 + i).collect();
        let s = pmt(4, 0x61, &tags, &[(0x02, 0x61, &[]), (0x81, 0x64, &[])]);
        assert!(s.len() > PACKET - 5);
        let mut p = SectionPackets::default();
        let mut out = Vec::new();
        p.write(0x60, &s, &mut out);
        assert_eq!(out.len(), 2 * PACKET);
        let (a, b) = out.split_at(PACKET);
        assert_eq!(
            (a[1], b[1], a[3], b[3]),
            (0x40, 0x00, 0x10, 0x11),
            "only the first starts the section"
        );
        let carried = [&a[5..], &b[4..]].concat();
        assert_eq!(parse_pmt(&carried), parse_pmt(&s));
        let mut theirs = [0x47, 0x40, 0x60, 0x17].to_vec();
        theirs.resize(PACKET, 0);
        p.renumber(&mut theirs);
        assert_eq!(theirs[3], 0x12);
        let mut no_payload = [0x47, 0x00, 0x60, 0x2C].to_vec();
        no_payload.resize(PACKET, 0);
        p.renumber(&mut no_payload);
        assert_eq!(no_payload[3], 0x2C, "adaptation only: not counted");
    }

    #[test]
    fn a_picture_that_decodes_on_its_own_is_told_by_its_start_code_or_its_random_access_mark() {
        let with = |head: &[u8], payload: &[u8]| {
            let mut p = [&[SYNC, 0x40, 0x61][..], head, payload].concat();
            p.resize(PACKET, 0xAB);
            p
        };
        let mpeg2 = |code: u8| with(&[0x10], &[0, 0, 1, 0xE0, 0, 0, 0x80, 0, 0, 0, 0, 1, code]);
        assert!(access_point(&mpeg2(0xB3), 0x02), "a sequence header");
        assert!(!access_point(&mpeg2(0x00), 0x02), "a picture alone");
        let h264 = |nal: u8| with(&[0x10], &[0, 0, 0, 1, 0x09, 0xF0, 0, 0, 0, 1, nal]);
        assert!(
            access_point(&h264(0x67), 0x1B) && access_point(&h264(0x65), 0x1B),
            "an SPS, an IDR slice"
        );
        assert!(!access_point(&h264(0x41), 0x1B), "another slice");
        assert!(
            access_point(&with(&[0x30, 1, 0x40], &[]), 0x02),
            "marked random access"
        );
        assert!(
            !access_point(&with(&[0x20, 1, 0x00], &[]), 0x02),
            "no payload, no mark"
        );
    }

    #[test]
    fn stamps_come_off_whole_units_only_and_a_misplaced_sync_byte_ends_the_copy() {
        let unit = |stamp: u32, first: u8| {
            let mut u = stamp.to_be_bytes().to_vec();
            u.push(first);
            u.resize(UNIT, 0);
            u
        };
        let mut data = [unit(1, SYNC), unit(2, SYNC)].concat();
        data.extend_from_slice(&[0, 0, 0]);
        let mut out = Vec::new();
        assert_eq!(strip_stamps(&data, &mut out), Ok(2));
        assert_eq!((out.len(), out[0], out[PACKET]), (2 * PACKET, SYNC, SYNC));
        let mut out = Vec::new();
        assert_eq!(
            strip_stamps(
                &[unit(1, SYNC), unit(2, 0), unit(3, SYNC)].concat(),
                &mut out
            ),
            Err(1)
        );
        assert_eq!(out.len(), PACKET);
    }
}
