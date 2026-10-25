using System.Windows;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Builds the clip rows of the playlist list.
///
/// Every option list a clip row offers — angle, speed, transition, direction, timing, audio
/// — is derived from the session document and the locale, and nothing about it needs the
/// window. Keeping it here means the window only decides *when* to rebuild and what to do
/// with an edit, which is all it should be doing with a list this size.
/// </summary>
internal static class ReplayClipRows
{
    /// <summary>
    /// The speeds a clip can be cut at.
    ///
    /// Longer than the deck's transport offers on purpose: the transport is what the
    /// operator plays at, this is what a clip is authored at, and a clip may be cut at half
    /// or double time without the deck ever running there.
    /// </summary>
    internal static readonly IReadOnlyList<Option<uint>> Speeds =
    [
        new(250u, "25%"),
        new(500u, "50%"),
        new(750u, "75%"),
        new(1000u, "100%"),
        new(1500u, "150%"),
        new(2000u, "200%"),
    ];

    /// <summary>
    /// How a clip can take its sound: follow the picture's angle, silence, or another
    /// angle's recording.
    /// </summary>
    /// <param name="angles">The angles a clip can borrow sound from.</param>
    internal static List<ClipAudioChoice> AudioChoices(IReadOnlyList<ReplayAngleInfo> angles)
    {
        var choices = new List<ClipAudioChoice>
        {
            new("FollowAngle", 0, Loc.T("replay.audioFollow")),
            new("Silent", 0, Loc.T("replay.audioSilent")),
        };
        foreach (var angle in angles)
            choices.Add(new ClipAudioChoice("Angle", angle.Id, angle.Label));
        return choices;
    }

    /// <summary>
    /// Fills one row from a clip, reusing the row the list already has so an edit being
    /// typed into is not thrown away by a rebuild.
    /// </summary>
    /// <param name="row">The row to fill.</param>
    /// <param name="clip">The clip it shows.</param>
    /// <param name="angles">The angles the clip can record from.</param>
    /// <param name="fps">The master rate, for the timecodes.</param>
    /// <param name="stamp">Formats a frame as a timecode on its recording's clock.</param>
    /// <param name="expired">Whether the clip's frames have been overwritten.</param>
    internal static void Fill(
        ClipRow row,
        ReplayClipInfo clip,
        IReadOnlyList<ReplayAngleInfo> angles,
        (uint Num, uint Den) fps,
        Func<ulong, string?, string> stamp,
        Func<ReplayClipInfo, bool> expired)
    {
        row.IsLoading = true;
        row.RecordingUuid = clip.RecordingUuid;
        row.InText = stamp(clip.InFrame, clip.RecordingUuid);
        row.OutText = stamp(clip.OutFrame, clip.RecordingUuid);
        var duration = ReplayModel.Duration(clip.InFrame, clip.OutFrame, fps);
        row.DurationText = expired(clip) ? duration + " " + Loc.T("replay.expired") : duration;

        row.AngleOptions = [.. angles.Select(angle => new Option<uint>(angle.Id, angle.Label))];
        row.AngleOption = row.AngleOptions.FirstOrDefault(option => option.Value == clip.AngleId);

        row.SpeedOptions = Speeds;
        row.SpeedOption = row.SpeedOptions.FirstOrDefault(
            option => option.Value == (uint)Math.Round(clip.Speed * 1000)) ?? row.SpeedOptions[3];

        // The mixing unit's names, limited to what the replay path can draw: it composes two
        // decoded frames, where the rest are GPU warps.
        row.TransitionOptions =
        [
            new("Cut", Loc.T("replay.transitionCut")),
            new("Dissolve", Loc.T("replay.transitionDissolve")),
            new("Dip", Loc.T("replay.transitionDip")),
            new("Wipe", Loc.T("replay.transitionWipe")),
            new("Slide", Loc.T("replay.transitionSlide")),
            new("Push", Loc.T("replay.transitionPush")),
        ];
        row.TransitionOption = row.TransitionOptions.FirstOrDefault(
            option => option.Value == clip.Transition.Kind) ?? row.TransitionOptions[0];

        row.DirectionOptions =
        [
            new("Left", Loc.T("replay.directionLeft")),
            new("Right", Loc.T("replay.directionRight")),
            new("Up", Loc.T("replay.directionUp")),
            new("Down", Loc.T("replay.directionDown")),
        ];
        row.DirectionOption = row.DirectionOptions.FirstOrDefault(
            option => option.Value == clip.Transition.Direction) ?? row.DirectionOptions[0];

        row.TimingOptions =
        [
            new("Overwrap", Loc.T("replay.timingOverwrap")),
            new("Additive", Loc.T("replay.timingAdditive")),
        ];
        row.TimingOption = row.TimingOptions.FirstOrDefault(
            option => option.Value == clip.Transition.Timing) ?? row.TimingOptions[0];

        row.TransitionFrames = clip.Transition.DurationFrames.ToString();
        row.GainDb = clip.GainDb.ToString("0.#");
        row.PreservePitch = clip.PreservePitch;
        row.AudioOptions = AudioChoices(angles);
        row.AudioOption = row.AudioOptions.FirstOrDefault(choice =>
            choice.Kind == clip.AudioKind
            && (choice.Kind != "Angle" || choice.AngleId == clip.AudioAngleId)) ?? row.AudioOptions[0];
        // The tags as the document holds them, with no decoration: this is the value an edit
        // writes back, so a prefix here would be saved as a tag. The row's cell draws the
        // decoration, the way the event list's cells do.
        row.TagsText = string.Join(", ", clip.Tags);
        row.IsLoading = false;
    }
}
