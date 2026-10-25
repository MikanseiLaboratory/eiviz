import Foundation

/// The recording health line the deck shows under the transport.
///
/// The numbers say how much was lost; the issues say why. Both are worth reading in one
/// line, and neither needs a window to be worked out — which is what lets this be checked
/// without a mixer, and keeps the deck from carrying the formatting as well as the poll.
enum ReplayHealth {
    /// The line for the angle being watched.
    ///
    /// Returns an empty line when there is no recording to report on, so the deck can hide
    /// the strip rather than show a blank row. The reason a recording stopped stays readable
    /// after it stops, which is when an operator most wants it.
    ///
    /// - Parameters:
    ///   - stats: the recording's own figures.
    ///   - angle: the angle being watched, or nil when the document names none.
    static func line(stats: EivizReplayStats, angle: EivizReplayAngleStats?) -> String {
        guard stats.state != 0 else { return "" }
        let issues = ReplayIssues.text(stats.issues)
        guard let angle else {
            // Without a selected angle there is no delivery figure, but why the recorder is
            // unhappy is still worth reading.
            return issues
        }
        var line = L10n.format(
            "replay.healthDelivery",
            "\(angle.frames_written)",
            "\(stats.frames_offered)",
            "\(stats.frames_pushed)",
            "\(angle.dropped_readback)"
        ) + "  " + L10n.format(
            "replay.healthDisk",
            "\(angle.dropped_encode)",
            "\(angle.dropped_write)",
            "\(angle.segments)",
            "\(angle.encode_queue_len)",
            "\(angle.encode_queue_cap)",
            "\(angle.write_latency_p95_ms)"
        )
        if !issues.isEmpty {
            line += "  " + issues
        }
        if stats.frames_dropped_recent > 0 {
            line += "  " + L10n.format("replay.dropped", "\(stats.frames_dropped_recent)")
        }
        return line
    }

    /// The angle a stat row belongs to: the one the operator picked, else the first.
    static func watchedAngle(active: UInt32, angles: [ReplayAngleInfo]) -> UInt32 {
        active != 0 ? active : (angles.first?.id ?? 0)
    }
}
