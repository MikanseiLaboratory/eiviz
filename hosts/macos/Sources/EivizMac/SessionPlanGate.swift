import EivizMixer
import Foundation

/// Refuses a session whose features the current plan or build cannot run.
///
/// The mixer refuses each unsupported operation when it is reached, but opening the
/// session first is the damaging part: the deck shows Replay channels, DeckLink I/O or
/// Mixing Units that can never start, and the operator has to work out which parts of
/// the project are real. The gate runs before the current session is torn down.
enum SessionPlanGate {
    /// Features in `session` that `caps` does not admit. Kept pure so the rules can be
    /// checked without opening a window or replacing the live session.
    static func issues(in session: MixerSessionData, caps: EivizCapabilities) -> [String] {
        var issues: [String] = []

        if caps.mixing_unit_limit != UInt32.max,
           session.units.count > Int(caps.mixing_unit_limit) {
            issues.append(L10n.format(
                "msg.sessionPlanUnits",
                "\(session.units.count)",
                "\(caps.mixing_unit_limit)"))
        }

        let replayInputs = session.inputs.filter { $0.kind == .replay }.count
        if replayInputs > 0 || session.replay != nil {
            let replay = ReplayDocument(session.replay)
            let channels = replay?.channels.count ?? replayInputs
            let angles = replay?.angles.count ?? 0
            if caps.replay_linked == 0
                || caps.replay_channel_limit == 0
                || caps.replay_angle_limit == 0 {
                issues.append(L10n.t("replay.planDenied"))
            } else if channels > Int(caps.replay_channel_limit)
                || angles > Int(caps.replay_angle_limit) {
                issues.append(L10n.format(
                    "replay.planLimits",
                    "\(caps.replay_angle_limit)",
                    "\(caps.replay_channel_limit)"))
            }
        }

        let decklinkInputs = session.inputs.filter { $0.kind == .deckLink }.count
        if decklinkInputs > 0,
           caps.decklink_linked == 0 || Int(caps.decklink_input_limit) < decklinkInputs {
            issues.append(L10n.format("msg.sessionPlanDeckLinkIn", "\(decklinkInputs)"))
        }

        let decklinkOutputs = session.outputs.filter { $0.transport == .deckLink }.count
        if decklinkOutputs > 0,
           caps.decklink_linked == 0 || Int(caps.decklink_output_limit) < decklinkOutputs {
            issues.append(L10n.format("msg.sessionPlanDeckLinkOut", "\(decklinkOutputs)"))
        }

        let rtmpOutputs = session.outputs.filter { $0.transport == .rtmp }.count
        if rtmpOutputs > 0, caps.rtmp_linked == 0 || caps.rtmp_max_width == 0 {
            issues.append(L10n.t("msg.sessionPlanRtmp"))
        }

        return issues
    }

    /// A message for the operator, or nil when the session can open.
    static func blockReason(for session: MixerSessionData) -> String? {
        var caps = EivizCapabilities()
        guard mixer_capabilities(&caps) == 0 else { return nil }
        let issues = issues(in: session, caps: caps)
        guard !issues.isEmpty else { return nil }
        return L10n.t("msg.sessionPlanIntro")
            + "\n\n"
            + issues.map { "• \($0)" }.joined(separator: "\n")
    }
}
