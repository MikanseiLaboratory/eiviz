using System.Text.Json.Nodes;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// One thing to export: either a run of frames joined as hard cuts, or a playlist as it
/// plays, transitions included.
///
/// The second form exists because a playlist is not a row of cuts. A writer that only
/// understands frame runs cannot reproduce the dissolve between two clips, and flattening
/// it would hand the operator a file that is not the programme they watched, so the two
/// forms are separate here rather than one form with a flag.
/// </summary>
public sealed class ReplayExportRequest
{
    private readonly JsonArray _segments = [];
    private readonly JsonArray _clips = [];

    /// <summary>Whether anything was selected at all.</summary>
    public bool IsEmpty => _segments.Count == 0 && _clips.Count == 0;

    /// <summary>Adds a run of frames, played as a hard cut.</summary>
    public void AddSegment(uint angleId, ulong fromFrame, ulong toFrame) =>
        _segments.Add(new JsonObject
        {
            ["angleId"] = angleId,
            ["fromFrame"] = fromFrame,
            ["toFrame"] = toFrame
        });

    /// <summary>
    /// Adds one playlist clip with everything that decides how it joins the next one.
    /// </summary>
    /// <param name="clip">The clip as the session stores it.</param>
    /// <param name="transition">Its transition out, or null for a cut.</param>
    internal void AddClip(ReplayClipInfo clip, ReplayTransitionExport? transition)
    {
        var node = new JsonObject
        {
            ["angleId"] = clip.AngleId,
            ["fromFrame"] = clip.InFrame,
            ["toFrame"] = clip.OutFrame,
            // The session stores a multiple of real time; the export contract takes
            // percent, so a clip at 0.5 plays at 50%. The transport offers up to 200%, so
            // the ceiling is 10000 percent: clamping at 100 turned a fast clip into real
            // time in the file.
            ["speedPercent"] = (uint)Math.Round(Math.Clamp(clip.Speed, 0.01, 10.0) * 100.0),
            ["gain"] = Math.Clamp(clip.GainDb, -60.0, 12.0) == 0.0
                ? 1.0
                : Math.Pow(10.0, Math.Clamp(clip.GainDb, -60.0, 12.0) / 20.0)
        };
        // Every clip names the angle its sound comes from. Leaving the key out for "follow
        // the clip's own angle" meant an absent id, and the writer reads an absent id as
        // silent — so the default export produced a muted track that looked correct.
        node["audioAngleId"] = clip.AudioKind switch
        {
            "Silent" => null,
            "Angle" => clip.AudioAngleId,
            _ => clip.AngleId,
        };
        if (transition is { } value)
        {
            node["transition"] = new JsonObject
            {
                ["kind"] = value.Kind,
                ["direction"] = value.Direction,
                ["timing"] = value.Timing,
                ["frames"] = value.Frames
            };
        }
        _clips.Add(node);
    }

    /// <summary>The two member names the export body carries.</summary>
    public (string Segments, string Clips) Members() =>
        (_segments.ToJsonString(), _clips.ToJsonString());
}

/// <summary>
/// A transition as an export sees it: numeric codes the writer understands, so it does
/// not have to guess from the session's words.
/// </summary>
/// <param name="Kind">0 Cut, 1 Dissolve, 2 Dip, 3 Wipe, 4 Slide, 5 Push.</param>
/// <param name="Direction">0 Left, 1 Right, 2 Up, 3 Down.</param>
/// <param name="Timing">0 Overwrap, 1 Additive.</param>
/// <param name="Frames">Frames the transition occupies on the exported timeline.</param>
public readonly record struct ReplayTransitionExport(uint Kind, uint Direction, uint Timing, uint Frames)
{
    /// <summary>
    /// Reads a clip's transition from the session's own words. A clip whose transition
    /// is a cut has none, which is what tells the writer to join it hard.
    /// </summary>
    internal static ReplayTransitionExport? From(ReplayTransitionInfo transition)
    {
        if (transition.IsCut)
            return null;
        var kind = transition.Kind switch
        {
            "Dissolve" => 1u,
            "Dip" => 2u,
            "Wipe" => 3u,
            "Slide" => 4u,
            "Push" => 5u,
            _ => 0u
        };
        if (kind == 0)
            return null;
        var direction = transition.Direction switch
        {
            "Right" => 1u,
            "Up" => 2u,
            "Down" => 3u,
            _ => 0u
        };
        var timing = transition.Timing == "Additive" ? 1u : 0u;
        return new ReplayTransitionExport(kind, direction, timing, transition.DurationFrames);
    }
}
