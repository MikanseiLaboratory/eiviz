import Foundation

/// What an export writes, decided without a window.
///
/// The deck used to carry this among its views: which clips a mode exports, how a
/// transition becomes numbers the writer understands, and what each mode is called. None of
/// that needs SwiftUI, and keeping it apart is what lets it be tested and what keeps the
/// window from growing again.
enum ReplayExportPlan {
    /// The modes the operator can choose, in the order the picker shows them.
    enum Mode: String, CaseIterable {
        case clip = "Clip"
        /// The playlist as it played, transitions included.
        case playlistWithTransitions = "PlaylistWithTransitions"
        /// The same clips joined by cuts.
        case playlist = "Playlist"
        case angle = "Angle"
        case allAngles = "AllAngles"

        var titleKey: String {
            switch self {
            case .clip: "replay.exportClip"
            case .playlistWithTransitions: "replay.exportPlaylistTransitions"
            case .playlist: "replay.exportPlaylistCut"
            case .angle: "replay.exportAngle"
            case .allAngles: "replay.exportAllAngles"
            }
        }
    }

    /// The clips a mode exports, each with the transition out of it.
    ///
    /// A single clip has nothing to join to, so its transition is not exported: there is no
    /// next clip for it to dissolve into. The angle modes work in whole recordings, which a
    /// clip list cannot describe, so they return nothing here.
    ///
    /// "Cuts only" returns nothing on purpose: it is exported as plain runs of frames, not
    /// as clips with no transition, so that a module which cannot composite transitions is
    /// never asked to.
    static func clips(
        mode: String,
        clips: [ReplayClipInfo],
        selectedClip: String?
    ) -> [(clip: ReplayClipInfo, transition: ReplayTransitionInfo?)] {
        switch mode {
        case Mode.clip.rawValue:
            guard let uid = selectedClip, let clip = clips.first(where: { $0.uid == uid }) else {
                return []
            }
            return [(clip, nil)]
        case Mode.playlistWithTransitions.rawValue:
            return clips.map { ($0, $0.transition) }
        default:
            return []
        }
    }

    /// Whole recordings a mode exports, one per angle, in the order to walk them.
    ///
    /// The angle on air comes first, so an export of every angle writes the one the operator
    /// is watching before the rest. "Cuts only" is exported this way too: each playlist clip
    /// becomes a run of frames, joined hard, which is exactly what a segment is.
    static func segments(
        mode: String,
        clips: [ReplayClipInfo],
        angles: [ReplayAngleInfo],
        active: UInt32,
        range: (UInt32) -> (first: UInt64, last: UInt64)?
    ) -> [(UInt32, UInt64, UInt64)] {
        switch mode {
        case Mode.playlist.rawValue:
            return clips.map { ($0.angleId, $0.inFrame, $0.outFrame) }
        case Mode.angle.rawValue, Mode.allAngles.rawValue:
            let ordered = active == 0
                ? angles
                : angles.sorted { ($0.id == active ? 0 : 1) < ($1.id == active ? 0 : 1) }
            var result: [(UInt32, UInt64, UInt64)] = []
            for angle in ordered {
                if let bounds = range(angle.id) {
                    result.append((angle.id, bounds.first, bounds.last))
                }
                if mode == Mode.angle.rawValue {
                    break
                }
            }
            return result
        default:
            return []
        }
    }
}
