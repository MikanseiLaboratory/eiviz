//! Default session used when headless starts without `--session`.

use crate::session::{Document, parse};

const DEFAULT_JSON: &[u8] = br#"{
  "version": 2,
  "inputs": [
    { "id": 1, "name": "Color Red", "kind": "Color", "colorR": 1, "colorG": 0, "colorB": 0 },
    { "id": 2, "name": "SMPTE HD Bars", "kind": "Bars", "scroll": true, "toneHz": 1000 },
    { "id": 3, "name": "Black", "kind": "Black", "colorR": 0, "colorG": 0, "colorB": 0 },
    { "id": 4, "name": "Blue", "kind": "Color", "colorR": 0, "colorG": 0, "colorB": 1 }
  ],
  "scenes": [
    { "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] },
    { "id": 2, "name": "Scene 2", "layers": [{ "inputId": 1, "width": 1, "height": 1 }] }
  ],
  "units": [{
    "id": 1,
    "name": "Mixing Unit 1",
    "previewSceneId": 1,
    "programSceneId": 2,
    "audioBusId": 1
  }],
  "outputs": [{
    "id": 100,
    "name": "eiviz-pgm",
    "transport": "Omt",
    "sourceKind": "MuProgram",
    "unitId": 1,
    "useGpu": false,
    "audioBusId": 1
  }],
  "buses": [{ "id": 1, "name": "Master", "role": "Master" }]
}"#;

pub fn default_document() -> Document {
    parse(DEFAULT_JSON).expect("default session JSON")
}

pub fn dated_session_filename(unix_secs: u64) -> String {
    let (year, month, day, hour, min, sec) = unix_utc_ymdhms(unix_secs);
    format!("eiviz-{year:04}{month:02}{day:02}-{hour:02}{min:02}{sec:02}.eivz")
}

pub fn dated_session_filename_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    dated_session_filename(secs)
}

fn unix_utc_ymdhms(unix_secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (unix_secs / 86_400) as i64;
    let tod = unix_secs % 86_400;
    let hour = (tod / 3_600) as u32;
    let min = ((tod % 3_600) / 60) as u32;
    let sec = (tod % 60) as u32;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, hour, min, sec)
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::validate;

    #[test]
    fn default_session_is_valid() {
        let doc = default_document();
        validate(&doc).unwrap();
        crate::session::validate_for_apply(&doc).unwrap();
        assert_eq!(doc.inputs.len(), 4);
        assert_eq!(doc.scenes.len(), 2);
        assert_eq!(doc.units.len(), 1);
        assert!(!doc.outputs[0].use_gpu);
    }

    #[test]
    fn dated_filename_uses_utc_stamp() {
        assert_eq!(dated_session_filename(0), "eiviz-19700101-000000.eivz");
        assert_eq!(
            dated_session_filename(1_700_000_000),
            "eiviz-20231114-221320.eivz"
        );
    }
}
