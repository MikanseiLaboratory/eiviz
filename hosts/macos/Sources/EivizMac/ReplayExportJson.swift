import Foundation

/// Writes one playlist clip as the export contract takes it.
///
/// The export is not a row of cuts: a clip carries the transition out of it, its speed,
/// its gain and where its sound comes from. Building the JSON by hand in one place keeps
/// the two hosts and the mixer agreeing on those names, which is exactly what drifted the
/// last time this was written twice.
enum ReplayExportJson {
    /// Numeric transition codes the writer understands.
    /// 0 Cut, 1 Dissolve, 2 Dip, 3 Wipe, 4 Slide, 5 Push.
    static func kindCode(_ kind: String) -> UInt32? {
        switch kind {
        case "Dissolve": 1
        case "Dip": 2
        case "Wipe": 3
        case "Slide": 4
        case "Push": 5
        default: nil
        }
    }

    /// 0 Left, 1 Right, 2 Up, 3 Down.
    static func directionCode(_ direction: String) -> UInt32 {
        switch direction {
        case "Right": 1
        case "Up": 2
        case "Down": 3
        default: 0
        }
    }

    /// One clip object. `transition` is the transition out of this clip, if any.
    static func clip(_ clip: ReplayClipInfo, _ transition: ReplayTransitionInfo?) -> String {
        var parts = [
            "\"angleId\":\(clip.angleId)",
            "\"fromFrame\":\(clip.inFrame)",
            "\"toFrame\":\(clip.outFrame)",
            // The session stores a multiple of real time; the contract takes percent. The
            // transport offers 25 % up to 200 %, so the ceiling is the transport's, not 100:
            // clamping at 100 turned a 200 % clip into real time in the file.
            "\"speedPercent\":\(UInt32((min(max(clip.speed, 0.01), 10.0) * 100).rounded()))",
            "\"gain\":\(gain(decibels: clip.gainDb))",
        ]
        switch clip.audioKind {
        case "Silent":
            parts.append("\"audioAngleId\":null")
        case "Angle":
            parts.append("\"audioAngleId\":\(clip.audioAngleId)")
        default:
            // "Follow the clip's own angle" is the writer's default, so the key is left out
            // rather than naming the angle: a clip that follows its own picture is not a
            // clip that was pointed at an angle, and writing one made every clip export its
            // sound even when the document never asked for it.
            break
        }
        if let transition, !transition.isCut, let kind = kindCode(transition.kind) {
            let timing = transition.timing == "Additive" ? 1 : 0
            parts.append(
                "\"transition\":{\"kind\":\(kind),"
                    + "\"direction\":\(directionCode(transition.direction)),"
                    + "\"timing\":\(timing),\"frames\":\(transition.durationFrames)}"
            )
        }
        return "{" + parts.joined(separator: ",") + "}"
    }

    /// Clip gain in dB as the linear multiplier the contract carries.
    static func gain(decibels: Double) -> Double {
        let clamped = min(max(decibels, -60.0), 12.0)
        return (pow(10.0, clamped / 20.0) * 1000).rounded() / 1000
    }
}
