using Eiviz.Host.I18n;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// The sentences the deck shows about a recording: what it is doing, and where the frames went.
///
/// The numbers say how much was lost; the issues say why. Neither needs a window to work out,
/// so the deck keeps the poll and this keeps the wording.
///
/// The macOS deck formats the same two sentences from the same figures. It also appends the
/// recently dropped count to the health line; here the deck shows that on the state line, so
/// the two hosts agree about the recording without printing the identical string.
/// </summary>
internal static class ReplayHealthText
{
    /// <summary>
    /// What the state line says while the recorder is writing.
    ///
    /// A full store says so rather than counting down to nothing, and a store that cannot say
    /// how much is left uses <see cref="uint.MaxValue"/> so the deck can call the figure
    /// unknown instead of printing a meaningless zero. Anything else is the time left, so an
    /// operator watching the deck sees the drive fill.
    /// </summary>
    public static string Recording(MixerReplayStats stats)
    {
        if ((stats.Issues & ReplayIssues.StorageFull) != 0)
            return Loc.T("replay.stateRecordingNoSpace");
        if (stats.RemainingSecs == uint.MaxValue)
            return Loc.T("replay.stateRecordingUnknown");
        return Loc.Format(
            "replay.stateRecording",
            TimeSpan.FromSeconds(stats.RemainingSecs).ToString(@"h\:mm\:ss"));
    }

    /// <summary>
    /// The line for the angle being watched.
    ///
    /// Returns the issues alone when no angle is selected: without one there is no delivery
    /// figure, but why the recorder is unhappy is still worth reading.
    /// </summary>
    /// <param name="stats">The recording's own figures.</param>
    /// <param name="angle">The angle being watched, or default when there is none.</param>
    /// <param name="hasAngle">Whether <paramref name="angle"/> is a real reading.</param>
    public static string Line(MixerReplayStats stats, MixerReplayAngleStats angle, bool hasAngle)
    {
        var issues = ReplayIssues.Text(stats.Issues);
        if (!hasAngle)
        {
            // No angle reading to show, but the recording still knows what it wrote. Saying
            // nothing reads as "nothing is being recorded", which is a different fact.
            if (stats.FramesWritten == 0 && stats.FramesOffered == 0)
                return issues;
            var totals = Loc.Format(
                "replay.healthDelivery",
                stats.FramesOffered,
                stats.FramesWritten,
                stats.FramesPushed,
                0);
            return issues.Length > 0 ? totals + "  " + issues : totals;
        }
        // The format is "readback {0}/{2} (written {1}, readback lost {3})": the first slot is
        // what the readback delivered, the second what was written. They were passed the other
        // way round, so "written" showed the readback's figure — which on the GPU path is zero,
        // because the picture never leaves the GPU.
        var line = Loc.Format(
            "replay.healthDelivery",
            stats.FramesOffered,
            angle.FramesWritten,
            stats.FramesPushed,
            angle.DroppedReadback)
            + "  "
            + Loc.Format(
                "replay.healthDisk",
                angle.DroppedEncode,
                angle.DroppedWrite,
                angle.Segments,
                angle.EncodeQueueLen,
                angle.EncodeQueueCap,
                angle.WriteLatencyP95Ms);
        if (issues.Length > 0)
            line += "  " + issues;
        return line;
    }
}
