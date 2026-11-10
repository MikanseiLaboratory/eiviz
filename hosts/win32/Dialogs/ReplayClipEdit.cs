using System.Text.Json.Nodes;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// Writes one clip cell's edit into the session document.
///
/// A clip row is edited cell by cell, and each cell writes the field it owns. The names and
/// units are a contract with the core, not a detail of the UI: `speed` is a multiple of real
/// time, not the deck's milli-units, and a clip's sound is `audio`, not `audioKind`. Both of
/// those were once wrong here, and both failed silently — the cell moved, the publish
/// succeeded, and the next read put the old value back. Keeping the mapping in one place, away
/// from the window, is what makes it reviewable.
/// </summary>
internal static class ReplayClipEdit
{
    /// <summary>
    /// Applies one changed cell to the clip's JSON.
    ///
    /// Returns false when the property is not one of the clip's own cells, so the caller can
    /// ignore it and leave the document alone.
    /// </summary>
    /// <param name="clip">The clip object in the document.</param>
    /// <param name="property">The name of the cell that changed.</param>
    /// <param name="row">The row that holds the new value.</param>
    public static bool Apply(JsonObject clip, string? property, ClipRow row)
    {
        switch (property)
        {
            case nameof(ClipRow.AngleOption) when row.AngleOption is { } angle:
                clip["angleId"] = angle.Value;
                return true;
            case nameof(ClipRow.SpeedOption) when row.SpeedOption is { } speed:
                // The document stores the speed as a multiple of normal speed. Writing the
                // deck's milli-units left an unknown key, which the core drops without
                // complaint.
                clip["speed"] = Math.Round(speed.Value / 1000.0, 3);
                return true;
            case nameof(ClipRow.TransitionOption) when row.TransitionOption is { } transition:
                SetTransition(clip, transition.Value, null, null);
                return true;
            case nameof(ClipRow.TimingOption) when row.TimingOption is { } timing:
                SetTransition(clip, null, timing.Value, null);
                return true;
            case nameof(ClipRow.DirectionOption) when row.DirectionOption is { } direction:
                SetTransition(clip, null, null, null, direction.Value);
                return true;
            case nameof(ClipRow.TransitionFrames):
                SetTransition(clip, null, null, ParseFrames(row.TransitionFrames));
                return true;
            case nameof(ClipRow.GainDb):
                clip["gainDb"] = ParseGain(row.GainDb);
                return true;
            case nameof(ClipRow.PreservePitch):
                clip["preservePitch"] = row.PreservePitch;
                return true;
            case nameof(ClipRow.AudioOption) when row.AudioOption is { } audio:
                // `audio`, not `audioKind`: the deck and Replay Setup disagreed on this name,
                // and the core only reads the former.
                clip["audio"] = audio.Kind;
                clip["audioAngleId"] = audio.AngleId;
                return true;
            case nameof(ClipRow.TagsText):
                clip["tags"] = Tags(row.TagsText);
                return true;
            default:
                return false;
        }
    }

    /// <summary>
    /// The tag list an operator typed, as the document holds it: trimmed names, blanks dropped
    /// so a stray comma does not create an empty tag, and no duplicates.
    ///
    /// The row's text is the data, not a rendering of it, so this is also what the cell would
    /// write back if the operator edited it.
    /// </summary>
    public static JsonArray Tags(string text)
    {
        var tags = new JsonArray();
        foreach (var tag in text.Split(','))
        {
            var name = tag.Trim();
            if (name.Length == 0)
                continue;
            var known = false;
            foreach (var existing in tags)
            {
                // A node the document holds that is not a string is not a tag this edit can
                // duplicate; skip it rather than throwing on the read.
                if (existing is JsonValue value && value.TryGetValue<string>(out var label)
                    && string.Equals(label, name, StringComparison.Ordinal))
                {
                    known = true;
                    break;
                }
            }
            if (!known)
                tags.Add(name);
        }
        return tags;
    }

    /// <summary>
    /// Writes the transition, keeping the parts the caller did not change. A cut has no length,
    /// so choosing it clears the frames instead of leaving a stale duration.
    /// </summary>
    private static void SetTransition(
        JsonObject clip,
        string? kind = null,
        string? timing = null,
        uint? frames = null,
        string? direction = null)
    {
        var current = ReplayTransitionInfo.Read(clip["transition"]);
        var nextKind = kind ?? current.Kind;
        var nextTiming = timing ?? current.Timing;
        var nextFrames = frames ?? current.DurationFrames;
        var nextDirection = direction ?? current.Direction;
        if (nextKind == "Cut")
            nextFrames = 0;
        clip["transition"] = new JsonObject
        {
            ["kind"] = nextKind,
            ["timing"] = nextTiming,
            ["direction"] = nextDirection,
            ["durationFrames"] = nextFrames,
        };
    }

    /// <summary>Frames a transition lasts, as typed. Ten seconds at 60 fps is the ceiling.</summary>
    public static uint ParseFrames(string text) =>
        uint.TryParse(text.Trim(), out var frames) ? Math.Min(frames, 600u) : 0u;

    /// <summary>Gain in decibels, as typed, inside the range the mixer accepts.</summary>
    public static double ParseGain(string text) =>
        double.TryParse(text.Trim(), out var gain) ? Math.Clamp(gain, -60, 12) : 0;
}
