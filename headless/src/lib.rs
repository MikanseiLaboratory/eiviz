//! Headless host package. Binaries live in `src/bin`.

pub mod prefs;

pub use prefs::HeadlessPrefs;

#[cfg(test)]
mod tests {
    #[test]
    fn sample_session_validates_without_gpu() {
        let bytes = include_bytes!("../tests/fixtures/bars.eivz");
        let document = eiviz_control::session::decode_file(bytes).expect("eivz");
        eiviz_control::session::validate_for_apply(&document).expect("validate");
        let encoded = eiviz_control::session::encode_file(&document).expect("encode");
        let roundtrip = eiviz_control::session::decode_file(&encoded).expect("roundtrip");
        eiviz_control::session::validate_for_apply(&roundtrip).expect("validate roundtrip");
        assert_eq!(document.inputs[0].kind, roundtrip.inputs[0].kind);
    }
}
