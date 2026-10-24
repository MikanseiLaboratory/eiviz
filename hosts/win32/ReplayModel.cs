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
    string RecordMode = "Composite",
    string Encoder = "Cpu",
    JsonObject? Json = null)
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

internal sealed record ReplayEventInfo(ulong Id, string RecordingUuid, ulong InFrame, ulong OutFrame, string Name, bool Protected);

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
    /// The document to publish from the deck.
    ///
    /// Replay Setup owns the drives, the angles, the format and the flags. The deck
    /// owns events, playlists and which playlist a channel plays. Publishing the
    /// deck's whole snapshot put a Replay Setup apply back to whatever the deck had
    /// loaded.
    /// </summary>
    public static JsonObject WithDeckEdits(
        Session session,
        JsonObject deck,
        bool angleEdited = true,
        IReadOnlySet<ulong>? deletedEvents = null,
        IReadOnlySet<string>? deletedClips = null)
    {
        var current = Parse(session);
        if (current is null)
            return deck;
        // The deck window owns the events and the playlists, but its copy can be older
        // than the document: an API mark, a remote edit or a refused publish leaves the
        // window holding fewer events than are on air. Replacing the arrays with that
        // copy deleted those events (and then the clips they carried), which the log
        // showed as "events 3->0" on a plain deck save. Merge by id instead, and let
        // the window say which ones it meant to delete.
        MergeEvents(current, deck, deletedEvents);
        MergePlaylists(current, deck, deletedClips);
        if (angleEdited && deck["activeAngleId"] is { } angle)
            current["activeAngleId"] = angle.DeepClone();
        // The counters only ever move forward: a stale window must not reuse an id.
        foreach (var name in new[] { "nextEventId", "nextPlaylistId" })
        {
            var mine = current[name]?.GetValue<ulong>() ?? 0;
            var theirs = deck[name]?.GetValue<ulong>() ?? 0;
            current[name] = Math.Max(mine, theirs);
        }
        CopyChannelPlaylists(deck, current);
        return current;
    }


    /// <summary>
    /// Applies the window's events onto the document by id. An event the window does
    /// not have is kept unless the window deleted it.
    /// </summary>
    private static void MergeEvents(JsonObject current, JsonObject deck, IReadOnlySet<ulong>? deleted)
    {
        if (deck["events"] is not JsonArray incoming)
            return;
        var byId = new Dictionary<ulong, JsonObject>();
        var order = new List<ulong>();
        if (current["events"] is JsonArray existing)
        {
            foreach (var item in existing.OfType<JsonObject>())
            {
                var id = item["id"]?.GetValue<ulong>() ?? 0;
                if (deleted?.Contains(id) == true || byId.ContainsKey(id))
                    continue;
                byId[id] = (JsonObject)item.DeepClone();
                order.Add(id);
            }
        }
        foreach (var item in incoming.OfType<JsonObject>())
        {
            var id = item["id"]?.GetValue<ulong>() ?? 0;
            if (deleted?.Contains(id) == true)
                continue;
            if (!byId.ContainsKey(id))
                order.Add(id);
            byId[id] = (JsonObject)item.DeepClone();
        }
        var merged = new JsonArray();
        foreach (var id in order)
            merged.Add(byId[id]);
        current["events"] = merged;
    }

    /// <summary>
    /// Applies the window's playlists by id and its clips by uid, so a clip the API
    /// added (or one recorded a moment ago) survives a save from this window.
    /// </summary>
    private static void MergePlaylists(
        JsonObject current,
        JsonObject deck,
        IReadOnlySet<string>? deletedClips)
    {
        if (deck["playlists"] is not JsonArray incoming)
            return;
        if (current["playlists"] is not JsonArray playlists)
        {
            current["playlists"] = incoming.DeepClone();
            return;
        }
        foreach (var deckList in incoming.OfType<JsonObject>())
        {
            var id = deckList["id"]?.GetValue<ulong>() ?? 0;
            var existing = playlists.OfType<JsonObject>()
                .FirstOrDefault(item => (item["id"]?.GetValue<ulong>() ?? 0) == id);
            if (existing is null)
            {
                playlists.Add(deckList.DeepClone());
                continue;
            }
            var byUid = new Dictionary<string, JsonObject>();
            var order = new List<string>();
            if (existing["clips"] is JsonArray currentClips)
            {
                foreach (var clip in currentClips.OfType<JsonObject>())
                {
                    var uid = clip["uid"]?.GetValue<string>() ?? "";
                    if (deletedClips?.Contains(uid) == true || byUid.ContainsKey(uid))
                        continue;
                    byUid[uid] = (JsonObject)clip.DeepClone();
                    order.Add(uid);
                }
            }
            if (deckList["clips"] is JsonArray deckClips)
            {
                foreach (var clip in deckClips.OfType<JsonObject>())
                {
                    var uid = clip["uid"]?.GetValue<string>() ?? "";
                    if (deletedClips?.Contains(uid) == true)
                        continue;
                    if (!byUid.ContainsKey(uid))
                        order.Add(uid);
                    byUid[uid] = (JsonObject)clip.DeepClone();
                }
            }
            var mergedClips = new JsonArray();
            foreach (var uid in order)
                mergedClips.Add(byUid[uid]);
            // Only a real edit moves the revision: the mixer reads it to decide whether
            // the running clip has to be rebuilt, and bumping it on every save reset the
            // playlist under the playhead.
            var before = (existing["clips"] as JsonArray)?.ToJsonString() ?? "";
            var after = mergedClips.ToJsonString();
            existing["clips"] = mergedClips;
            foreach (var name in new[] { "name", "defaultTransition" })
            {
                if (deckList[name] is { } value)
                    existing[name] = value.DeepClone();
            }
            var mine = existing["revision"]?.GetValue<ulong>() ?? 0;
            var theirs = deckList["revision"]?.GetValue<ulong>() ?? 0;
            existing["revision"] = string.Equals(before, after, StringComparison.Ordinal)
                ? Math.Max(mine, theirs)
                : Math.Max(mine, theirs) + 1;
        }
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

    /// <summary>
    /// Points each existing channel at the playlist the deck has for that index.
    /// Channels themselves belong to the Replay inputs, so a deck snapshot must not
    /// add or remove them.
    /// </summary>
    private static void CopyChannelPlaylists(JsonObject deck, JsonObject current)
    {
        if (current["channels"] is not JsonArray channels)
            return;
        foreach (var item in channels)
        {
            if (item is not JsonObject channel)
                continue;
            if (channel["index"] is not JsonValue indexValue || !indexValue.TryGetValue<uint>(out var index))
                continue;
            var source = ChannelObject(deck, index);
            if (source is null)
                continue;
            if (source["playlistId"] is { } playlist)
                channel["playlistId"] = playlist.DeepClone();
            else
                channel.Remove("playlistId");
        }
    }

    private static JsonObject? ChannelObject(JsonObject root, uint index)
    {
        if (root["channels"] is not JsonArray channels)
            return null;
        foreach (var item in channels)
        {
            if (item is not JsonObject channel)
                continue;
            if (channel["index"] is JsonValue indexValue
                && indexValue.TryGetValue<uint>(out var found)
                && found == index)
                return channel;
        }
        return null;
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
                angle["recordMode"]?.GetValue<string>() ?? "Composite",
                ReplayAngleInfo.EncoderOf(angle)));
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
        var drives = Storages(root);
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var missing = 0;
        void Consider(string? uuid)
        {
            if (string.IsNullOrWhiteSpace(uuid) || !seen.Add(uuid))
                return;
            if (!string.IsNullOrEmpty(recordingNow)
                && string.Equals(uuid, recordingNow, StringComparison.OrdinalIgnoreCase))
                return;
            var present = drives.Any(drive =>
                Directory.Exists(Path.Combine(drive, uuid)));
            if (!present)
                missing++;
        }
        foreach (var entry in Events(root))
            Consider(entry.RecordingUuid);
        foreach (var list in Playlists(root))
        {
            foreach (var clip in Clips(root, list.Id))
                Consider(clip.RecordingUuid);
        }
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
            ["revision"] = 1UL,
            ["clips"] = list
        });
        root["playlists"] = document;
        return id;
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
                return id;
            break;
        }
        // A channel that names none, or names one that is gone, plays the first.
        return Playlists(root).FirstOrDefault()?.Id;
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







