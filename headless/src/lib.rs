//! Headless host package. Binaries live in `src/bin`.

#[cfg(test)]
mod tests {
    #[test]
    fn sample_session_validates_without_gpu() {
        let bytes = include_bytes!("../tests/fixtures/bars.eiviz.json");
        let doc = eiviz_control::parse(bytes).expect("parse");
        eiviz_control::session::validate_for_apply(&doc).expect("validate");
    }
}
