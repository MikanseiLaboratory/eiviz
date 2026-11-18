//! Plan and backend admission for session files.
//!
//! A session documents what its author could run. Opening it on a Community plan or
//! a build without a Pro backend should fail before the host tears down the live
//! session and draws controls that can never start. The host has a localized gate for
//! its own dialog; this one is the last line shared by every caller (the Windows host,
//! the remote host and headless tooling) that goes through the session load ABI.

use crate::abi::MixerCapabilities;
use eiviz_control::session::{Document, InputKind, OutputTransport};

/// Refuses a document the current plan or build cannot run.
pub fn check(document: &Document) -> Result<(), String> {
    let mut caps = MixerCapabilities::default();
    crate::pro::fill_capabilities(&mut caps);
    check_with(document, &caps)
}

fn check_with(document: &Document, caps: &MixerCapabilities) -> Result<(), String> {
    let mut issues: Vec<String> = Vec::new();

    if caps.mixing_unit_limit != u32::MAX
        && document.units.len() as u64 > u64::from(caps.mixing_unit_limit)
    {
        issues.push(format!(
            "{} Mixing Units; this plan allows {}",
            document.units.len(),
            caps.mixing_unit_limit
        ));
    }

    let replay_inputs = document
        .inputs
        .iter()
        .filter(|input| input.kind == InputKind::Replay)
        .count();
    if replay_inputs > 0 || document.replay.is_some() {
        if caps.replay_linked == 0 || caps.replay_channel_limit == 0 || caps.replay_angle_limit == 0
        {
            issues.push("Instant Replay is not included in this plan".into());
        } else {
            let channels = document
                .replay
                .as_ref()
                .map(|replay| replay.channels.len())
                .unwrap_or(replay_inputs);
            let angles = document
                .replay
                .as_ref()
                .map(|replay| replay.angles.len())
                .unwrap_or(0);
            if channels as u64 > u64::from(caps.replay_channel_limit) {
                issues.push(format!(
                    "{} Replay channels; this plan allows {}",
                    channels, caps.replay_channel_limit
                ));
            }
            if angles as u64 > u64::from(caps.replay_angle_limit) {
                issues.push(format!(
                    "{} Replay angles; this plan allows {}",
                    angles, caps.replay_angle_limit
                ));
            }
        }
    }

    let decklink_inputs = document
        .inputs
        .iter()
        .filter(|input| input.kind == InputKind::DeckLink)
        .count();
    if decklink_inputs > 0
        && (caps.decklink_linked == 0
            || caps.decklink_input_limit == 0
            || decklink_inputs as u64 > u64::from(caps.decklink_input_limit))
    {
        issues.push(format!(
            "{} DeckLink inputs; this plan or build does not support them",
            decklink_inputs
        ));
    }

    let decklink_outputs = document
        .outputs
        .iter()
        .filter(|output| output.transport == OutputTransport::DeckLink)
        .count();
    if decklink_outputs > 0
        && (caps.decklink_linked == 0
            || caps.decklink_output_limit == 0
            || decklink_outputs as u64 > u64::from(caps.decklink_output_limit))
    {
        issues.push(format!(
            "{} DeckLink outputs; this plan or build does not support them",
            decklink_outputs
        ));
    }

    let rtmp_outputs = document
        .outputs
        .iter()
        .filter(|output| output.transport == OutputTransport::Rtmp)
        .count();
    if rtmp_outputs > 0 && (caps.rtmp_linked == 0 || caps.rtmp_max_width == 0) {
        issues.push("RTMP output; streaming is not linked in this build".into());
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "this session cannot open on the current plan: {}",
            issues.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn community() -> MixerCapabilities {
        MixerCapabilities {
            plan: 0,
            mixing_unit_limit: 4,
            decklink_input_limit: 0,
            decklink_output_limit: 0,
            rtmp_max_width: 1280,
            rtmp_max_height: 720,
            rtmp_max_fps_num: 30,
            rtmp_max_fps_den: 1,
            recording: 0,
            srt: 0,
            hardware_encode: 0,
            decklink_linked: 0,
            rtmp_linked: 1,
            replay_angle_limit: 0,
            replay_channel_limit: 0,
            replay_linked: 0,
            stream_encoder_mask: 1,
        }
    }

    fn document(extra: &str) -> Document {
        let json = format!(
            r#"{{
              "version": 2,
              "inputs": [{{ "id": 2, "name": "Bars", "kind": "Bars" }}],
              "scenes": [{{ "id": 1, "name": "Scene 1", "layers": [{{ "inputId": 2, "width": 1, "height": 1 }}] }}],
              "units": [{{ "id": 1, "name": "MU 1" }}]{extra}
            }}"#
        );
        crate::session::parse(json.as_bytes()).expect("test document")
    }

    #[test]
    fn a_plain_session_opens() {
        assert!(check_with(&document(""), &community()).is_ok());
    }

    #[test]
    fn replay_is_refused_without_the_plan() {
        let mut doc = document("");
        doc.inputs[0].kind = InputKind::Replay;
        assert!(check_with(&doc, &community()).is_err());
    }

    #[test]
    fn decklink_input_is_refused() {
        let mut doc = document("");
        doc.inputs[0].kind = InputKind::DeckLink;
        assert!(check_with(&doc, &community()).is_err());
    }

    #[test]
    fn a_fifth_mixing_unit_is_refused() {
        let mut doc = document("");
        for id in 2..=5 {
            let mut unit = doc.units[0].clone();
            unit.id = id;
            unit.name = format!("MU {id}");
            doc.units.push(unit);
        }
        assert!(check_with(&doc, &community()).is_err());
    }

    #[test]
    fn rtmp_is_refused_when_streaming_is_not_linked() {
        let doc = document(r#", "outputs": [{"id": 100, "name": "stream", "transport": "Rtmp"}]"#);
        let mut caps = community();
        caps.rtmp_linked = 0;
        assert!(check_with(&doc, &caps).is_err());
        assert!(check_with(&doc, &community()).is_ok());
    }
}
