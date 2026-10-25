import Foundation

/// Where a replay channel is: at the live edge or in recorded material, and whether it is
/// advancing.
///
/// The deck used to hold this as two separate `@State` values — `followingLive` and
/// `playing` — and each action set them by hand. That let them disagree: stepping cleared
/// one, a seek set both, and the tile strip decided what to draw from whichever it happened
/// to read first. The two questions are genuinely independent — an operator parks on the
/// edge without starting the feed, which is what the LIVE button is for — so this keeps both,
/// but only as one value that every transition produces at once.
///
/// The Windows deck carries the same type, so the two hosts answer "is it playing" and "is it
/// at the edge" the same way.
struct ReplayTransportState: Equatable {
    /// Following the recording as it is written.
    var live: Bool
    /// The channel is advancing.
    var playing: Bool

    /// Following the recording edge, advancing.
    static let following = ReplayTransportState(live: true, playing: true)
    /// Parked on the edge, holding the newest frame. What the LIVE button produces.
    static let atEdge = ReplayTransportState(live: true, playing: false)
    /// Holding a frame of recorded material.
    static let held = ReplayTransportState(live: false, playing: false)
}

/// What the deck asks the mixer to do, and what it should show afterwards.
///
/// The seek frame is what `replayTransport` takes, and `-1` is the mixer's own "do not move
/// the playhead" value — the same value that starts a live follow, because both mean "the
/// transport decides where this is". There is one constant for it rather than two names for
/// one number.
struct ReplayTransportAction {
    /// The transport call's "leave the playhead to the mixer" seek value.
    static let keepPlayhead: Int64 = -1

    let next: ReplayTransportState
    let playing: Bool
    let seekFrame: Int64
}

enum ReplayTransportRules {
    /// Pressing play/pause.
    ///
    /// Only the play half changes. The edge flag is where the picture came from, and pressing
    /// play does not move the playhead — an operator who paused on the live feed and presses
    /// again is watching the same edge catch up. The Windows deck has always behaved this way;
    /// the macOS deck used to clear the flag here, which left the two hosts disagreeing about
    /// whether a channel playing from the edge is "live".
    ///
    /// The state that comes back is the one the button should show, whether or not the mixer
    /// accepted the command.
    static func toggle(_ state: ReplayTransportState) -> ReplayTransportAction {
        let next = ReplayTransportState(live: state.live, playing: !state.playing)
        return ReplayTransportAction(
            next: next, playing: next.playing, seekFrame: ReplayTransportAction.keepPlayhead)
    }

    /// Jumping to live: seek to the edge and play from there, so the picture appears at once
    /// instead of on the next frame the mixer happens to notice.
    static func jumpToLive() -> ReplayTransportAction {
        ReplayTransportAction(
            next: .following, playing: true, seekFrame: ReplayTransportAction.keepPlayhead)
    }

    /// Leaving the edge without touching the play state.
    ///
    /// Dragging the playhead or cueing an event moves the picture off the live edge, but the
    /// transport was handed the play state the channel already had and keeps it: a drag while
    /// paused must not start the channel, and one while playing must not stop it. Only the
    /// edge half changes, which is why this takes the state rather than producing a fixed one.
    static func leaveEdge(_ state: ReplayTransportState) -> ReplayTransportAction {
        ReplayTransportAction(
            next: ReplayTransportState(live: false, playing: state.playing),
            playing: state.playing,
            seekFrame: ReplayTransportAction.keepPlayhead)
    }

    /// Moving the playhead to hold a frame: pause there, and leave the edge behind. A seek
    /// never keeps playing, because the operator is looking at a picture in order to cut on it.
    static func seek() -> ReplayTransportAction {
        ReplayTransportAction(
            next: .held, playing: false, seekFrame: ReplayTransportAction.keepPlayhead)
    }

    /// What the mixer's own report means for a channel the deck is not driving.
    ///
    /// The mixer knows whether the channel is advancing; the deck only knows what it last
    /// asked for, and the two drift when a clip ends or a recording stops. Only the playing
    /// half is taken from the mixer: where the picture came from is the operator's choice,
    /// and the mixer does not report it.
    static func adopt(_ state: ReplayTransportState, mixerPlaying: Bool) -> ReplayTransportState {
        ReplayTransportState(live: state.live, playing: mixerPlaying)
    }
}
