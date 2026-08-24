mod bitstream;
mod flv;
mod fmp4;
mod h264;
mod mpegts;
mod openh264;

use eiviz_media::{EncodedAccessUnit, EncodedKind, VideoFrame};

pub use flv::{flv_avc_nalu, flv_avc_sequence_header, flv_header};
pub use fmp4::FragmentedMp4;
pub use h264::{
    AvccError, avcc_parameter_sets_to_annexb, avcc_sample_to_annexb, encode_idr, extract_sps_pps,
    split_annexb,
};
pub use mpegts::{pat, pes_video, pmt};
pub use openh264::{OpenH264Decoder, OpenH264Error};

/// Legacy private AU used when a caller needs a non-H.264 dump.
pub fn wrap_raw(frame: &VideoFrame) -> EncodedAccessUnit {
    let mut bytes = Vec::with_capacity(16 + frame.data.len());
    bytes.extend_from_slice(b"RAW0");
    bytes.extend_from_slice(&frame.width.to_le_bytes());
    bytes.extend_from_slice(&frame.height.to_le_bytes());
    bytes.extend_from_slice(&frame.data);
    EncodedAccessUnit {
        pts: frame.pts,
        dts: Some(frame.pts),
        keyframe: true,
        bytes,
        kind: EncodedKind::Avc,
    }
}

pub fn is_raw(au: &EncodedAccessUnit) -> bool {
    au.bytes.starts_with(b"RAW0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_media::VideoFrame;
    use eiviz_time::MediaTime;

    #[test]
    fn wrap_roundtrip_header() {
        let f = VideoFrame::rgba_solid(0, MediaTime::ZERO, 2, 2, [9, 8, 7, 255]);
        let au = wrap_raw(&f);
        assert!(is_raw(&au));
        assert!(au.keyframe);
    }

    #[test]
    fn h264_annexb_starts_with_start_code() {
        let f = VideoFrame::rgba_solid(1, MediaTime::ZERO, 16, 16, [20, 40, 80, 255]);
        let au = encode_idr(&f);
        assert!(au.bytes.starts_with(&[0, 0, 0, 1]));
        let (sps, pps) = extract_sps_pps(&au);
        assert!(!sps.is_empty() && !pps.is_empty());
        let mut mp4 = FragmentedMp4::new(60000, 16, 16, &sps, &pps);
        mp4.write_sample(&au, 1001);
        assert!(mp4.bytes.windows(4).any(|w| w == b"ftyp"));
        assert!(mp4.bytes.windows(4).any(|w| w == b"moof"));
        let flv = flv_header();
        assert_eq!(&flv[..3], b"FLV");
        let ts = pat();
        assert_eq!(ts[0], 0x47);
        assert_eq!(ts.len(), 188);
    }

    #[test]
    fn checked_avcc_conversion_rejects_malformed_samples() {
        let sample = [
            0, 0, 0, 2, 0x65, 0xaa, // IDR
            0, 0, 0, 3, 0x41, 0xbb, 0xcc, // non-IDR
        ];
        let annexb = avcc_sample_to_annexb(&sample, 4, 64).unwrap();
        assert_eq!(
            annexb,
            vec![0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x41, 0xbb, 0xcc]
        );
        assert_eq!(
            avcc_sample_to_annexb(&[0, 0, 0, 0], 4, 64),
            Err(AvccError::ZeroLength)
        );
        assert_eq!(
            avcc_sample_to_annexb(&[0, 0, 0, 4, 0x65], 4, 64),
            Err(AvccError::TruncatedNal)
        );
        assert_eq!(
            avcc_sample_to_annexb(&sample, 4, 8),
            Err(AvccError::OutputLimit(8))
        );
    }

    #[test]
    fn avcc_parameter_sets_are_injected_on_decoder_reset() {
        let preamble =
            avcc_parameter_sets_to_annexb(&[vec![0x67, 1]], &[vec![0x68, 2]], 32).unwrap();
        assert_eq!(preamble, vec![0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x68, 2]);
    }
}
