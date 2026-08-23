use eiviz_media::{EncodedAccessUnit, EncodedKind, VideoFrame};

/// Software fallback: wrap RGBA as a private "raw" AU so muxers can still
/// fan-out without Vulkan Video. Not a legal H.264 bitstream.
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
}
