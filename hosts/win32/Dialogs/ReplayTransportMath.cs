namespace Eiviz.Host.Dialogs;

/// <summary>
/// The transport's arithmetic, with no window in it.
///
/// The deck reads the speed as thousandths of real time, advances a playhead by wall clock,
/// and offers a fixed set of rates. None of that needs a control to be true, and the
/// numbers are the sort that go wrong quietly — a playhead that drifts by one frame a second
/// looks fine until the clip's remaining time is wrong — so they live here where they can be
/// tested.
/// </summary>
internal static class ReplayTransportMath
{
    /// <summary>Real time, in the thousandths the transport takes.</summary>
    public const int RealTimeMilli = 1000;

    /// <summary>
    /// The rates the deck offers, in the order its buttons show them. Fast enough to scrub
    /// a slow motion replay without the operator reaching for the jog wheel.
    /// </summary>
    public static readonly IReadOnlyList<int> Speeds = [250, 500, 750, RealTimeMilli];

    /// <summary>A rate in thousandths as a multiplier: 500 is half speed.</summary>
    public static double Multiplier(int speedMilli) => speedMilli / (double)RealTimeMilli;

    /// <summary>
    /// Frames a playhead advances in `seconds` at `speedMilli`.
    ///
    /// Truncated, not rounded: a playhead that rounds up would show a frame the decoder has
    /// not reached yet, and the countdown to the clip's end would then be short by the
    /// rounding on every tick.
    /// </summary>
    public static ulong FramesAdvanced(double seconds, double framesPerSecond, int speedMilli)
    {
        if (seconds <= 0 || framesPerSecond <= 0 || speedMilli <= 0)
            return 0;
        var frames = seconds * framesPerSecond * Multiplier(speedMilli);
        // A stall of days would otherwise overflow into a nonsense position; all three
        // inputs are positive here, so the only thing left to guard is the magnitude.
        return frames >= ulong.MaxValue ? ulong.MaxValue : (ulong)frames;
    }

    /// <summary>
    /// What is left of a clip, in frames, given where the playhead is.
    ///
    /// A playhead at or past the out point leaves nothing, and a clip with no frames in it
    /// has nothing to count down from: both read as zero rather than wrapping around.
    /// </summary>
    public static ulong RemainingFrames(ulong outFrame, ulong position) =>
        outFrame > position ? outFrame - position : 0;
}
