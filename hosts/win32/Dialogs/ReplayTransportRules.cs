namespace Eiviz.Host.Dialogs;

/// <summary>
/// Where a replay channel is: at the live edge or in recorded material, and whether it is
/// advancing.
///
/// The deck used to hold this as two separate flags and every action set them by hand. That
/// let them disagree: a seek cleared the live flag and the playing flag in two statements,
/// and a path that forgot one left the deck showing a live button over a paused channel. The
/// two questions are genuinely independent — an operator parks on the edge without starting
/// the feed, which is what the LIVE button is for — so this keeps both, but only as one
/// value that every transition produces at once.
/// </summary>
internal readonly record struct ReplayTransportState(bool Live, bool Playing)
{
    /// <summary>Following the recording edge, advancing.</summary>
    public static readonly ReplayTransportState Following =
        new(Live: true, Playing: true);

    /// <summary>Parked on the edge, holding the newest frame.</summary>
    public static readonly ReplayTransportState AtEdge =
        new(Live: true, Playing: false);

    /// <summary>Playing recorded material, away from the edge.</summary>
    public static readonly ReplayTransportState Recorded =
        new(Live: false, Playing: true);

    /// <summary>Holding a frame of recorded material.</summary>
    public static readonly ReplayTransportState Held =
        new(Live: false, Playing: false);
}

/// <summary>
/// What the deck asks the mixer to do, and what it should show afterwards.
///
/// The seek frame is what the transport call takes: <see cref="LiveEdge"/> follows the
/// recording, <see cref="KeepPlayhead"/> does not move it, and any other value is a frame to
/// hold.
/// </summary>
internal readonly record struct ReplayTransportAction(
    ReplayTransportState Next,
    bool Playing,
    long SeekFrame)
{
    /// <summary>The transport call's "go to the live edge" seek value.</summary>
    public const long LiveEdge = -1;

    /// <summary>The transport call's "leave the playhead alone" seek value.</summary>
    public const long KeepPlayhead = -1;
}

internal static class ReplayTransportRules
{
    /// <summary>
    /// Pressing play/pause.
    ///
    /// From anything advancing it pauses; from anything held it plays. Pausing on the edge
    /// keeps the live flag, so the deck still says it is at the edge — that is how a mark is
    /// taken from live material. The state that comes back is the one the button should show,
    /// whether or not the mixer accepted the command.
    /// </summary>
    public static ReplayTransportAction Toggle(ReplayTransportState state)
    {
        var next = state.Playing
            ? state with { Playing = false }
            : state with { Playing = true };
        return new ReplayTransportAction(next, next.Playing, ReplayTransportAction.KeepPlayhead);
    }

    /// <summary>
    /// Jumping to live: seek to the edge and play from there, so the picture appears at once
    /// instead of on the next frame the mixer happens to notice.
    /// </summary>
    public static ReplayTransportAction JumpToLive() =>
        new(ReplayTransportState.Following, true, ReplayTransportAction.LiveEdge);

    /// <summary>
    /// Parking on the edge without starting the feed, which is what the LIVE button does.
    /// </summary>
    public static ReplayTransportAction ParkAtEdge() =>
        new(ReplayTransportState.AtEdge, false, ReplayTransportAction.LiveEdge);

    /// <summary>
    /// Moving the playhead: hold on the target frame and leave the edge behind. A seek never
    /// keeps playing, because the operator is looking at a picture in order to cut on it.
    /// </summary>
    public static ReplayTransportAction Seek() =>
        new(ReplayTransportState.Held, false, ReplayTransportAction.KeepPlayhead);

    /// <summary>
    /// Leaving the edge without touching the play state.
    ///
    /// Dragging the playhead or cueing an event moves the picture off the live edge, but the
    /// transport was handed the play state the channel already had and keeps it: a drag while
    /// paused must not start the channel, and one while playing must not stop it. Only the
    /// edge half changes, which is why this takes the state rather than producing a fixed one.
    /// </summary>
    public static ReplayTransportAction LeaveEdge(ReplayTransportState state) =>
        new(state with { Live = false }, state.Playing, ReplayTransportAction.KeepPlayhead);

    /// <summary>
    /// Stopping: the channel holds where it is, but it has not left the edge. A stop while
    /// parked on the live feed keeps the LIVE lamp lit, because the operator has not moved the
    /// playhead anywhere.
    /// </summary>
    public static ReplayTransportAction StopInPlace(ReplayTransportState state) =>
        new(state with { Playing = false }, false, ReplayTransportAction.KeepPlayhead);

    /// <summary>
    /// Starting a clip or an event: play recorded material, which is by definition not the
    /// live edge.
    /// </summary>
    public static ReplayTransportAction PlayRecorded() =>
        new(ReplayTransportState.Recorded, true, ReplayTransportAction.KeepPlayhead);

    /// <summary>Stopping: hold where the channel is, off the edge.</summary>
    public static ReplayTransportAction Stop() =>
        new(ReplayTransportState.Held, false, ReplayTransportAction.KeepPlayhead);

    /// <summary>
    /// What the mixer's own report means for a channel the deck is not driving.
    ///
    /// The mixer knows whether the channel is advancing; the deck only knows what it last
    /// asked for, and the two drift when a clip ends or a recording stops. Only the playing
    /// half is taken from the mixer: where the picture came from is the operator's choice,
    /// and the mixer does not report it.
    /// </summary>
    public static ReplayTransportState Adopt(ReplayTransportState deck, bool mixerPlaying) =>
        deck with { Playing = mixerPlaying };
}
