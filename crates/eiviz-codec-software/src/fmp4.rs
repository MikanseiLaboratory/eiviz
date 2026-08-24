use crate::h264;
use eiviz_media::{EncodedAccessUnit, EncodedKind, EncodedStreamConfig};

pub struct FragmentedMp4 {
    pub bytes: Vec<u8>,
    sequence: u32,
    config: EncodedStreamConfig,
    include_audio: bool,
}

impl FragmentedMp4 {
    /// Video-only constructor retained for codec unit tests.
    pub fn new(timescale: u32, width: u16, height: u16, sps: &[u8], pps: &[u8]) -> Self {
        Self::new_inner(
            EncodedStreamConfig {
                h264_sps: sps.to_vec().into(),
                h264_pps: pps.to_vec().into(),
                aac_audio_specific_config: Vec::new().into(),
                video_width: width,
                video_height: height,
                video_timescale: timescale,
                video_sample_duration: 1,
                audio_sample_rate: 48_000,
                audio_channels: 2,
            },
            false,
        )
    }

    pub fn new_av(config: EncodedStreamConfig) -> Self {
        Self::new_inner(config, true)
    }

    fn new_inner(config: EncodedStreamConfig, include_audio: bool) -> Self {
        let mut mux = Self {
            bytes: Vec::new(),
            sequence: 1,
            config,
            include_audio,
        };
        mux.bytes.extend_from_slice(&file_type());
        mux.bytes.extend_from_slice(&mux.movie());
        mux
    }

    /// Append one independently decodable movie fragment.
    pub fn write_sample(&mut self, access_unit: &EncodedAccessUnit, duration: u32) {
        let (track_id, timescale, payload, sample_flags) = match access_unit.kind {
            EncodedKind::Avc => {
                let mut payload = Vec::new();
                for nal in h264::split_annexb(&access_unit.bytes) {
                    if nal.is_empty() || matches!(nal[0] & 0x1f, 7 | 8) {
                        continue;
                    }
                    payload.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    payload.extend_from_slice(&nal);
                }
                let flags = if access_unit.keyframe {
                    0x0200_0000
                } else {
                    0x0101_0000
                };
                (1, self.config.video_timescale, payload, flags)
            }
            EncodedKind::Aac if self.include_audio => (
                2,
                self.config.audio_sample_rate,
                access_unit.bytes.to_vec(),
                0x0200_0000,
            ),
            _ => return,
        };
        if payload.is_empty() {
            return;
        }

        let decode_time = media_time_units(
            access_unit.dts.unwrap_or(access_unit.pts),
            timescale,
        );
        let fragment = movie_fragment(
            self.sequence,
            track_id,
            decode_time,
            duration,
            payload.len() as u32,
            sample_flags,
        );
        self.bytes.extend_from_slice(&fragment);
        self.bytes.extend_from_slice(&bx(b"mdat", &payload));
        self.sequence = self.sequence.saturating_add(1);
    }

    fn movie(&self) -> Vec<u8> {
        let mut body = movie_header();
        body.extend_from_slice(&track(
            1,
            self.config.video_timescale,
            self.config.video_width,
            self.config.video_height,
            video_sample_entry(&self.config),
            true,
        ));
        if self.include_audio {
            body.extend_from_slice(&track(
                2,
                self.config.audio_sample_rate,
                0,
                0,
                audio_sample_entry(&self.config),
                false,
            ));
        }
        let mut mvex = trex(1, self.config.video_sample_duration);
        if self.include_audio {
            mvex.extend_from_slice(&trex(2, 1024));
        }
        body.extend_from_slice(&bx(b"mvex", &mvex));
        bx(b"moov", &body)
    }
}

fn file_type() -> Vec<u8> {
    let mut body = b"isom".to_vec();
    body.extend_from_slice(&0x200u32.to_be_bytes());
    body.extend_from_slice(b"isomiso6mp41avc1");
    bx(b"ftyp", &body)
}

fn movie_header() -> Vec<u8> {
    let mut body = vec![0; 8];
    body.extend_from_slice(&1_000u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    body.extend_from_slice(&0x0100u16.to_be_bytes());
    body.extend_from_slice(&[0; 10]);
    append_matrix(&mut body);
    body.extend_from_slice(&[0; 24]);
    body.extend_from_slice(&3u32.to_be_bytes());
    full(b"mvhd", 0, 0, &body)
}

fn track(
    id: u32,
    timescale: u32,
    width: u16,
    height: u16,
    sample_entry: Vec<u8>,
    video: bool,
) -> Vec<u8> {
    let mut tkhd = vec![0; 8];
    tkhd.extend_from_slice(&id.to_be_bytes());
    tkhd.extend_from_slice(&0u32.to_be_bytes());
    tkhd.extend_from_slice(&0u32.to_be_bytes());
    tkhd.extend_from_slice(&[0; 8]);
    tkhd.extend_from_slice(&0u16.to_be_bytes());
    tkhd.extend_from_slice(&0u16.to_be_bytes());
    tkhd.extend_from_slice(&(if video { 0 } else { 0x0100u16 }).to_be_bytes());
    tkhd.extend_from_slice(&0u16.to_be_bytes());
    append_matrix(&mut tkhd);
    tkhd.extend_from_slice(&((width as u32) << 16).to_be_bytes());
    tkhd.extend_from_slice(&((height as u32) << 16).to_be_bytes());

    let mut mdhd = vec![0; 8];
    mdhd.extend_from_slice(&timescale.to_be_bytes());
    mdhd.extend_from_slice(&0u32.to_be_bytes());
    mdhd.extend_from_slice(&0x55c4u16.to_be_bytes());
    mdhd.extend_from_slice(&0u16.to_be_bytes());
    let mut mdia = full(b"mdhd", 0, 0, &mdhd);

    let mut hdlr = vec![0; 4];
    hdlr.extend_from_slice(if video { b"vide" } else { b"soun" });
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.extend_from_slice(if video { b"eiviz video\0" } else { b"eiviz audio\0" });
    mdia.extend_from_slice(&full(b"hdlr", 0, 0, &hdlr));

    let mut minf = if video {
        full(b"vmhd", 0, 1, &[0; 8])
    } else {
        full(b"smhd", 0, 0, &[0; 4])
    };
    let url = full(b"url ", 0, 1, &[]);
    let mut dref = 1u32.to_be_bytes().to_vec();
    dref.extend_from_slice(&url);
    minf.extend_from_slice(&bx(b"dinf", &full(b"dref", 0, 0, &dref)));
    minf.extend_from_slice(&sample_table(sample_entry));
    mdia.extend_from_slice(&bx(b"minf", &minf));

    let mut body = full(b"tkhd", 0, 7, &tkhd);
    body.extend_from_slice(&bx(b"mdia", &mdia));
    bx(b"trak", &body)
}

fn sample_table(sample_entry: Vec<u8>) -> Vec<u8> {
    let mut stsd = 1u32.to_be_bytes().to_vec();
    stsd.extend_from_slice(&sample_entry);
    let mut body = full(b"stsd", 0, 0, &stsd);
    body.extend_from_slice(&full(b"stts", 0, 0, &0u32.to_be_bytes()));
    body.extend_from_slice(&full(b"stsc", 0, 0, &0u32.to_be_bytes()));
    body.extend_from_slice(&full(b"stsz", 0, 0, &[0; 8]));
    body.extend_from_slice(&full(b"stco", 0, 0, &0u32.to_be_bytes()));
    bx(b"stbl", &body)
}

fn video_sample_entry(config: &EncodedStreamConfig) -> Vec<u8> {
    let mut avc1 = vec![0; 6];
    avc1.extend_from_slice(&1u16.to_be_bytes());
    avc1.extend_from_slice(&[0; 16]);
    avc1.extend_from_slice(&config.video_width.to_be_bytes());
    avc1.extend_from_slice(&config.video_height.to_be_bytes());
    avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    avc1.extend_from_slice(&0u32.to_be_bytes());
    avc1.extend_from_slice(&1u16.to_be_bytes());
    avc1.extend_from_slice(&[0; 32]);
    avc1.extend_from_slice(&0x0018u16.to_be_bytes());
    avc1.extend_from_slice(&0xffffu16.to_be_bytes());
    avc1.extend_from_slice(&bx(
        b"avcC",
        &avcc(&config.h264_sps, &config.h264_pps),
    ));
    bx(b"avc1", &avc1)
}

fn audio_sample_entry(config: &EncodedStreamConfig) -> Vec<u8> {
    let mut mp4a = vec![0; 6];
    mp4a.extend_from_slice(&1u16.to_be_bytes());
    mp4a.extend_from_slice(&[0; 8]);
    mp4a.extend_from_slice(&config.audio_channels.to_be_bytes());
    mp4a.extend_from_slice(&16u16.to_be_bytes());
    mp4a.extend_from_slice(&0u16.to_be_bytes());
    mp4a.extend_from_slice(&0u16.to_be_bytes());
    mp4a.extend_from_slice(&(config.audio_sample_rate << 16).to_be_bytes());
    mp4a.extend_from_slice(&esds(&config.aac_audio_specific_config));
    bx(b"mp4a", &mp4a)
}

fn esds(audio_specific_config: &[u8]) -> Vec<u8> {
    // ES_Descriptor -> DecoderConfigDescriptor -> DecoderSpecificInfo.
    let mut decoder_specific = vec![0x05, audio_specific_config.len() as u8];
    decoder_specific.extend_from_slice(audio_specific_config);
    let mut decoder = vec![0x04, (13 + decoder_specific.len()) as u8, 0x40, 0x15];
    decoder.extend_from_slice(&[0, 0, 0]); // bufferSizeDB
    decoder.extend_from_slice(&0u32.to_be_bytes()); // max bitrate
    decoder.extend_from_slice(&0u32.to_be_bytes()); // average bitrate
    decoder.extend_from_slice(&decoder_specific);
    let mut es = vec![0x03, (3 + decoder.len() + 3) as u8, 0, 2, 0];
    es.extend_from_slice(&decoder);
    es.extend_from_slice(&[0x06, 0x01, 0x02]);
    full(b"esds", 0, 0, &es)
}

fn avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let profile = sps.get(1).copied().unwrap_or(66);
    let compatibility = sps.get(2).copied().unwrap_or(0);
    let level = sps.get(3).copied().unwrap_or(31);
    let mut bytes = vec![1, profile, compatibility, level, 0xff, 0xe1];
    bytes.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    bytes.extend_from_slice(sps);
    bytes.push(1);
    bytes.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    bytes.extend_from_slice(pps);
    bytes
}

fn trex(track_id: u32, default_duration: u32) -> Vec<u8> {
    let mut body = track_id.to_be_bytes().to_vec();
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&default_duration.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    full(b"trex", 0, 0, &body)
}

fn movie_fragment(
    sequence: u32,
    track_id: u32,
    decode_time: u64,
    duration: u32,
    size: u32,
    sample_flags: u32,
) -> Vec<u8> {
    let build = |data_offset: i32| {
        let mfhd = full(b"mfhd", 0, 0, &sequence.to_be_bytes());
        let tfhd = full(b"tfhd", 0, 0x020000, &track_id.to_be_bytes());
        let tfdt = full(b"tfdt", 1, 0, &decode_time.to_be_bytes());
        let mut trun = 1u32.to_be_bytes().to_vec();
        trun.extend_from_slice(&data_offset.to_be_bytes());
        trun.extend_from_slice(&duration.to_be_bytes());
        trun.extend_from_slice(&size.to_be_bytes());
        trun.extend_from_slice(&sample_flags.to_be_bytes());
        let mut traf = tfhd;
        traf.extend_from_slice(&tfdt);
        traf.extend_from_slice(&full(b"trun", 0, 0x000701, &trun));
        let mut body = mfhd;
        body.extend_from_slice(&bx(b"traf", &traf));
        bx(b"moof", &body)
    };
    let initial = build(0);
    build((initial.len() + 8) as i32)
}

fn media_time_units(time: eiviz_time::MediaTime, timescale: u32) -> u64 {
    let base = time.timebase();
    let value = time.ticks() as i128 * base.numerator() as i128 * timescale as i128
        / base.denominator() as i128;
    value.max(0).min(u64::MAX as i128) as u64
}

fn append_matrix(bytes: &mut Vec<u8>) {
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
}

fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

fn full(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![
        version,
        ((flags >> 16) & 0xff) as u8,
        ((flags >> 8) & 0xff) as u8,
        (flags & 0xff) as u8,
    ];
    body.extend_from_slice(payload);
    bx(kind, &body)
}
