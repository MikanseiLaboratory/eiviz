import EivizMixer
import Foundation

extension JSONValue {
    var objectValue: [String: JSONValue]? {
        if case .object(let value) = self { return value }
        return nil
    }

    var arrayValue: [JSONValue]? {
        if case .array(let value) = self { return value }
        return nil
    }

    var stringValue: String? {
        if case .string(let value) = self { return value }
        return nil
    }

    var numberValue: Double? {
        if case .number(let value) = self { return value }
        return nil
    }

    var boolValue: Bool? {
        if case .bool(let value) = self { return value }
        return nil
    }

    var uint32Value: UInt32? {
        guard let value = numberValue, value >= 0 else { return nil }
        return UInt32(value.rounded())
    }

    var uint64Value: UInt64? {
        guard let value = numberValue, value >= 0 else { return nil }
        return UInt64(value.rounded())
    }
}

/// What became of a Mark Out. The host that took the mark owns the event; a core
/// that owns the recording creates the event itself and the session document
/// brings it back. `refused` leaves the range open for another try.
enum ReplayMarkOutcome {
    case refused
    case local(uuid: String, inFrame: UInt64, outFrame: UInt64)
    case remote
}

/// The bits the mixer raises in `EivizReplayStats.issues`, and the words for
/// them. The severity colour only says something is wrong; an operator needs to
/// read what.
enum ReplayIssues {
    static let cpuEncode: UInt32 = 1 << 0
    static let storageWrite: UInt32 = 1 << 1
    static let readback: UInt32 = 1 << 2
    static let storageLow: UInt32 = 1 << 3
    static let storageFull: UInt32 = 1 << 4
    static let framesDropped: UInt32 = 1 << 5
    static let stoppedByPolicy: UInt32 = 1 << 6
    static let proUnavailable: UInt32 = 1 << 7

    /// Most urgent first, because the list is read from the front.
    private static let named: [(bit: UInt32, key: String)] = [
        (storageFull, "replay.issueStorageFull"),
        (storageLow, "replay.issueStorageLow"),
        (storageWrite, "replay.issueStorageWrite"),
        (cpuEncode, "replay.issueCpuEncode"),
        (readback, "replay.issueReadback"),
        (framesDropped, "replay.issueDropped"),
        (stoppedByPolicy, "replay.issueStopped"),
        (proUnavailable, "replay.issuePro"),
    ]

    static func text(_ issues: UInt32) -> String {
        let names = named.filter { issues & $0.bit != 0 }.map { L10n.t($0.key) }
        return names.isEmpty ? "" : L10n.format("replay.issues", names.joined(separator: ", "))
    }
}

struct ReplayChannelInfo: Identifiable, Hashable {
    let index: UInt32
    let inputId: UInt64
    /// Playlist this channel plays, when it selects one.
    var playlistId: UInt64? = nil

    var id: UInt32 { index }

    var letter: String {
        let letters = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZ")
        return index < 26 ? String(letters[Int(index)]) : "\(index)"
    }

    var label: String { "Replay \(letter)" }

    /// Source the channel paints into: the input that owns it, or the reserved
    /// channel id for a document written without one.
    var sourceId: UInt64 {
        inputId != 0 ? inputId : EIVIZ_REPLAY_CHANNEL_BASE | UInt64(index + 1)
    }
}

/// Must match `REPLAY_CHANNEL_BASE` in `eiviz_control::session::replay`. The band
/// is reserved so a Replay channel can never collide with a normal input.
let EIVIZ_REPLAY_CHANNEL_BASE: UInt64 = 0x0008_0000

struct ReplayAngleInfo: Identifiable, Hashable {
    let id: UInt32
    let name: String
    let sceneId: UInt64
    /// Mixing unit whose program mix feeds this angle. Zero means silent.
    let audioUnitId: UInt64

    var sceneGpuId: UInt64 { EIVIZ_SCENE_BASE | sceneId }

    var label: String { name.isEmpty ? "Angle \(id)" : name }
}

struct ReplayEventInfo: Identifiable, Hashable {
    let id: UInt64
    let recordingUuid: String
    let inFrame: UInt64
    let outFrame: UInt64
    let name: String
    let protected: Bool
}

/// Transition out of a clip. `Overwrap` overlaps the two clips, `Additive`
/// inserts the duration between them.
struct ReplayTransitionInfo: Hashable {
    var kind: String
    var timing: String
    var durationFrames: UInt32

    static let cut = ReplayTransitionInfo(kind: "Cut", timing: "Overwrap", durationFrames: 0)

    var isCut: Bool { kind == "Cut" || durationFrames == 0 }

    var label: String { isCut ? "Cut" : "\(kind) \(durationFrames)f \(timing)" }

    var value: JSONValue {
        .object([
            "kind": .string(kind),
            "timing": .string(timing),
            "durationFrames": .number(Double(durationFrames)),
        ])
    }

    /// Accepts both the structured form and the bare kind name older sessions
    /// stored.
    static func read(_ node: JSONValue?) -> ReplayTransitionInfo {
        if let name = node?.stringValue {
            return ReplayTransitionInfo(kind: name, timing: "Overwrap", durationFrames: 0)
        }
        guard let object = node?.objectValue else { return .cut }
        return ReplayTransitionInfo(
            kind: object["kind"]?.stringValue ?? "Cut",
            timing: object["timing"]?.stringValue ?? "Overwrap",
            durationFrames: object["durationFrames"]?.uint32Value ?? 0
        )
    }
}

struct ReplayClipInfo: Identifiable, Hashable {
    let uid: String
    let angleId: UInt32
    let recordingUuid: String
    let inFrame: UInt64
    let outFrame: UInt64
    let speed: Double
    let gainDb: Double
    let transition: ReplayTransitionInfo
    let tags: [String]
    /// Hold the pitch when the clip plays at something other than normal speed.
    let preservePitch: Bool
    /// Audio source: `FollowAngle`, `Silent`, or `Angle`.
    let audioKind: String
    let audioAngleId: UInt32

    var id: String { uid }
}

/// Reads and edits the replay document the session carries. The shape is the one
/// `eiviz_control::session::replay::ReplayDto` serializes, so both hosts and the
/// core stay interchangeable.
/// JSON escaping for the strings this host builds by hand. The mixer parses them
/// with a real JSON reader, so a path with a quote or a backslash must survive.
enum ReplayJson {
    static func string(_ value: String) -> String {
        var escaped = "\""
        for character in value {
            switch character {
            case "\"": escaped += "\\\""
            case "\\": escaped += "\\\\"
            case "\n": escaped += "\\n"
            case "\r": escaped += "\\r"
            case "\t": escaped += "\\t"
            default: escaped.append(character)
            }
        }
        return escaped + "\""
    }
}

/// State of an MP4 export, matching the module's `EXPORT_*`.
enum ReplayExportState: UInt32 {
    case idle = 0
    case running = 1
    case done = 2
    case failed = 3
    case cancelled = 4
}

/// Progress of an MP4 export, as the mixer reports it.
struct ReplayExportProgress {
    let state: ReplayExportState
    let done: UInt64
    let total: UInt64
    let skipped: UInt64
    let bytes: UInt64
    let message: String

    init?(json: String) {
        guard let data = json.data(using: .utf8),
              let node = try? JSONDecoder().decode(JSONValue.self, from: data),
              let object = node.objectValue
        else {
            return nil
        }
        state = ReplayExportState(rawValue: object["state"]?.uint32Value ?? 0) ?? .idle
        done = object["done"]?.uint64Value ?? 0
        total = object["total"]?.uint64Value ?? 0
        skipped = object["skipped"]?.uint64Value ?? 0
        bytes = object["bytes"]?.uint64Value ?? 0
        message = object["message"]?.stringValue ?? ""
    }
}

struct ReplayDocument {
    var raw: [String: JSONValue]

    init?(_ node: JSONValue?) {
        guard let object = node?.objectValue else { return nil }
        raw = object
    }

    init(raw: [String: JSONValue]) {
        self.raw = raw
    }

    /// The document a project gets when its first Replay input is added. Replay
    /// never appears in a session without this.
    static func empty() -> ReplayDocument {
        ReplayDocument(raw: [
            "enabled": .bool(false),
            "storage": .array([]),
            "angles": .array([]),
            "events": .array([]),
            "playlists": .array([]),
            "channels": .array([]),
            "autoAddToPlaylist": .bool(false),
        ])
    }

    mutating func addChannel(index: UInt32, inputId: UInt64) {
        var channels = array("channels")
        channels.append(.object([
            "index": .number(Double(index)),
            "inputId": .number(Double(inputId)),
        ]))
        raw["channels"] = .array(channels)
    }

    /// Drops the channel an input owns. Returns true when no channel is left, so
    /// the caller can remove the document entirely.
    mutating func removeChannel(inputId: UInt64) -> Bool {
        let remaining = array("channels").filter { item in
            (item.objectValue?["inputId"]?.uint64Value ?? 0) != inputId
        }
        raw["channels"] = .array(remaining)
        return remaining.isEmpty
    }

    var value: JSONValue { .object(raw) }

    /// The document as the mixer and the session file want it.
    var json: String {
        guard let data = try? JSONEncoder().encode(value) else { return "{}" }
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    private func array(_ key: String) -> [JSONValue] { raw[key]?.arrayValue ?? [] }

    var channels: [ReplayChannelInfo] {
        array("channels")
            .compactMap { item -> ReplayChannelInfo? in
                guard let object = item.objectValue else { return nil }
                return ReplayChannelInfo(
                    index: object["index"]?.uint32Value ?? 0,
                    inputId: object["inputId"]?.uint64Value ?? 0,
                    playlistId: object["playlistId"]?.uint64Value
                )
            }
            .sorted { $0.index < $1.index }
    }

    var angles: [ReplayAngleInfo] {
        array("angles").compactMap { item -> ReplayAngleInfo? in
            guard let object = item.objectValue else { return nil }
            return ReplayAngleInfo(
                id: object["id"]?.uint32Value ?? 0,
                name: object["name"]?.stringValue ?? "",
                sceneId: object["sceneId"]?.uint64Value ?? 0,
                // Silence is the only other source, so an explicit unit id is what
                // matters.
                audioUnitId: (object["audio"]?.stringValue ?? "Silent").caseInsensitiveCompare("Unit") == .orderedSame
                    ? (object["audioUnitId"]?.uint64Value ?? 0)
                    : 0
            )
        }
    }

    var events: [ReplayEventInfo] {
        array("events").compactMap { item -> ReplayEventInfo? in
            guard let object = item.objectValue else { return nil }
            return ReplayEventInfo(
                id: object["id"]?.uint64Value ?? 0,
                recordingUuid: object["recordingUuid"]?.stringValue ?? "",
                inFrame: object["inFrame"]?.uint64Value ?? 0,
                outFrame: object["outFrame"]?.uint64Value ?? 0,
                name: object["name"]?.stringValue ?? "",
                protected: object["protected"]?.boolValue ?? false
            )
        }
    }

    var activeAngleId: UInt32 { raw["activeAngleId"]?.uint32Value ?? 0 }

    /// Angle the operator is on: the one they picked, or the first angle when the
    /// document has never had one chosen. The mixer treats the first angle as the
    /// default (live follows it, `jump_to_now` records it), so the deck reads the
    /// same way instead of refusing to name the camera a clip came from.
    var activeAngle: UInt32 {
        let chosen = activeAngleId
        if chosen != 0 { return chosen }
        return angles.first?.id ?? 0
    }

    /// Default transition of a playlist, or cut when it has none.
    func defaultTransition(playlistId: UInt64) -> ReplayTransitionInfo {
        guard let playlists = raw["playlists"]?.arrayValue else { return .cut }
        for item in playlists {
            guard let playlist = item.objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            return ReplayTransitionInfo.read(playlist["defaultTransition"])
        }
        return .cut
    }

    var autoAddToPlaylist: Bool { raw["autoAddToPlaylist"]?.boolValue ?? false }
    var enabled: Bool { raw["enabled"]?.boolValue ?? false }

    var profile: String {
        raw["format"]?.objectValue?["profile"]?.stringValue ?? "Sq"
    }

    var buffering: (queue: UInt32, buffer: UInt32, reserve: UInt32, overflow: String) {
        let object = raw["buffering"]?.objectValue ?? [:]
        return (
            object["encodeQueueFrames"]?.uint32Value ?? 6,
            object["writeBufferMb"]?.uint32Value ?? 512,
            object["reserveMb"]?.uint32Value ?? 2048,
            object["overflowPolicy"]?.stringValue ?? "DropFrames"
        )
    }

    var retention: String { raw["retention"]?.stringValue ?? "StopWhenFull" }

    var fps: (num: UInt32, den: UInt32) {
        let format = raw["format"]?.objectValue ?? [:]
        return (
            format["fpsNum"]?.uint32Value ?? 0,
            format["fpsDen"]?.uint32Value ?? 0
        )
    }

    var firstPlaylistId: UInt64 {
        array("playlists").first?.objectValue?["id"]?.uint64Value ?? 0
    }

    /// Every playlist the document holds, so a caller can look at all clips.
    var playlistIds: [UInt64] {
        array("playlists").compactMap { $0.objectValue?["id"]?.uint64Value }
    }

    func clips(playlistId: UInt64) -> [ReplayClipInfo] {
        for item in array("playlists") {
            guard let playlist = item.objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            return (playlist["clips"]?.arrayValue ?? []).compactMap { entry -> ReplayClipInfo? in
                guard let clip = entry.objectValue else { return nil }
                return ReplayClipInfo(
                    uid: clip["uid"]?.stringValue ?? "",
                    angleId: clip["angleId"]?.uint32Value ?? 0,
                    recordingUuid: clip["recordingUuid"]?.stringValue ?? "",
                    inFrame: clip["inFrame"]?.uint64Value ?? 0,
                    outFrame: clip["outFrame"]?.uint64Value ?? 0,
                    speed: clip["speed"]?.numberValue ?? 1,
                    gainDb: clip["gainDb"]?.numberValue ?? 0,
                    transition: ReplayTransitionInfo.read(clip["transition"]),
                    tags: (clip["tags"]?.arrayValue ?? []).compactMap { $0.stringValue },
                    preservePitch: clip["preservePitch"]?.boolValue ?? false,
                    audioKind: clip["audio"]?.stringValue ?? "FollowAngle",
                    audioAngleId: clip["audioAngleId"]?.uint32Value ?? 0
                )
            }
        }
        return []
    }

    func angleLabel(_ angleId: UInt32) -> String {
        for angle in angles where angle.id == angleId {
            return angle.label
        }
        return angleId == 0 ? "-" : "\(angleId)"
    }

    mutating func setActiveAngle(_ id: UInt32) {
        raw["activeAngleId"] = .number(Double(id))
    }

    mutating func reserveEventId() -> UInt64 {
        var highest: UInt64 = 0
        for item in array("events") {
            highest = max(highest, item.objectValue?["id"]?.uint64Value ?? 0)
        }
        let next = max(raw["nextEventId"]?.uint64Value ?? 1, highest + 1)
        raw["nextEventId"] = .number(Double(next + 1))
        return next
    }

    mutating func reservePlaylistId() -> UInt64 {
        var highest: UInt64 = 0
        for item in array("playlists") {
            highest = max(highest, item.objectValue?["id"]?.uint64Value ?? 0)
        }
        let next = max(raw["nextPlaylistId"]?.uint64Value ?? 1, highest + 1)
        raw["nextPlaylistId"] = .number(Double(next + 1))
        return next
    }

    /// Creates the first playlist when the project has none yet.
    mutating func ensurePlaylist() -> UInt64 {
        if let existing = array("playlists").first?.objectValue?["id"]?.uint64Value {
            return existing
        }
        let id = reservePlaylistId()
        var playlists = array("playlists")
        playlists.append(.object([
            "id": .number(Double(id)),
            "name": .string("Replay"),
            "revision": .number(0),
            "clips": .array([]),
            "defaultTransition": ReplayTransitionInfo.cut.value,
        ]))
        raw["playlists"] = .array(playlists)
        return id
    }

    mutating func appendEvent(uuid: String, inFrame: UInt64, outFrame: UInt64, name: String) -> UInt64 {
        let id = reserveEventId()
        var events = array("events")
        events.append(.object([
            "id": .number(Double(id)),
            "recordingUuid": .string(uuid),
            "inFrame": .number(Double(inFrame)),
            "outFrame": .number(Double(outFrame)),
            "name": .string(name),
            "tags": .array([]),
            "protected": .bool(false),
        ]))
        raw["events"] = .array(events)
        return id
    }

    mutating func setEventProtected(_ id: UInt64, _ value: Bool) {
        var events = array("events")
        for index in events.indices {
            guard var object = events[index].objectValue,
                  (object["id"]?.uint64Value ?? 0) == id
            else { continue }
            object["protected"] = .bool(value)
            events[index] = .object(object)
        }
        raw["events"] = .array(events)
    }

    mutating func removeEvent(_ id: UInt64) {
        raw["events"] = .array(array("events").filter { item in
            (item.objectValue?["id"]?.uint64Value ?? 0) != id
        })
    }

    /// Drops the link from every clip to an event that no longer exists. The clip
    /// keeps its own recording, range and angle, so it still plays.
    mutating func clearSourceEvent(_ eventId: UInt64) {
        guard var playlists = raw["playlists"]?.arrayValue else { return }
        for index in playlists.indices {
            guard var playlist = playlists[index].objectValue,
                  var clips = playlist["clips"]?.arrayValue
            else { continue }
            var changed = false
            for clipIndex in clips.indices {
                guard var clip = clips[clipIndex].objectValue,
                      (clip["sourceEvent"]?.uint64Value ?? 0) == eventId
                else { continue }
                clip.removeValue(forKey: "sourceEvent")
                clips[clipIndex] = .object(clip)
                changed = true
            }
            guard changed else { continue }
            playlist["clips"] = .array(clips)
            playlist["revision"] = .number(Double((playlist["revision"]?.uint64Value ?? 0) + 1))
            playlists[index] = .object(playlist)
        }
        raw["playlists"] = .array(playlists)
    }

    /// A clip uid that no playlist uses yet. The uid is the handle a channel plays
    /// by, so the control layer refuses a document where two clips share one.
    /// Playing the same event twice is a normal replay move, so the second clip
    /// takes a suffix instead of the operator being told no.
    func uniqueClipUid(_ eventId: UInt64) -> String {
        var used = Set<String>()
        for playlist in raw["playlists"]?.arrayValue ?? [] {
            for clip in playlist.objectValue?["clips"]?.arrayValue ?? [] {
                if let uid = clip.objectValue?["uid"]?.stringValue {
                    used.insert(uid)
                }
            }
        }
        var candidate = "clip-\(eventId)"
        var suffix = 2
        while used.contains(candidate) {
            candidate = "clip-\(eventId)-\(suffix)"
            suffix += 1
        }
        return candidate
    }

    /// Adds a clip for an event at the end of the playlist. Returns false when no
    /// angle is selected, because the control layer refuses to guess one.
    mutating func appendClip(playlistId: UInt64, event: ReplayEventInfo, angleId: UInt32) -> Bool {
        guard angleId != 0 else { return false }
        guard var playlists = raw["playlists"]?.arrayValue else { return false }
        for index in playlists.indices {
            guard var playlist = playlists[index].objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            var clips = playlist["clips"]?.arrayValue ?? []
            // A new clip starts on the playlist's default transition, so an
            // operator who sets "always dissolve" gets it without editing every clip.
            let fallback = playlist["defaultTransition"] ?? ReplayTransitionInfo.cut.value
            clips.append(.object([
                "uid": .string(uniqueClipUid(event.id)),
                "source": .string("Angle"),
                "angleId": .number(Double(angleId)),
                "sourceEvent": .number(Double(event.id)),
                "recordingUuid": .string(event.recordingUuid),
                "inFrame": .number(Double(event.inFrame)),
                "outFrame": .number(Double(event.outFrame)),
                "speed": .number(1),
                "gainDb": .number(0),
                "transition": fallback,
            ]))
            playlist["clips"] = .array(clips)
            playlist["revision"] = .number(Double((playlist["revision"]?.uint64Value ?? 0) + 1))
            playlists[index] = .object(playlist)
            raw["playlists"] = .array(playlists)
            return true
        }
        return false
    }

    mutating func removeClip(playlistId: UInt64, uid: String) {
        guard var playlists = raw["playlists"]?.arrayValue else { return }
        for index in playlists.indices {
            guard var playlist = playlists[index].objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            playlist["clips"] = .array((playlist["clips"]?.arrayValue ?? []).filter { item in
                (item.objectValue?["uid"]?.stringValue ?? "") != uid
            })
            playlist["revision"] = .number(Double((playlist["revision"]?.uint64Value ?? 0) + 1))
            playlists[index] = .object(playlist)
        }
        raw["playlists"] = .array(playlists)
    }

    /// Applies an edit to one clip. Playlists are rebuilt because the document is
    /// a value tree.
    private mutating func updateClip(
        playlistId: UInt64,
        uid: String,
        _ edit: (inout [String: JSONValue]) -> Void
    ) {
        guard var playlists = raw["playlists"]?.arrayValue else { return }
        for index in playlists.indices {
            guard var playlist = playlists[index].objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            var clips = playlist["clips"]?.arrayValue ?? []
            for clipIndex in clips.indices {
                guard var clip = clips[clipIndex].objectValue,
                      (clip["uid"]?.stringValue ?? "") == uid
                else { continue }
                edit(&clip)
                clips[clipIndex] = .object(clip)
            }
            playlist["clips"] = .array(clips)
            playlist["revision"] = .number(Double((playlist["revision"]?.uint64Value ?? 0) + 1))
            playlists[index] = .object(playlist)
        }
        raw["playlists"] = .array(playlists)
    }

    /// Replaces a clip's tags. The catalog that suggests them lives on the
    /// session, the same way input and scene tags do.
    mutating func setClipTags(playlistId: UInt64, uid: String, tags: [String]) {
        guard var playlists = raw["playlists"]?.arrayValue else { return }
        for index in playlists.indices {
            guard var playlist = playlists[index].objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            var clips = playlist["clips"]?.arrayValue ?? []
            for clipIndex in clips.indices {
                guard var clip = clips[clipIndex].objectValue,
                      (clip["uid"]?.stringValue ?? "") == uid
                else { continue }
                clip["tags"] = .array(tags.map { .string($0) })
                clips[clipIndex] = .object(clip)
            }
            playlist["clips"] = .array(clips)
            playlist["revision"] = .number(Double((playlist["revision"]?.uint64Value ?? 0) + 1))
            playlists[index] = .object(playlist)
        }
        raw["playlists"] = .array(playlists)
    }

    /// Writes the playlist's default transition. New clips start on it.
    mutating func setDefaultTransition(playlistId: UInt64, transition: ReplayTransitionInfo) {
        guard var playlists = raw["playlists"]?.arrayValue else { return }
        for index in playlists.indices {
            guard var playlist = playlists[index].objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            playlist["defaultTransition"] = transition.value
            playlists[index] = .object(playlist)
        }
        raw["playlists"] = .array(playlists)
    }

    /// Writes the clip gain in dB. The mixer clamps it, and the editor refuses
    /// nonsense before it gets that far.
    mutating func setClipGain(playlistId: UInt64, uid: String, db: Double) {
        updateClip(playlistId: playlistId, uid: uid) { clip in
            clip["gainDb"] = .number(db)
        }
    }

    /// Writes where a clip takes its audio from.
    mutating func setClipAudio(playlistId: UInt64, uid: String, kind: String, angleId: UInt32) {
        updateClip(playlistId: playlistId, uid: uid) { clip in
            clip["audio"] = .string(kind)
            clip["audioAngleId"] = .number(Double(angleId))
        }
    }

    /// Writes the pitch flag of one clip. Slow motion otherwise drops the pitch
    /// along with the speed.
    mutating func setClipPreservePitch(playlistId: UInt64, uid: String, value: Bool) {
        updateClip(playlistId: playlistId, uid: uid) { clip in
            clip["preservePitch"] = .bool(value)
        }
    }

    mutating func setClipTransition(playlistId: UInt64, uid: String, transition: ReplayTransitionInfo) {
        guard var playlists = raw["playlists"]?.arrayValue else { return }
        for index in playlists.indices {
            guard var playlist = playlists[index].objectValue,
                  (playlist["id"]?.uint64Value ?? 0) == playlistId
            else { continue }
            var clips = playlist["clips"]?.arrayValue ?? []
            for clipIndex in clips.indices {
                guard var clip = clips[clipIndex].objectValue,
                      (clip["uid"]?.stringValue ?? "") == uid
                else { continue }
                clip["transition"] = transition.value
                clips[clipIndex] = .object(clip)
            }
            playlist["clips"] = .array(clips)
            playlist["revision"] = .number(Double((playlist["revision"]?.uint64Value ?? 0) + 1))
            playlists[index] = .object(playlist)
        }
        raw["playlists"] = .array(playlists)
    }

    /// One recording drive. An angle may name the drive it records to.
    struct StorageSpec: Identifiable, Hashable {
        var path: String
        var angleIds: [UInt32]

        var id: String { path }
    }

    var storages: [StorageSpec] {
        array("storage").compactMap { item -> StorageSpec? in
            guard let object = item.objectValue else { return nil }
            return StorageSpec(
                path: object["path"]?.stringValue ?? "",
                angleIds: (object["angleIds"]?.arrayValue ?? []).compactMap { $0.uint32Value }
            )
        }
    }

    mutating func setStorages(_ specs: [StorageSpec]) {
        let clean = specs.filter { !$0.path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        raw["enabled"] = .bool(!clean.isEmpty)
        raw["storage"] = .array(clean.map { spec in
            .object([
                "path": .string(spec.path.trimmingCharacters(in: .whitespacesAndNewlines)),
                "angleIds": .array(spec.angleIds.sorted().map { .number(Double($0)) }),
            ])
        })
    }

    mutating func setAngles(_ angles: [ReplayAngleInfo]) {
        raw["angles"] = .array(angles.map { angle in
            .object([
                "id": .number(Double(angle.id)),
                "name": .string(angle.name),
                "sceneId": .number(Double(angle.sceneId)),
                "audio": .string(angle.audioUnitId == 0 ? "Silent" : "Unit"),
                "audioUnitId": .number(Double(angle.audioUnitId)),
                "recordMode": .string("Composite"),
            ])
        })
        if !angles.contains(where: { $0.id == activeAngleId }) {
            raw.removeValue(forKey: "activeAngleId")
        }
    }

    mutating func setRetention(_ value: String) {
        raw["retention"] = .string(value)
    }

    mutating func setAutoAddToPlaylist(_ value: Bool) {
        raw["autoAddToPlaylist"] = .bool(value)
    }
}

enum ReplayTime {
    static func rate(_ fps: (num: UInt32, den: UInt32)) -> Double {
        fps.den == 0 ? 60.0 : Double(fps.num) / Double(fps.den)
    }

    /// HH:MM:SS:FF.
    static func timecode(_ frames: UInt64, _ fps: (num: UInt32, den: UInt32)) -> String {
        let perSecond = Int(rate(fps).rounded())
        guard perSecond > 0 else { return "\(frames)" }
        let seconds = frames / UInt64(perSecond)
        let frame = frames % UInt64(perSecond)
        return String(
            format: "%02llu:%02llu:%02llu:%02llu",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            frame
        )
    }

    static func duration(_ inFrame: UInt64, _ outFrame: UInt64, _ fps: (num: UInt32, den: UInt32)) -> String {
        timecode(outFrame > inFrame ? outFrame - inFrame : 0, fps)
    }

    /// `HH:MM:SS` for a duration in seconds.
    static func clock(_ seconds: UInt32) -> String {
        String(format: "%02u:%02u:%02u", seconds / 3600, seconds / 60 % 60, seconds % 60)
    }

    /// Falls back to the session master rate while the replay format has none.
    static func resolve(_ fps: (num: UInt32, den: UInt32), _ settings: SessionSettings) -> (num: UInt32, den: UInt32) {
        fps.num == 0 || fps.den == 0 ? (settings.masterFpsNum, settings.masterFpsDen) : fps
    }
}
