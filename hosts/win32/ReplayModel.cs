using System.Text.Json;
using System.Text.Json.Nodes;
using Eiviz.Host.Interop;
using Eiviz.Host.I18n;

namespace Eiviz.Host;

/// <summary>
/// The bits the mixer raises in <see cref="MixerReplayStats.Issues"/>, and the
/// words for them. The severity colour only says something is wrong; an operator
/// needs to read what.
/// </summary>
internal static class ReplayIssues
{
    public const uint CpuEncode = 1 << 0;
    public const uint StorageWrite = 1 << 1;
    public const uint Readback = 1 << 2;
    public const uint StorageLow = 1 << 3;
    public const uint StorageFull = 1 << 4;
    public const uint FramesDropped = 1 << 5;
    public const uint StoppedByPolicy = 1 << 6;
    public const uint ProUnavailable = 1 << 7;

    /// <summary>
    /// Most urgent first, because the list is read from the front.
    /// </summary>
    private static readonly (uint Bit, string Key)[] Named =
    [
        (StorageFull, "replay.issueStorageFull"),
        (StorageLow, "replay.issueStorageLow"),
        (StorageWrite, "replay.issueStorageWrite"),
        (CpuEncode, "replay.issueCpuEncode"),
        (Readback, "replay.issueReadback"),
        (FramesDropped, "replay.issueDropped"),
        (StoppedByPolicy, "replay.issueStopped"),
        (ProUnavailable, "replay.issuePro"),
    ];

    public static string Text(uint issues)
    {
        var names = Named
            .Where(entry => (issues & entry.Bit) != 0)
            .Select(entry => Loc.T(entry.Key))
            .ToList();
        return names.Count == 0 ? "" : Loc.Format("replay.issues", string.Join(", ", names));
    }
}

/// <summary>
/// Keeps the session's animation cues consistent with what Replay still has. A cue
/// whose channel or clip is gone would make the session unloadable, so it goes
/// with the thing it named.
/// </summary>
internal static class ReplayCues
{
    /// <summary>
    /// Clears the cues naming a channel, or a clip (or both when both are given).
    /// An empty <paramref name="clipUid"/> matches nothing, because that is what
    /// "the newest clip" is written as. Returns how many were cleared.
    /// </summary>
    public static int Clear(IEnumerable<SceneEntry> scenes, uint? channel = null, string? clipUid = null)
    {
        var cleared = 0;
        foreach (var scene in scenes)
        {
            foreach (var sequence in scene.Sequences)
            {
                foreach (var step in sequence.Steps)
                {
                    if (step.Replay is not { } action)
                        continue;
                    var namedChannel = channel is { } index && action.Channel == index;
                    var namedClip = !string.IsNullOrEmpty(clipUid)
                        && string.Equals(action.ClipUid, clipUid, StringComparison.Ordinal);
                    if (!namedChannel && !namedClip)
                        continue;
                    step.Replay = null;
                    cleared++;
                }
            }
        }
        return cleared;
    }

    /// <summary>Clears every cue, for when Replay itself is removed.</summary>
    public static int ClearAll(IEnumerable<SceneEntry> scenes)
    {
        var cleared = 0;
        foreach (var scene in scenes)
        {
            foreach (var sequence in scene.Sequences)
            {
                foreach (var step in sequence.Steps)
                {
                    if (step.Replay is null)
                        continue;
                    step.Replay = null;
                    cleared++;
                }
            }
        }
        return cleared;
    }
}

internal sealed record ReplayChannelInfo(uint Index, ulong InputId, ulong? PlaylistId)
{
    public string Letter => Index < 26 ? ((char)('A' + (int)Index)).ToString() : Index.ToString();
    public string Label => $"Replay {Letter}";
}

internal sealed record ReplayAngleInfo(uint Id, string Name, ulong SceneId, ulong AudioUnitId)
{
    public ulong SceneGpuId => MixerNative.SceneGpuId(SceneId);
    public string Label => string.IsNullOrWhiteSpace(Name) ? $"Angle {Id}" : Name;
}

internal sealed record ReplayEventInfo(ulong Id, string RecordingUuid, ulong InFrame, ulong OutFrame, string Name, bool Protected);

internal sealed record ReplayClipInfo(
    string Uid,
    uint AngleId,
    string RecordingUuid,
    ulong InFrame,
    ulong OutFrame,
    double Speed,
    double GainDb,
    ReplayTransitionInfo Transition,
    IReadOnlyList<string> Tags,
    bool PreservePitch,
    string AudioKind,
    uint AudioAngleId);

/// <summary>
/// Transition out of a clip. `Overwrap` overlaps the two clips, `Additive`
/// inserts the duration between them.
/// </summary>
internal sealed record ReplayTransitionInfo(string Kind, string Timing, uint DurationFrames)
{
    public static readonly ReplayTransitionInfo Cut = new("Cut", "Overwrap", 0);

    public bool IsCut => Kind == "Cut" || DurationFrames == 0;

    public string Label => IsCut ? "Cut" : $"{Kind} {DurationFrames}f {Timing}";

    /// <summary>
    /// Reads both the structured form and the bare kind name older sessions
    /// stored.
    /// </summary>
    public static ReplayTransitionInfo Read(JsonNode? node)
    {
        if (node is JsonValue value && value.TryGetValue<string>(out var name))
            return new ReplayTransitionInfo(name, "Overwrap", 0);
        if (node is not JsonObject entry)
            return Cut;
        return new ReplayTransitionInfo(
            entry["kind"]?.GetValue<string>() ?? "Cut",
            entry["timing"]?.GetValue<string>() ?? "Overwrap",
            entry["durationFrames"]?.GetValue<uint>() ?? 0);
    }
}

/// <summary>
/// Reads the replay document the host stores in <see cref="Session.ReplayJson"/>.
/// The JSON shape is the same one <c>eiviz_control::session::replay::ReplayDto</c>
/// serializes, so both sides stay interchangeable.
/// </summary>
internal static class ReplayModel
{
    public static JsonObject? Parse(Session session)
    {
        if (string.IsNullOrWhiteSpace(session.ReplayJson))
            return null;
        try
        {
            return JsonNode.Parse(session.ReplayJson) as JsonObject;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    public static string Serialize(JsonObject root) => root.ToJsonString();

    private static JsonArray Array(JsonObject root, string name) =>
        root[name] as JsonArray ?? [];

    public static IReadOnlyList<ReplayChannelInfo> Channels(JsonObject root)
    {
        var result = new List<ReplayChannelInfo>();
        foreach (var item in Array(root, "channels"))
        {
            if (item is not JsonObject channel)
                continue;
            var index = channel["index"]?.GetValue<uint>() ?? 0;
            var inputId = channel["inputId"]?.GetValue<ulong>() ?? 0;
            // A channel without a playlist has nothing to cue.
            ulong? playlistId = channel["playlistId"] is { } list ? list.GetValue<ulong>() : null;
            result.Add(new ReplayChannelInfo(index, inputId, playlistId));
        }
        result.Sort((a, b) => a.Index.CompareTo(b.Index));
        return result;
    }

    public static IReadOnlyList<ReplayAngleInfo> Angles(JsonObject root)
    {
        var result = new List<ReplayAngleInfo>();
        foreach (var item in Array(root, "angles"))
        {
            if (item is not JsonObject angle)
                continue;
            result.Add(new ReplayAngleInfo(
                angle["id"]?.GetValue<uint>() ?? 0,
                angle["name"]?.GetValue<string>() ?? "",
                angle["sceneId"]?.GetValue<ulong>() ?? 0,
                // Silence is the only other source, so an explicit unit id is what
                // matters.
                string.Equals(angle["audio"]?.GetValue<string>(), "Unit", StringComparison.OrdinalIgnoreCase)
                    ? angle["audioUnitId"]?.GetValue<ulong>() ?? 0
                    : 0));
        }
        return result;
    }

    public static IReadOnlyList<ReplayEventInfo> Events(JsonObject root)
    {
        var result = new List<ReplayEventInfo>();
        foreach (var item in Array(root, "events"))
        {
            if (item is not JsonObject entry)
                continue;
            result.Add(new ReplayEventInfo(
                entry["id"]?.GetValue<ulong>() ?? 0,
                entry["recordingUuid"]?.GetValue<string>() ?? "",
                entry["inFrame"]?.GetValue<ulong>() ?? 0,
                entry["outFrame"]?.GetValue<ulong>() ?? 0,
                entry["name"]?.GetValue<string>() ?? "",
                entry["protected"]?.GetValue<bool>() ?? false));
        }
        return result;
    }

    public static IReadOnlyList<ReplayClipInfo> Clips(JsonObject root, ulong? playlistId)
    {
        foreach (var item in Array(root, "playlists"))
        {
            if (item is not JsonObject playlist)
                continue;
            if (playlistId is not { } wanted || (playlist["id"]?.GetValue<ulong>() ?? 0) != wanted)
                continue;
            var result = new List<ReplayClipInfo>();
            foreach (var entry in playlist["clips"] as JsonArray ?? [])
            {
                if (entry is not JsonObject clip)
                    continue;
                result.Add(new ReplayClipInfo(
                    clip["uid"]?.GetValue<string>() ?? "",
                    clip["angleId"]?.GetValue<uint>() ?? 0,
                    clip["recordingUuid"]?.GetValue<string>() ?? "",
                    clip["inFrame"]?.GetValue<ulong>() ?? 0,
                    clip["outFrame"]?.GetValue<ulong>() ?? 0,
                    clip["speed"]?.GetValue<float>() ?? 1.0f,
                    clip["gainDb"]?.GetValue<float>() ?? 0.0f,
                    ReplayTransitionInfo.Read(clip["transition"]),
                    (clip["tags"] as JsonArray ?? [])
                        .Select(tag => tag?.GetValue<string>() ?? "")
                        .Where(tag => tag.Length > 0)
                        .ToList(),
                    clip["preservePitch"]?.GetValue<bool>() ?? false,
                    clip["audio"]?.GetValue<string>() ?? "FollowAngle",
                    clip["audioAngleId"]?.GetValue<uint>() ?? 0));
            }
            return result;
        }
        return [];
    }

    public static JsonObject? Playlist(JsonObject root, ulong playlistId)
    {
        foreach (var item in Array(root, "playlists"))
        {
            if (item is JsonObject playlist && (playlist["id"]?.GetValue<ulong>() ?? 0) == playlistId)
                return playlist;
        }
        return null;
    }

    /// <summary>
    /// The frame rate the timecodes are expressed in. The replay document is the
    /// authority; the session master rate is only used while the document has not
    /// recorded one yet.
    /// </summary>
    public static (uint Num, uint Den) Fps(JsonObject root, Session session)
    {
        var format = root["format"] as JsonObject;
        var num = format?["fpsNum"]?.GetValue<uint>() ?? 0;
        var den = format?["fpsDen"]?.GetValue<uint>() ?? 0;
        if (num == 0 || den == 0)
            return (session.Settings.MasterFpsNum, session.Settings.MasterFpsDen);
        return (num, den);
    }

    public static double Rate((uint Num, uint Den) fps) =>
        fps.Den == 0 ? 60.0 : (double)fps.Num / fps.Den;

    /// <summary>HH:MM:SS:FF.</summary>
    public static string Timecode(ulong frames, (uint Num, uint Den) fps)
    {
        var rate = Rate(fps);
        var perSecond = (int)Math.Round(rate);
        if (perSecond <= 0)
            return frames.ToString();
        var totalSeconds = frames / (ulong)perSecond;
        var frame = frames % (ulong)perSecond;
        return $"{totalSeconds / 3600:00}:{totalSeconds / 60 % 60:00}:{totalSeconds % 60:00}:{frame:00}";
    }

    public static string Duration(ulong inFrame, ulong outFrame, (uint Num, uint Den) fps)
    {
        var frames = outFrame > inFrame ? outFrame - inFrame : 0;
        var rate = Rate(fps);
        var seconds = rate <= 0 ? 0 : frames / rate;
        var whole = (ulong)seconds;
        var sub = (uint)Math.Round((seconds - whole) * (uint)Math.Round(rate));
        return $"{whole / 3600:00}:{whole / 60 % 60:00}:{whole % 60:00}:{sub:00}";
    }

    public static uint ActiveAngle(JsonObject root) =>
        root["activeAngleId"]?.GetValue<uint>() ?? 0;

    public static bool AutoAddToPlaylist(JsonObject root) =>
        root["autoAddToPlaylist"]?.GetValue<bool>() ?? false;

    /// <summary>
    /// Allocates an event id that no existing event uses. The win32 window builds
    /// the document without the counters, so the highest id wins.
    /// </summary>
    public static ulong ReserveEventId(JsonObject root)
    {
        var highest = 0UL;
        foreach (var item in Array(root, "events"))
            highest = Math.Max(highest, item?["id"]?.GetValue<ulong>() ?? 0);
        var next = Math.Max(root["nextEventId"]?.GetValue<ulong>() ?? 1, highest + 1);
        root["nextEventId"] = next + 1;
        return next;
    }

    public static ulong ReservePlaylistId(JsonObject root)
    {
        var highest = 0UL;
        foreach (var item in Array(root, "playlists"))
            highest = Math.Max(highest, item?["id"]?.GetValue<ulong>() ?? 0);
        var next = Math.Max(root["nextPlaylistId"]?.GetValue<ulong>() ?? 1, highest + 1);
        root["nextPlaylistId"] = next + 1;
        return next;
    }

    /// <summary>First playlist, created on demand when the caller needs one.</summary>
    public static ulong EnsurePlaylist(JsonObject root)
    {
        foreach (var item in Array(root, "playlists"))
        {
            if (item is JsonObject playlist)
                return playlist["id"]?.GetValue<ulong>() ?? 1;
        }
        var id = ReservePlaylistId(root);
        var list = new JsonArray();
        var document = root["playlists"] as JsonArray ?? new JsonArray();
        document.Add(new JsonObject
        {
            ["id"] = id,
            ["name"] = "Replay",
            ["revision"] = 1,
            ["clips"] = list
        });
        root["playlists"] = document;
        return id;
    }

    /// <summary>
    /// A clip uid that no playlist uses yet. The uid is the handle a channel plays
    /// by, so the control layer refuses a document where two clips share one.
    /// Playing the same event twice is a normal replay move, so the second clip
    /// takes a suffix instead of the operator being told no.
    /// </summary>
    public static string UniqueClipUid(JsonObject root, ulong eventId)
    {
        var used = new HashSet<string>(StringComparer.Ordinal);
        foreach (var item in Array(root, "playlists"))
        {
            if (item is not JsonObject playlist)
                continue;
            foreach (var clip in playlist["clips"] as JsonArray ?? [])
            {
                if (clip?["uid"]?.GetValue<string>() is { } uid)
                    used.Add(uid);
            }
        }
        var candidate = $"clip-{eventId}";
        for (var suffix = 2; used.Contains(candidate); suffix++)
            candidate = $"clip-{eventId}-{suffix}";
        return candidate;
    }
}







