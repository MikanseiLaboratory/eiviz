using System.IO;
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
    /// <summary>A GPU angle could not encode: the codec or the texture was refused.</summary>
    public const uint EncoderUnavailable = 1 << 8;

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
        (EncoderUnavailable, "replay.issueEncoder"),
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

    /// <summary>
    /// Source the channel paints into, which is the input that owns it or, for a
    /// document written without one, the reserved channel id.
    /// </summary>
    public ulong SourceId => InputId != 0 ? InputId : 0x0008_0000UL | (Index + 1);
}

internal sealed record ReplayAngleInfo(
    uint Id,
    string Name,
    ulong SceneId,
    ulong AudioUnitId,
    string Source = "Scene",
    ulong InputId = 0,
    string Encoder = "Cpu",
    JsonObject? Json = null,
    string Audio = "Silent")
{
    /// <summary>`Gpu` encodes from the scene texture; `Cpu` reads it back first.</summary>
    public static string EncoderOf(JsonObject? angle) =>
        angle?["encoder"]?.GetValue<string>() ?? "Cpu";

    public ulong SceneGpuId => MixerNative.SceneGpuId(SceneId);
    /// <summary>
    /// What the angle shows live: the scene it records, or the input it records
    /// directly. The deck's monitors follow this, so an input angle previews the
    /// input rather than a scene it does not use.
    /// </summary>
    public ulong LiveSourceId => Source == "Input" ? InputId : SceneGpuId;
    public string Label => string.IsNullOrWhiteSpace(Name) ? $"Angle {Id}" : Name;
}

internal sealed record ReplayEventInfo(ulong Id, string RecordingUuid, ulong InFrame, ulong OutFrame, string Name, bool Protected, IReadOnlyList<string> Tags);

/// <summary>One playlist a project holds: its name and how many clips are filed in it.</summary>
internal sealed record ReplayPlaylistInfo(ulong Id, string Name, int Clips)
{
    public string Label => Clips == 0 ? Name : $"{Name}  ({Clips})";

    /// <summary>What a screen reader and the automation tree read out.</summary>
    public override string ToString() => Label;
}

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
internal sealed record ReplayTransitionInfo(string Kind, string Timing, uint DurationFrames, string Direction = "Left")
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
            entry["durationFrames"]?.GetValue<uint>() ?? 0,
            entry["direction"]?.GetValue<string>() ?? "Left");
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

    /// <summary>
    /// A Replay document with one channel and nothing recorded yet.
    ///
    /// The format comes from the project's Mixing Unit, which is the rule the core's own
    /// <c>seed_format</c> applies: Replay records at the programme's size and rate unless
    /// the operator says otherwise. Replay is off until the operator turns it on, so
    /// adding the input never starts recording.
    /// </summary>
    public static string NewDocument(
        uint width,
        uint height,
        uint fpsNum,
        uint fpsDen,
        ulong channel,
        ulong inputId) =>
        new JsonObject
        {
            ["enabled"] = false,
            ["storage"] = new JsonArray(),
            ["format"] = new JsonObject
            {
                ["width"] = width,
                ["height"] = height,
                ["fpsNum"] = fpsNum,
                ["fpsDen"] = fpsDen,
                // Standard quality: the same default the session model carries.
                ["profile"] = "Sq",
                ["segmentFrames"] = 120
            },
            ["channels"] = new JsonArray
            {
                new JsonObject
                {
                    ["index"] = channel,
                    ["inputId"] = inputId
                }
            },
            ["angles"] = new JsonArray(),
            ["events"] = new JsonArray(),
            ["playlists"] = new JsonArray()
        }.ToJsonString();

    /// <summary>
    /// The same document with one more channel. Everything else is left untouched, so a
    /// second Replay input does not reset the operator's angles, events or playlists.
    /// </summary>
    public static string AddChannel(string json, ulong channel, ulong inputId)
    {
        if (ParseText(json) is not { } root)
            return json;
        var channels = root["channels"] as JsonArray ?? [];
        channels.Add(new JsonObject
        {
            ["index"] = channel,
            ["inputId"] = inputId
        });
        root["channels"] = channels;
        return Serialize(root);
    }

    /// <summary>Parses a Replay document that is not attached to a session yet.</summary>
    private static JsonObject? ParseText(string json)
    {
        if (string.IsNullOrWhiteSpace(json))
            return null;
        try
        {
            return JsonNode.Parse(json) as JsonObject;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// The settings part of a Replay document, for the `updateReplaySettings`
    /// mutation. A remote core owns the events and playlists, so Replay Setup must
    /// not send them back from a copy that may already be stale.
    /// </summary>
    public static string SerializeSettings(JsonObject root)
    {
        var settings = new JsonObject();
        foreach (var name in new[]
                 {
                     "enabled", "storage", "format", "buffering", "retention", "warnings",
                     "angles", "autoAddToPlaylist", "decodeAhead", "profiling"
                 })
        {
            if (root[name] is { } value)
                settings[name] = value.DeepClone();
        }
        return settings.ToJsonString();
    }

    /// <summary>
    /// Builds the deck mutation for a save.
    ///
    /// The deck owns events, playlists and which playlist a channel plays; Replay Setup
    /// owns the drives, the angles and the format. Sending the deck's whole document put
    /// a Replay Setup apply back to whatever the deck had loaded, and merging the arrays
    /// here instead meant writing the core's own rules — by id, forward-only counters,
    /// deletions said explicitly — a second time, in C#. That copy is what drifted: an
    /// event marked by the API while the window was open vanished on the next deck save.
    /// The core now applies the difference, so this only says what the deck changed.
    /// </summary>
    /// <param name="deck">The document as the deck currently has it.</param>
    /// <param name="angleEdited">Whether this window picked a camera.</param>
    /// <param name="deletedEvents">Events the operator deleted here.</param>
    /// <param name="deletedClips">Clips the operator deleted here.</param>
    public static string DeckMutation(
        JsonObject deck,
        bool angleEdited = true,
        IReadOnlySet<ulong>? deletedEvents = null,
        IReadOnlySet<string>? deletedClips = null)
    {
        var body = SerializeDeck(deck);
        var deletions = new JsonObject
        {
            // Said explicitly: "not in the deck" also describes an event this window never
            // saw, and the core must not treat the two as the same thing.
            ["deletedEvents"] = new JsonArray([.. (deletedEvents ?? new HashSet<ulong>()).Select(id => (JsonNode?)JsonValue.Create(id))]),
            ["deletedClips"] = new JsonArray([.. (deletedClips ?? new HashSet<string>()).Select(uid => (JsonNode?)JsonValue.Create(uid))]),
            ["angleEdited"] = angleEdited
        };
        return $"{{\"kind\":\"updateReplayDeck\",\"deck\":{body},\"edit\":{deletions.ToJsonString()}}}";
    }

    /// <summary>
    /// Events, playlists, the selected angle and channel playlist ids. Settings
    /// stay on the core, so this is what a remote deck sends.
    /// </summary>
    public static string SerializeDeck(JsonObject root)
    {
        var deck = new JsonObject();
        foreach (var name in new[]
                 {
                     "events", "playlists", "activeAngleId", "nextEventId", "nextPlaylistId",
                     "channels", "timelineCues"
                 })
        {
            if (root[name] is { } value)
                deck[name] = value.DeepClone();
        }
        return deck.ToJsonString();
    }


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
                    : 0,
                // A document written before the source existed is a scene angle.
                angle["source"]?.GetValue<string>() ?? "Scene",
                angle["inputId"]?.GetValue<ulong>() ?? 0,
                    ReplayAngleInfo.EncoderOf(angle),
                null,
                angle["audio"]?.GetValue<string>() ?? "Silent"));
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
                entry["protected"]?.GetValue<bool>() ?? false,
                (entry["tags"] as JsonArray ?? [])
                    .Select(tag => tag?.GetValue<string>() ?? "")
                    .Where(tag => tag.Length > 0)
                    .ToList()));
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
                    // Speed and gain are written as JSON numbers, which System.Text.Json
                    // types as Double: reading them as float throws on a document this
                    // host just wrote.
                    (float)(clip["speed"]?.GetValue<double>() ?? 1.0),
                    (float)(clip["gainDb"]?.GetValue<double>() ?? 0.0),
                    ReplayTransitionInfo.Read(clip["transition"]),
                    (clip["tags"] as JsonArray ?? [])
                        .Select(tag => tag?.GetValue<string>() ?? "")
                        .Where(tag => tag.Length > 0)
                        .ToList(),
                    clip["preservePitch"]?.GetValue<bool>() ?? true,
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

    /// <summary>
    /// When a recording's frame 0 was written, in milliseconds since the epoch. Null
    /// when the session does not know: the recording predates the timecode, or it is
    /// not in the list.
    /// </summary>
    public static ulong? RecordingStart(JsonObject root, string? uuid)
    {
        if (string.IsNullOrWhiteSpace(uuid))
            return null;
        foreach (var item in Array(root, "recordings").OfType<JsonObject>())
        {
            if (!string.Equals(item["uuid"]?.GetValue<string>(), uuid, StringComparison.Ordinal))
                continue;
            var started = item["createdMs"]?.GetValue<ulong>() ?? 0;
            return started == 0 ? null : started;
        }
        return null;
    }

    /// <summary>
    /// The wall-clock time of a frame: the recording started at `startMs` and this frame
    /// is `frames` into it. "HH:MM:SS:FF". A recording whose start is unknown falls back
    /// to the time from the recording's own beginning, which is what the deck showed
    /// before it knew the clock.
    /// </summary>
    public static string Timestamp(ulong? startMs, ulong frames, (uint Num, uint Den) fps)
    {
        if (startMs is not ulong start || start == 0)
            return Timecode(frames, fps);
        var rate = Rate(fps);
        var perSecond = (int)Math.Round(rate);
        if (perSecond <= 0)
            return Timecode(frames, fps);
        // The recording start is stored in UTC milliseconds; the operator reads the
        // clock on the wall, so the timecode is in this machine's zone.
        var local = DateTimeOffset.FromUnixTimeMilliseconds((long)start).ToLocalTime();
        var dayMs = local.TimeOfDay.TotalMilliseconds;
        var totalSeconds = (ulong)(dayMs / 1000) + frames / (ulong)perSecond;
        var frame = (int)(frames % (ulong)perSecond);
        return $"{totalSeconds / 3600 % 24:00}:{totalSeconds / 60 % 60:00}:{totalSeconds % 60:00}:{frame:00}";
    }

    /// <summary>
    /// Reads a wall-clock timecode ("HH:MM:SS:FF", optionally prefixed with the date
    /// part discarded here) back into frames from the recording's start. Falls back to
    /// <see cref="TryParseTimecode"/> when the recording's start is unknown.
    /// </summary>
    public static bool TryParseTimestamp(
        string? text,
        ulong? startMs,
        (uint Num, uint Den) fps,
        out ulong frames)
    {
        frames = 0;
        if (startMs is not ulong start || start == 0)
            return TryParseTimecode(text, fps, out frames);
        var rate = Rate(fps);
        var perSecond = (int)Math.Round(rate);
        if (perSecond <= 0)
            return TryParseTimecode(text, fps, out frames);
        var parts = (text ?? "").Trim().Split(':', '.', ';');
        if (parts.Length != 4
            || !uint.TryParse(parts[0], out var hours)
            || !uint.TryParse(parts[1], out var minutes)
            || !uint.TryParse(parts[2], out var seconds)
            || !uint.TryParse(parts[3], out var frame)
            || frame >= (uint)perSecond)
            return false;
        var local = DateTimeOffset.FromUnixTimeMilliseconds((long)start).ToLocalTime();
        var startSeconds = (long)local.TimeOfDay.TotalSeconds;
        // The recording's own frames per day: the timecode wraps at midnight, so the
        // typed value may be a little before or after the recording's start.
        var typed = (long)hours * 3600 + (long)minutes * 60 + seconds;
        var delta = typed - startSeconds;
        if (delta < -12 * 3600)
            delta += 24 * 3600;
        if (delta > 12 * 3600)
            delta -= 24 * 3600;
        if (delta < 0)
            return false;
        frames = (ulong)delta * (ulong)perSecond + frame;
        return true;
    }
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

    /// <summary>
    /// Reads a timecode back into frames: "HH:MM:SS:FF" (the shape <see cref="Timecode"/>
    /// writes) or a plain frame count. Used where the operator types an in or out point.
    /// </summary>
    public static bool TryParseTimecode(string? text, (uint Num, uint Den) fps, out ulong frames)
    {
        frames = 0;
        var trimmed = (text ?? "").Trim();
        if (trimmed.Length == 0)
            return false;
        if (ulong.TryParse(trimmed, out var plain))
        {
            frames = plain;
            return true;
        }
        var parts = trimmed.Split(':', '.', ';');
        if (parts.Length != 4)
            return false;
        if (!uint.TryParse(parts[0], out var hours)
            || !uint.TryParse(parts[1], out var minutes)
            || !uint.TryParse(parts[2], out var seconds)
            || !uint.TryParse(parts[3], out var frame))
            return false;
        var perSecond = (int)Math.Round(Rate(fps));
        if (perSecond <= 0 || frame >= (uint)perSecond)
            return false;
        frames = ((ulong)hours * 3600 + (ulong)minutes * 60 + (ulong)seconds) * (ulong)perSecond + frame;
        return true;
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

    /// <summary>
    /// Angle the operator is on: the one they picked, or the first angle when the
    /// document has never had one chosen. The mixer treats the first angle as the
    /// default (live follows it, `jump_to_now` records it), so the deck reads the
    /// same way instead of refusing to name the camera a clip came from.
    /// </summary>
    public static uint ActiveAngle(JsonObject root)
    {
        var active = root["activeAngleId"]?.GetValue<uint>() ?? 0;
        if (active != 0)
            return active;
        foreach (var angle in Angles(root))
            return angle.Id;
        return 0;
    }

    /// <summary>
    /// Drives the recording is allowed to use. A recorder without one writes
    /// nothing, which is what the deck checks before it starts.
    /// </summary>
    /// <summary>
    /// The recordings this document uses, by id. The list is the authority: a clip's
    /// recording is played from where the session says it lives.
    /// </summary>
    public static IReadOnlyList<(string Uuid, string Storage)> Recordings(JsonObject root)
    {
        var result = new List<(string, string)>();
        foreach (var item in Array(root, "recordings").OfType<JsonObject>())
        {
            var uuid = item["uuid"]?.GetValue<string>()?.Trim() ?? "";
            if (uuid.Length == 0)
                continue;
            result.Add((uuid, item["storage"]?.GetValue<string>()?.Trim() ?? ""));
        }
        return result;
    }

    /// <summary>
    /// Links a recording the session uses: called when a recording starts, so the id
    /// the clips carry is written down with the drive it went to.
    /// </summary>
    public static bool AddRecording(
        JsonObject root,
        string uuid,
        string storage,
        IReadOnlyList<uint> angles,
        uint width,
        uint height,
        uint fpsNum,
        uint fpsDen,
        ulong firstFrame,
        ulong startedMs = 0)
    {
        uuid = uuid.Trim();
        if (uuid.Length == 0)
            return false;
        var recordings = root["recordings"] as JsonArray ?? new JsonArray();
        root["recordings"] = recordings;
        JsonObject? entry = null;
        foreach (var item in recordings.OfType<JsonObject>())
        {
            if (string.Equals(item["uuid"]?.GetValue<string>(), uuid, StringComparison.Ordinal))
                entry = item;
        }
        entry ??= new JsonObject { ["uuid"] = uuid };
        if (entry.Parent is null)
            recordings.Add(entry);
        var ids = new JsonArray();
        foreach (var angle in angles)
            ids.Add(angle);
        entry["storage"] = storage;
        entry["width"] = width;
        entry["height"] = height;
        entry["fpsNum"] = fpsNum;
        entry["fpsDen"] = fpsDen;
        entry["firstFrame"] = firstFrame;
        entry["angles"] = ids;
        entry["lastFrame"] ??= 0UL;
        // The wall clock of frame 0: the timecode the deck shows. Kept when the caller
        // does not know it (a session written before it existed).
        if (startedMs != 0 || entry["createdMs"] is null)
        {
            entry["createdMs"] = startedMs != 0
                ? startedMs
                : (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        }
        return true;
    }

    /// <summary>
    /// Links the recordings a session written before this list names only on its
    /// clips. One direct path check at load; afterwards the list is the authority and
    /// nothing searches a drive for a recording.
    /// </summary>
    public static bool EnsureRecordings(Session session, JsonObject root)
    {
        var known = Recordings(root).Select(item => item.Uuid).ToHashSet(StringComparer.Ordinal);
        var storages = Storages(root);
        var referenced = new List<string>();
        foreach (var list in Playlists(root))
            foreach (var clip in Clips(root, list.Id))
                if (clip.RecordingUuid.Length > 0)
                    referenced.Add(clip.RecordingUuid);
        foreach (var entry in Events(root))
            if (entry.RecordingUuid.Length > 0)
                referenced.Add(entry.RecordingUuid);
        var added = false;
        foreach (var uuid in referenced.Distinct(StringComparer.Ordinal))
        {
            if (known.Contains(uuid))
                continue;
            var storage = storages.FirstOrDefault(path => Directory.Exists(Path.Combine(path, uuid)));
            if (storage is null)
                continue;
            if (AddRecording(
                    root,
                    uuid,
                    storage,
                    Angles(root).Select(angle => angle.Id).ToList(),
                    root["format"]?["width"]?.GetValue<uint>() ?? 0,
                    root["format"]?["height"]?.GetValue<uint>() ?? 0,
                    root["format"]?["fpsNum"]?.GetValue<uint>() ?? 0,
                    root["format"]?["fpsDen"]?.GetValue<uint>() ?? 0,
                    0))
            {
                added = true;
            }
        }
        if (added)
            session.ReplayJson = Serialize(root);
        return added;
    }
    public static IReadOnlyList<string> Storages(JsonObject root)
    {
        var result = new List<string>();
        foreach (var item in Array(root, "storage"))
        {
            if (item is not JsonObject storage)
                continue;
            var path = storage["path"]?.GetValue<string>() ?? "";
            if (!string.IsNullOrWhiteSpace(path))
                result.Add(path);
        }
        return result;
    }

    public static bool AutoAddToPlaylist(JsonObject root) =>
        root["autoAddToPlaylist"]?.GetValue<bool>() ?? false;

    /// <summary>
    /// Whether playout decodes ahead of the picture. On unless the document says
    /// otherwise; a document written before the setting existed plays with it on.
    /// </summary>
    public static bool DecodeAhead(JsonObject root) =>
        root["decodeAhead"]?.GetValue<bool>() ?? true;

    /// <summary>Whether the mixer writes per-stage playout timings to its log.</summary>
    public static bool Profiling(JsonObject root) =>
        root["profiling"]?.GetValue<bool>() ?? false;

    /// <summary>
    /// Whether the document is a recording setup. The control layer requires a drive
    /// and an angle, so the deck reads the same flag it wrote.
    /// </summary>
    public static bool Enabled(JsonObject root) =>
        root["enabled"]?.GetValue<bool>() ?? false;

    /// <summary>
    /// How many recordings the events and clips name that are not on any drive.
    /// The timecodes stay in the session either way; this is what the deck says
    /// when the files themselves are gone.
    /// </summary>
    public static int MissingRecordingCount(JsonObject root, string? recordingNow = null)
    {
        // The session says where each recording lives; a recording it does not list is
        // not playable, and so is one whose directory is gone. Nothing is searched.
        var storageOf = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var (uuid, storage) in Recordings(root))
            storageOf[uuid] = storage;
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var missing = 0;
        void Consider(string? uuid)
        {
            uuid = uuid?.Trim();
            if (string.IsNullOrEmpty(uuid) || !seen.Add(uuid))
                return;
            if (!string.IsNullOrEmpty(recordingNow)
                && string.Equals(uuid, recordingNow, StringComparison.OrdinalIgnoreCase))
                return;
            if (storageOf.TryGetValue(uuid, out var storage)
                && storage.Length > 0
                && Directory.Exists(Path.Combine(storage, uuid)))
                return;
            missing++;
        }
        foreach (var entry in Events(root))
            Consider(entry.RecordingUuid);
        foreach (var list in Playlists(root))
            foreach (var clip in Clips(root, list.Id))
                Consider(clip.RecordingUuid);
        return missing;
    }

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
    /// <summary>
    /// Files an event onto a playlist as a clip, on the playlist's default transition
    /// so an operator who set "always dissolve" does not edit every clip. Both the
    /// auto-add after a mark and the event list's own button come through here, so
    /// the two cannot drift apart.
    /// </summary>
    /// <returns>The new clip's uid, or null when the playlist is not there.</returns>
    public static string? AppendClip(
        JsonObject root,
        ulong playlistId,
        ulong eventId,
        string recordingUuid,
        ulong inFrame,
        ulong outFrame,
        uint angleId)
    {
        var playlist = Playlist(root, playlistId);
        if (playlist is null)
            return null;
        var clips = playlist["clips"] as JsonArray ?? new JsonArray();
        var fallback = playlist["defaultTransition"] as JsonObject;
        var uid = UniqueClipUid(root, eventId);
        clips.Add(new JsonObject
        {
            ["uid"] = uid,
            ["source"] = "Angle",
            ["angleId"] = angleId,
            ["recordingUuid"] = recordingUuid,
            ["inFrame"] = inFrame,
            ["outFrame"] = outFrame,
            ["speed"] = 1.0,
            ["gainDb"] = 0.0,
            ["preservePitch"] = true,
            ["transition"] = fallback is null
                ? new JsonObject { ["kind"] = "Cut", ["timing"] = "Overwrap", ["durationFrames"] = 0U }
                : new JsonObject
                {
                    ["kind"] = fallback["kind"]?.GetValue<string>() ?? "Cut",
                    ["timing"] = fallback["timing"]?.GetValue<string>() ?? "Overwrap",
                    ["durationFrames"] = fallback["durationFrames"]?.GetValue<uint>() ?? 0
                }
        });
        playlist["clips"] = clips;
        playlist["revision"] = (playlist["revision"]?.GetValue<ulong>() ?? 0) + 1;
        return uid;
    }

    /// <summary>
    /// The playlist a channel should add to: the one it already plays, or a new one.
    ///
    /// A project can hold several channels, and each plays its own list. Returning the first
    /// playlist no matter what — which this did — filed channel B's clips into channel A's
    /// list, so an operator on B watched their clip appear somewhere they were not looking.
    /// </summary>
    /// <param name="root">The Replay document.</param>
    /// <param name="channelIndex">The channel the operator is working on.</param>
    public static ulong EnsurePlaylist(JsonObject root, uint channelIndex)
    {
        if (ChannelPlaylist(root, channelIndex) is { } existing && Playlist(root, existing) is not null)
            return existing;
        var id = ReservePlaylistId(root);
        var list = new JsonArray();
        var document = root["playlists"] as JsonArray ?? new JsonArray();
        document.Add(new JsonObject
        {
            ["id"] = id,
            ["name"] = $"Replay {(char)('A' + (int)Math.Min(channelIndex, 25))}",
            ["revision"] = 1UL,
            ["clips"] = list
        });
        root["playlists"] = document;
        // Point the channel at it, so the two do not drift the moment it is created.
        SetChannelPlaylist(root, channelIndex, id);
        return id;
    }

    /// <summary>
    /// The playlist to add an event to: the channel's own.
    ///
    /// The channel decides, not the picker. An operator who has just switched to channel B is
    /// still looking at a picker that shows the old list for a moment, and taking that list
    /// filed B's clip on A's playlist — the clip was then played, and exported, from a channel
    /// the operator was not using.
    /// </summary>
    /// <param name="root">The Replay document.</param>
    /// <param name="openPlaylistId">The playlist the deck is showing, or 0.</param>
    /// <param name="channelIndex">The channel the deck is on.</param>
    public static ulong EnsurePlaylist(JsonObject root, ulong openPlaylistId, uint channelIndex)
    {
        if (ChannelPlaylist(root, channelIndex) is { } playing && Playlist(root, playing) is not null)
            return playing;
        // A channel with no list of its own takes the one on screen only when no other channel
        // already plays it: a one-channel project then behaves as it always did, and a second
        // channel gets a list of its own instead of sharing the first one's.
        if (openPlaylistId != 0
            && Playlist(root, openPlaylistId) is not null
            && Channels(root).All(channel => channel.PlaylistId != openPlaylistId))
        {
            SetChannelPlaylist(root, channelIndex, openPlaylistId);
            return openPlaylistId;
        }
        return EnsurePlaylist(root, channelIndex);
    }

    /// <summary>Every playlist the project holds, in the order the document lists them.</summary>
    public static IReadOnlyList<ReplayPlaylistInfo> Playlists(JsonObject root)
    {
        var result = new List<ReplayPlaylistInfo>();
        foreach (var item in Array(root, "playlists"))
        {
            if (item is not JsonObject playlist)
                continue;
            var id = playlist["id"]?.GetValue<ulong>() ?? 0;
            if (id == 0)
                continue;
            var name = playlist["name"]?.GetValue<string>() ?? "";
            result.Add(new ReplayPlaylistInfo(
                id,
                string.IsNullOrWhiteSpace(name) ? $"Playlist {id}" : name,
                (playlist["clips"] as JsonArray)?.Count ?? 0));
        }
        return result;
    }

    /// <summary>
    /// The playlist a channel plays. A project can hold several, and the channel is what
    /// points at one, so the deck shows the channel's rather than whichever happens to
    /// be first.
    /// </summary>
    /// <summary>
    /// The playlist a channel is pointed at, or null when it is pointed at none.
    ///
    /// A channel that names no playlist is not "playing the first one": it is playing nothing,
    /// and the caller decides what to do about that. Answering with the first list made the
    /// project look as though channel B already had a list — it had A's — so nothing ever made
    /// B one, and B's clips were filed and played from A.
    /// </summary>
    public static ulong? ChannelPlaylist(JsonObject root, uint channelIndex)
    {
        foreach (var item in Array(root, "channels"))
        {
            if (item is not JsonObject channel)
                continue;
            if (channel["index"]?.GetValue<uint>() != channelIndex)
                continue;
            var id = channel["playlistId"]?.GetValue<ulong>() ?? 0;
            if (id != 0)
                return Playlist(root, id) is not null ? id : null;
            return null;
        }
        return null;
    }

    /// <summary>Points a channel at a playlist, creating the channel entry when needed.</summary>
    public static void SetChannelPlaylist(JsonObject root, uint channelIndex, ulong playlistId)
    {
        var channels = root["channels"] as JsonArray ?? new JsonArray();
        foreach (var item in channels)
        {
            if (item is JsonObject channel && channel["index"]?.GetValue<uint>() == channelIndex)
            {
                channel["playlistId"] = playlistId;
                root["channels"] = channels;
                return;
            }
        }
        channels.Add(new JsonObject
        {
            ["index"] = channelIndex,
            ["playlistId"] = playlistId,
            ["inputId"] = 0UL
        });
        root["channels"] = channels;
    }

    /// <summary>Adds an empty playlist and returns its id.</summary>
    public static ulong AddPlaylist(JsonObject root, string name)
    {
        var id = ReservePlaylistId(root);
        var document = root["playlists"] as JsonArray ?? new JsonArray();
        document.Add(new JsonObject
        {
            ["id"] = id,
            ["name"] = string.IsNullOrWhiteSpace(name) ? $"Playlist {id}" : name.Trim(),
            ["revision"] = 1UL,
            ["clips"] = new JsonArray()
        });
        root["playlists"] = document;
        return id;
    }

    /// <summary>Renames a playlist. An empty name is refused rather than saved.</summary>
    public static bool RenamePlaylist(JsonObject root, ulong playlistId, string name)
    {
        var trimmed = name.Trim();
        if (trimmed.Length == 0)
            return false;
        foreach (var item in Array(root, "playlists"))
        {
            if (item is JsonObject playlist && playlist["id"]?.GetValue<ulong>() == playlistId)
            {
                playlist["name"] = trimmed;
                return true;
            }
        }
        return false;
    }

    /// <summary>How many channels play this playlist, so it is not deleted under one.</summary>
    public static int ChannelsUsingPlaylist(JsonObject root, ulong playlistId)
    {
        var count = 0;
        foreach (var item in Array(root, "channels"))
        {
            if (item is JsonObject channel && channel["playlistId"]?.GetValue<ulong>() == playlistId)
                count++;
        }
        return count;
    }

    /// <summary>Removes a playlist and its clips.</summary>
    public static bool RemovePlaylist(JsonObject root, ulong playlistId)
    {
        if (root["playlists"] is not JsonArray playlists)
            return false;
        for (var index = 0; index < playlists.Count; index++)
        {
            if (playlists[index] is JsonObject playlist && playlist["id"]?.GetValue<ulong>() == playlistId)
            {
                playlists.RemoveAt(index);
                return true;
            }
        }
        return false;
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







