@testable import EivizMac
import Foundation

var failures = 0

@MainActor func check(_ name: String, _ condition: @autoclosure () -> Bool) {
    if condition() {
        print("ok   \(name)")
    } else {
        print("FAIL \(name)")
        failures += 1
    }
}

let fps: (num: UInt32, den: UInt32) = (60_000, 1_001)
check("timecode zero", ReplayTime.timecode(0, fps) == "00:00:00:00")
check("timecode one second", ReplayTime.timecode(60, fps) == "00:00:01:00")
check("duration", ReplayTime.duration(30, 90, fps) == "00:00:01:00")
check("clock", ReplayTime.clock(3661) == "01:01:01")

let livePlaying = ReplayTransportState(live: true, playing: true)
let pause = ReplayTransportRules.toggle(livePlaying)
check("toggle pauses", !pause.playing)
check("toggle keeps edge", pause.next.live)

let held = ReplayTransportState(live: false, playing: false)
let play = ReplayTransportRules.toggle(held)
check("toggle plays", play.playing)
check("toggle stays held", !play.next.live)

let seek = ReplayTransportRules.seek()
check("seek pauses", !seek.playing)
check("seek leaves edge", !seek.next.live)

let jump = ReplayTransportRules.jumpToLive()
check("jump is playing", jump.playing)
check("jump is live", jump.next.live)

check("adopt follows mixer stop", !ReplayTransportRules.adopt(livePlaying, mixerPlaying: false).playing)
check("adopt follows mixer play", ReplayTransportRules.adopt(held, mixerPlaying: true).playing)


// MARK: - Deck arithmetic

check("step moves from playhead", ReplayDeckMath.stepTarget(current: 100, delta: 1, markIn: nil, markOut: nil, range: 0...500) == 101)
check("step back clamps at zero", ReplayDeckMath.stepTarget(current: 0, delta: -1, markIn: nil, markOut: nil, range: 0...500) == 0)
check("step clamps to marks", ReplayDeckMath.stepTarget(current: 150, delta: 100, markIn: 50, markOut: 200, range: 0...500) == 200)
check("step from mark does not jump to it", ReplayDeckMath.stepTarget(current: 150, delta: 1, markIn: 50, markOut: 200, range: 0...500) == 151)
check("step clamps to range", ReplayDeckMath.stepTarget(current: 500, delta: 10, markIn: nil, markOut: nil, range: 0...500) == 500)

check("remaining clip uses channel out", ReplayDeckMath.remainingClip(position: 100, channelOut: 300, markOut: nil) == 200)
check("remaining clip falls back to mark", ReplayDeckMath.remainingClip(position: 100, channelOut: 0, markOut: 250) == 150)
check("remaining clip none", ReplayDeckMath.remainingClip(position: 300, channelOut: 300, markOut: nil) == nil)

let clips = [
    ReplayClipInfo(uid: "a", angleId: 1, recordingUuid: "r", inFrame: 0, outFrame: 100, speed: 1, gainDb: 0, transition: .cut, tags: [], preservePitch: false, audioKind: "FollowAngle", audioAngleId: 1),
    ReplayClipInfo(uid: "b", angleId: 1, recordingUuid: "r", inFrame: 100, outFrame: 200, speed: 1, gainDb: 0, transition: .cut, tags: [], preservePitch: false, audioKind: "FollowAngle", audioAngleId: 1),
]
check("playlist remaining first clip", ReplayDeckMath.remainingPlaylist(clips: clips, position: 50) == 150)
check("playlist remaining second clip", ReplayDeckMath.remainingPlaylist(clips: clips, position: 150) == 50)
check("playlist remaining past end", ReplayDeckMath.remainingPlaylist(clips: clips, position: 250) == 0)

check("clip hash is stable", ReplayDeckMath.clipUidHash("abc") == ReplayDeckMath.clipUidHash("abc"))
check("clip hash differs", ReplayDeckMath.clipUidHash("abc") != ReplayDeckMath.clipUidHash("abd"))

// MARK: - Document edits

let format = ReplayFormatValues(width: 1920, height: 1080, fpsNum: 60_000, fpsDen: 1_001)
var document = ReplayDocument.empty(format: format)
let eventId = document.appendEvent(uuid: "rec", inFrame: 100, outFrame: 200, name: "one")
document.setEventRange(eventId, inFrame: 40, outFrame: 180)
check("event range edit", document.events.first?.inFrame == 40)
check("event range edit out", document.events.first?.outFrame == 180)

let secondId = document.appendEvent(uuid: "rec", inFrame: 300, outFrame: 400, name: "two")
let playlist = document.ensurePlaylist()
if let firstEvent = document.events.first(where: { $0.id == eventId }) {
    _ = document.appendClip(playlistId: playlist, event: firstEvent, angleId: 1)
}
if let secondEvent = document.events.first(where: { $0.id == secondId }) {
    _ = document.appendClip(playlistId: playlist, event: secondEvent, angleId: 1)
}
check("two clips appended", document.clips(playlistId: playlist).count == 2)
document.sortClips(playlistId: playlist)
let sorted = document.clips(playlistId: playlist)
check("clips sort by timecode", sorted.first?.inFrame == 40 && sorted.last?.inFrame == 300)


// MARK: - Auto add honours clip order

var autoAddDoc = ReplayDocument.empty(format: format)
autoAddDoc.raw["autoAddToPlaylist"] = .bool(true)
let late = autoAddDoc.appendEvent(uuid: "rec", inFrame: 300, outFrame: 400, name: "late")
let early = autoAddDoc.appendEvent(uuid: "rec", inFrame: 10, outFrame: 50, name: "early")
_ = autoAddDoc.addEventClip(eventId: late, angleId: 1, sortByTimecode: true)
_ = autoAddDoc.addEventClip(eventId: early, angleId: 1, sortByTimecode: true)
let ordered = autoAddDoc.clips(playlistId: autoAddDoc.ensurePlaylist())
check("auto add sorts by timecode", ordered.first?.inFrame == 10 && ordered.last?.inFrame == 300)


// MARK: - Range validation and waveform mapping

check("event range must advance", ReplayDeckMath.eventRangeValid(inFrame: 10, outFrame: 20))
check("event range rejects equal", !ReplayDeckMath.eventRangeValid(inFrame: 10, outFrame: 10))
check("event range rejects backwards", !ReplayDeckMath.eventRangeValid(inFrame: 20, outFrame: 10))

check("waveform start", ReplayDeckMath.waveformFrame(ratio: 0, bucketRate: 60, buckets: 60, dropped: 0, fps: 60) == 0)
check("waveform half", ReplayDeckMath.waveformFrame(ratio: 0.5, bucketRate: 60, buckets: 60, dropped: 0, fps: 60) == 30)
check("waveform end", ReplayDeckMath.waveformFrame(ratio: 1, bucketRate: 60, buckets: 60, dropped: 0, fps: 60) == 60)
check("waveform dropped offset", ReplayDeckMath.waveformFrame(ratio: 0, bucketRate: 60, buckets: 60, dropped: 10, fps: 60) == 10)
check("waveform clamps left", ReplayDeckMath.waveformFrame(ratio: -1, bucketRate: 60, buckets: 60, dropped: 0, fps: 60) == 0)
check("waveform clamps right", ReplayDeckMath.waveformFrame(ratio: 2, bucketRate: 60, buckets: 60, dropped: 0, fps: 60) == 60)


// MARK: - Export planning and JSON

let exportClip = ReplayClipInfo(
    uid: "a", angleId: 2, recordingUuid: "r", inFrame: 10, outFrame: 70, speed: 2.0,
    gainDb: 0, transition: ReplayTransitionInfo(kind: "Dissolve", timing: "Overwrap", durationFrames: 15),
    tags: [], preservePitch: false, audioKind: "Silent", audioAngleId: 0)
let exportClips = ReplayExportPlan.clips(mode: ReplayExportPlan.Mode.clip.rawValue, clips: [exportClip], selectedClip: "a")
check("clip mode selects clip", exportClips.count == 1 && exportClips[0].transition == nil)
let playlistClips = ReplayExportPlan.clips(mode: ReplayExportPlan.Mode.playlistWithTransitions.rawValue, clips: [exportClip], selectedClip: nil)
check("transition mode keeps transition", playlistClips.first?.transition?.kind == "Dissolve")
let invalidClip = ReplayClipInfo(
    uid: "b", angleId: 2, recordingUuid: "r", inFrame: 70, outFrame: 10, speed: 1,
    gainDb: 0, transition: .cut, tags: [], preservePitch: false,
    audioKind: "FollowAngle", audioAngleId: 0)
check("invalid clip omitted from playlist export",
      ReplayExportPlan.clips(mode: ReplayExportPlan.Mode.playlistWithTransitions.rawValue, clips: [invalidClip], selectedClip: nil).isEmpty)
check("invalid clip omitted from segment export",
      ReplayExportPlan.segments(mode: ReplayExportPlan.Mode.playlist.rawValue, clips: [invalidClip], angles: [], active: 0, range: { _ in nil }).isEmpty)

let angles = [
    ReplayAngleInfo(id: 1, name: "A", source: "Scene", sceneId: 1, inputId: 0, audioUnitId: 0, encoder: "Cpu"),
    ReplayAngleInfo(id: 2, name: "B", source: "Scene", sceneId: 2, inputId: 0, audioUnitId: 0, encoder: "Cpu"),
]
let ranges: [UInt32: (first: UInt64, last: UInt64)] = [1: (0, 100), 2: (0, 200)]
let angleSegments = ReplayExportPlan.segments(
    mode: ReplayExportPlan.Mode.allAngles.rawValue, clips: [], angles: angles, active: 2,
    range: { ranges[$0] })
check("all angles starts with active", angleSegments.first?.0 == 2)
check("all angles has both", angleSegments.count == 2)

let json = ReplayExportJson.clip(exportClip, exportClip.transition)
if let data = json.data(using: .utf8),
   let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
    check("export speed percent", object["speedPercent"] as? Int == 200)
    check("export silent audio", object["audioAngleId"] is NSNull)
    check("export transition kind", (object["transition"] as? [String: Any])?["kind"] as? Int == 1)
} else {
    check("export json parses", false)
}
check("kind codes", ReplayExportJson.kindCode("Dissolve") == 1 && ReplayExportJson.kindCode("Cut") == nil)
check("direction codes", ReplayExportJson.directionCode("Down") == 3 && ReplayExportJson.directionCode("Left") == 0)
check("gain zero db", abs(ReplayExportJson.gain(decibels: 0) - 1.0) < 0.001)
check("gain minus six db", abs(ReplayExportJson.gain(decibels: -6.0) - 0.501) < 0.001)
check("health watches first angle", ReplayHealth.watchedAngle(active: 0, angles: angles) == 1)
check("health watches active angle", ReplayHealth.watchedAngle(active: 2, angles: angles) == 2)


// MARK: - Session and document round trip

var settings = SessionSettings()
settings.replayPresentInterval = 3
if let encoded = try? JSONEncoder().encode(settings),
   let decoded = try? JSONDecoder().decode(SessionSettings.self, from: encoded) {
    check("session keeps replay present interval", decoded.replayPresentInterval == 3)
} else {
    check("session settings encode", false)
}

var roundTrip = ReplayDocument.empty(format: format)
let roundTripId = roundTrip.appendEvent(uuid: "rec", inFrame: 5, outFrame: 25, name: "event")
roundTrip.raw["decodeAhead"] = .bool(false)
roundTrip.raw["profiling"] = .bool(true)
if let value = try? JSONDecoder().decode(JSONValue.self, from: Data(roundTrip.json.utf8)),
   let restored = ReplayDocument(value) {
    check("document keeps event", restored.events.contains { $0.id == roundTripId })
    check("document keeps event range", restored.events.first?.outFrame == 25)
    check("document keeps decode ahead", restored.raw["decodeAhead"]?.boolValue == false)
    check("document keeps profiling", restored.raw["profiling"]?.boolValue == true)
} else {
    check("document round trip encodes", false)
}


// MARK: - Settings payload sent on Apply

if let value = try? JSONDecoder().decode(JSONValue.self, from: Data(roundTrip.settingsJSON.utf8)),
   let object = value.objectValue {
    check("settings payload keeps decode ahead", object["decodeAhead"]?.boolValue == false)
    check("settings payload keeps profiling", object["profiling"]?.boolValue == true)
    check("settings payload keeps angles", object["angles"] != nil)
    check("settings payload omits deck", object["events"] == nil && object["playlists"] == nil)
} else {
    check("settings payload parses", false)
}


// MARK: - Angle id allocation

var angleDoc = ReplayDocument.empty(format: format)
let firstAngle = angleDoc.reserveAngleId()
let secondAngle = angleDoc.reserveAngleId()
check("angle ids advance", firstAngle == 1 && secondAngle == 2)
check("angle next id stored", angleDoc.raw["nextAngleId"]?.uint32Value == 3)


// MARK: - Protected events and playlist removal

var protectDoc = ReplayDocument.empty(format: format)
let protectedId = protectDoc.appendEvent(uuid: "rec", inFrame: 0, outFrame: 10, name: "p")
protectDoc.setEventProtected(protectedId, true)
check("protected event refuses delete", !protectDoc.removeEventIfUnprotected(protectedId))
check("protected event remains", protectDoc.events.count == 1)
protectDoc.setEventProtected(protectedId, false)
check("unprotected event deletes", protectDoc.removeEventIfUnprotected(protectedId))
check("unprotected event gone", protectDoc.events.isEmpty)

var playlistDoc = ReplayDocument.empty(format: format)
let playlistA = playlistDoc.ensurePlaylist()
let playlistB = playlistDoc.addPlaylist(name: "B")
check("playlist ids advance", playlistA == 1 && playlistB == 2)
check("playlist next id stored", playlistDoc.raw["nextPlaylistId"]?.uint64Value == 3)


// MARK: - Auto add targets the selected playlist

var routingDoc = ReplayDocument.empty(format: format)
let firstPlaylist = routingDoc.ensurePlaylist()
let secondPlaylist = routingDoc.addPlaylist(name: "Second")
let routedEvent = routingDoc.appendEvent(uuid: "rec", inFrame: 0, outFrame: 30, name: "routed")
check("ensure preferred playlist", routingDoc.ensurePlaylist(preferred: secondPlaylist) == secondPlaylist)
check("ensure unknown falls back", routingDoc.ensurePlaylist(preferred: 999) == firstPlaylist)
_ = routingDoc.addEventClip(
    eventId: routedEvent, angleId: 1, playlistId: secondPlaylist, sortByTimecode: false)
check("clip lands in selected playlist", routingDoc.clips(playlistId: secondPlaylist).count == 1)
check("other playlist untouched", routingDoc.clips(playlistId: firstPlaylist).isEmpty)


// MARK: - Export queue

var queue = ReplayExportQueue()
let jobA = ReplayExportJob(json: "a", label: "a.mp4")
let jobB = ReplayExportJob(json: "b", label: "b.mp4")
queue.enqueue(jobA)
queue.enqueue(jobB)
check("queue is FIFO", queue.next() == jobA)
check("queue then second", queue.next() == jobB)
check("queue empty after drain", queue.isEmpty && queue.next() == nil)
queue.enqueue(jobA)
queue.removeAll()
check("queue removeAll", queue.isEmpty)


// MARK: - Export runner state machine

@MainActor
final class FakeExportMachine: ReplayExportMachine {
    var starts: [String] = []
    var startResult = true
    var progress: [ReplayExportProgress?] = []
    private(set) var cancelled = false

    func startReplayExport(_ json: String) -> Bool {
        starts.append(json)
        return startResult
    }

    func cancelReplayExport() {
        cancelled = true
    }

    func replayExportProgress() -> ReplayExportProgress? {
        guard !progress.isEmpty else { return nil }
        return progress.removeFirst()
    }
}

@MainActor
func exportProgress(_ state: Int, message: String = "") -> ReplayExportProgress {
    ReplayExportProgress(json: "{\"state\":\(state),\"done\":1,\"total\":2,\"skipped\":0,\"bytes\":10,\"message\":\"\(message)\"}")!
}

@MainActor
func runExportRunnerChecks() {
    let fake = FakeExportMachine()
    let runner = ReplayExportRunner()
    fake.startResult = false
    check("export refusal returns false", !runner.start(json: "a", label: "a", mixer: fake))
    check("export refusal reports", runner.status == L10n.t("replay.exportFailed"))
    fake.startResult = true
    check("export after refusal starts fresh", runner.start(json: "b", label: "b", mixer: fake))
    check("export after refusal used new job", fake.starts == ["a", "b"])
    check("export busy after start", runner.busy)

    fake.progress = [exportProgress(2)]
    runner.poll(mixer: fake)
    check("export done clears busy", !runner.busy)
    check("export done reports", runner.status == L10n.format("replay.exportDone", "1", "0"))

    let queued = FakeExportMachine()
    let runner2 = ReplayExportRunner()
    check("export first queues", runner2.start(json: "1", label: "1", mixer: queued))
    check("export second queues", runner2.start(json: "2", label: "2", mixer: queued))
    check("export only first started", queued.starts == ["1"])
    queued.progress = [exportProgress(2)]
    runner2.poll(mixer: queued)
    check("export advances to second", queued.starts == ["1", "2"])
    queued.progress = [exportProgress(2)]
    runner2.poll(mixer: queued)
    check("export finishes queue", !runner2.busy)

    let cancelled = FakeExportMachine()
    let runner3 = ReplayExportRunner()
    _ = runner3.start(json: "x", label: "x", mixer: cancelled)
    runner3.cancel(mixer: cancelled)
    check("export cancel reaches mixer", cancelled.cancelled)
    cancelled.progress = [exportProgress(4)]
    runner3.poll(mixer: cancelled)
    check("export cancelled clears busy", !runner3.busy)
    check("export cancelled reports", runner3.status == L10n.t("replay.exportCancelled"))

    let failed = FakeExportMachine()
    let runner4 = ReplayExportRunner()
    _ = runner4.start(json: "f", label: "f", mixer: failed)
    failed.progress = [exportProgress(3, message: "disk")]
    runner4.poll(mixer: failed)
    check("export failure clears busy", !runner4.busy)
    check("export failure reports", runner4.status.contains("disk"))
}
runExportRunnerChecks()


// MARK: - All-angle export paths

check("angle path keeps extension",
      ReplayExportPlan.anglePath("/tmp/out.mp4", angleId: 2) == "/tmp/out-angle2.mp4")
check("angle path without extension",
      ReplayExportPlan.anglePath("/tmp/out", angleId: 1) == "/tmp/out-angle1")
check("angle path with dotted directory",
      ReplayExportPlan.anglePath("/tmp/a.b/out.mp4", angleId: 3) == "/tmp/a.b/out-angle3.mp4")


// MARK: - Export output path

check("output path adds mp4", ReplayExportPlan.outputPath("/tmp/out") == "/tmp/out.mp4")
check("output path keeps mp4", ReplayExportPlan.outputPath("/tmp/out.mp4") == "/tmp/out.mp4")
check("output path trims", ReplayExportPlan.outputPath(" /tmp/out ") == "/tmp/out.mp4")
check("output path dotted directory", ReplayExportPlan.outputPath("/tmp/a.b/out") == "/tmp/a.b/out.mp4")


// MARK: - Recording presence and export progress parsing

let tempRoot = (NSTemporaryDirectory() as NSString)
    .appendingPathComponent("eiviz-replay-checks-\(UUID().uuidString)")
try? FileManager.default.createDirectory(atPath: tempRoot, withIntermediateDirectories: true)
let recordingUuid = "rec-check"
let recordingFile = (tempRoot as NSString).appendingPathComponent(recordingUuid)
FileManager.default.createFile(atPath: recordingFile, contents: Data("x".utf8))
var presenceDoc = ReplayDocument(raw: [
    "recordings": .array([
        .object([
            "uuid": .string(recordingUuid),
            "storage": .string(tempRoot),
            "firstFrame": .number(0),
            "lastFrame": .number(10),
        ])
    ]),
    "events": .array([
        .object([
            "id": .number(1),
            "recordingUuid": .string(recordingUuid),
            "inFrame": .number(0),
            "outFrame": .number(10),
            "name": .string("e"),
            "protected": .bool(false),
            "tags": .array([]),
        ])
    ]),
    "playlists": .array([
        .object([
            "id": .number(1),
            "name": .string("p"),
            "revision": .number(1),
            "clips": .array([
                .object([
                    "uid": .string("c"),
                    "recordingUuid": .string(recordingUuid),
                    "angleId": .number(1),
                    "inFrame": .number(0),
                    "outFrame": .number(10),
                ])
            ]),
        ])
    ]),
])
check("present recording is not missing", presenceDoc.missingRecordingCount(skip: nil) == 0)
check("active recording skipped", presenceDoc.missingRecordingCount(skip: recordingUuid) == 0)
try? FileManager.default.removeItem(atPath: recordingFile)
check("missing recording counted once", presenceDoc.missingRecordingCount(skip: nil) == 1)
check("missing recording skip current", presenceDoc.missingRecordingCount(skip: recordingUuid) == 0)
try? FileManager.default.removeItem(atPath: tempRoot)

check("cancelled progress parses", ReplayExportProgress(json: "{\"state\":4,\"done\":0,\"total\":1}")?.state == .cancelled)
check("invalid progress is nil", ReplayExportProgress(json: "not json") == nil)
check("unknown state falls back to failed", ReplayExportProgress(json: "{\"state\":99}")?.state == .failed)


// MARK: - Clip creation

var clipDoc = ReplayDocument.empty(format: format)
let clipEvent = clipDoc.appendEvent(uuid: "rec", inFrame: 0, outFrame: 60, name: "clip event")
let clipPlaylist = clipDoc.ensurePlaylist()
check("clip needs an angle",
      !clipDoc.appendClip(playlistId: clipPlaylist, event: clipDoc.events[0], angleId: 0))
check("clip appends", clipDoc.appendClip(playlistId: clipPlaylist, event: clipDoc.events[0], angleId: 1))
check("clip is playable", clipDoc.clips(playlistId: clipPlaylist).count == 1)
check("clip uid uses event id", clipDoc.clips(playlistId: clipPlaylist).first?.uid == "clip-\(clipEvent)")
check("clip starts at real time", clipDoc.clips(playlistId: clipPlaylist).first?.speed == 1.0)
let secondPlaylistForClip = clipDoc.addPlaylist(name: "Other")
_ = clipDoc.appendClip(playlistId: secondPlaylistForClip, event: clipDoc.events[0], angleId: 1)
check("clip uid stays unique across playlists",
      clipDoc.clips(playlistId: secondPlaylistForClip).first?.uid == "clip-\(clipEvent)-2")

var transitionDoc = ReplayDocument.empty(format: format)
let transitionPlaylist = transitionDoc.ensurePlaylist()
transitionDoc.setDefaultTransition(
    playlistId: transitionPlaylist,
    transition: ReplayTransitionInfo(kind: "Dissolve", timing: "Overwrap", durationFrames: 30))
let transitionEvent = transitionDoc.appendEvent(uuid: "rec", inFrame: 0, outFrame: 30, name: "t")
_ = transitionDoc.appendClip(
    playlistId: transitionPlaylist,
    event: transitionDoc.events.first(where: { $0.id == transitionEvent })!,
    angleId: 1)
check("clip starts on playlist default transition",
      transitionDoc.clips(playlistId: transitionPlaylist).first?.transition.kind == "Dissolve")


// MARK: - Enabled flag follows the setup

var setupDoc = ReplayDocument.empty(format: format)
setupDoc.setStorages([ReplayDocument.StorageSpec(path: "/tmp/replay", angleIds: [1])])
setupDoc.refreshEnabled()
check("drive without angle is disabled", setupDoc.raw["enabled"]?.boolValue == false)
let setupAngle = ReplayAngleInfo(id: 1, name: "A", source: "Scene", sceneId: 1, inputId: 0, audioUnitId: 0, encoder: "Cpu")
setupDoc.setAngles([setupAngle])
setupDoc.refreshEnabled()
check("first angle and drive enables", setupDoc.raw["enabled"]?.boolValue == true)
setupDoc.setAngles([])
setupDoc.refreshEnabled()
check("no angle disables", setupDoc.raw["enabled"]?.boolValue == false)


// MARK: - Setup validation

let validAngle = ReplayAngleInfo(id: 1, name: "A", source: "Scene", sceneId: 1, inputId: 0, audioUnitId: 0, encoder: "Cpu")
let validStorage = ReplayDocument.StorageSpec(path: "/tmp/replay", angleIds: [1])
check("valid setup accepted", ReplayDeckMath.setupProblem(angles: [validAngle], storages: [validStorage]) == nil)
check("setup needs a drive", ReplayDeckMath.setupProblem(angles: [validAngle], storages: []) == .noStorage)
check("setup needs an angle", ReplayDeckMath.setupProblem(angles: [], storages: [validStorage]) == .noAngles)
check("angle needs a drive",
      ReplayDeckMath.setupProblem(angles: [validAngle], storages: [ReplayDocument.StorageSpec(path: "/tmp/replay", angleIds: [])]) == .angleNeedsDrive(1))
let sourceless = ReplayAngleInfo(id: 2, name: "B", source: "Scene", sceneId: 0, inputId: 0, audioUnitId: 0, encoder: "Cpu")
check("angle needs a source",
      ReplayDeckMath.setupProblem(angles: [sourceless], storages: [ReplayDocument.StorageSpec(path: "/tmp/replay", angleIds: [2])]) == .angleNeedsSource(2))
let duplicateA = ReplayAngleInfo(id: 3, name: "C", source: "Scene", sceneId: 5, inputId: 0, audioUnitId: 0, encoder: "Cpu")
let duplicateB = ReplayAngleInfo(id: 4, name: "D", source: "Scene", sceneId: 5, inputId: 0, audioUnitId: 0, encoder: "Cpu")
check("duplicate scene refused",
      ReplayDeckMath.setupProblem(angles: [duplicateA, duplicateB], storages: [ReplayDocument.StorageSpec(path: "/tmp/replay", angleIds: [3, 4])]) == .angleDuplicate(4, kind: "Scene"))


// MARK: - GPU angle capacity

let fourGpuAngles = (1...4).map {
    ReplayAngleInfo(id: UInt32($0), name: "G\($0)", source: "Scene", sceneId: UInt64($0), inputId: 0, audioUnitId: 0, encoder: "Gpu")
}
check("four gpu angles fit", ReplayDeckMath.gpuAnglesOverCapacity(fourGpuAngles) == nil)
let fiveGpuAngles = (1...5).map {
    ReplayAngleInfo(id: UInt32($0), name: "G\($0)", source: "Scene", sceneId: UInt64($0), inputId: 0, audioUnitId: 0, encoder: "Gpu")
}
check("five gpu angles warn", ReplayDeckMath.gpuAnglesOverCapacity(fiveGpuAngles) == 5)
let mixedAngles = [
    ReplayAngleInfo(id: 1, name: "a", source: "Scene", sceneId: 1, inputId: 0, audioUnitId: 0, encoder: "Cpu"),
    ReplayAngleInfo(id: 2, name: "b", source: "Scene", sceneId: 2, inputId: 0, audioUnitId: 0, encoder: "Gpu"),
]
check("cpu angles do not count", ReplayDeckMath.gpuAnglesOverCapacity(mixedAngles) == nil)


// MARK: - Full session round trip

var fullSession = MixerSessionData.default()
fullSession.nextMonitorId = 4321
fullSession.replay = .object([
    "events": .array([.object(["id": .number(1), "recordingUuid": .string("r"), "inFrame": .number(0), "outFrame": .number(10), "name": .string("e"), "protected": .bool(true), "tags": .array([])])]),
    "playlists": .array([]),
    "recordings": .array([]),
])
if var scene = fullSession.scenes.first {
    scene.sequences = [SceneSequence(id: 1, name: "s", steps: [
        SequenceStep(stateId: 1, motion: nil, holdFrames: 0, replay: SequenceReplayAction(channel: 0, clipUid: "clip-1", play: true))
    ])]
    fullSession.scenes[0] = scene
}
if let encoded = try? JSONEncoder().encode(fullSession),
   let decoded = try? JSONDecoder().decode(MixerSessionData.self, from: encoded) {
    // The core session format has no nextMonitorId; the host allocates monitors on load.
    // A missing key must not fail the whole session decode, or inputs stop syncing.
    check("session defaults missing monitor counter", decoded.nextMonitorId == 1000)
    check("session keeps replay document", decoded.replay?.objectValue?["events"] != nil)
    check("session keeps replay cue", decoded.scenes.first?.sequences.first?.steps.first?.replay?.clipUid == "clip-1")
    check("session keeps cue play", decoded.scenes.first?.sequences.first?.steps.first?.replay?.play == true)
    if let encodedText = String(data: encoded, encoding: .utf8) {
        check("session omits monitor counter", !encodedText.contains("nextMonitorId"))
    } else {
        check("session encodes text", false)
    }
    check("session keeps inputs", decoded.inputs.count == fullSession.inputs.count)
} else {
    check("session round trip encodes", false)
}


// MARK: - Transition persistence

var transitionRoundTrip = ReplayDocument.empty(format: format)
let roundTripPlaylist = transitionRoundTrip.ensurePlaylist()
transitionRoundTrip.setDefaultTransition(
    playlistId: roundTripPlaylist,
    transition: ReplayTransitionInfo(kind: "Wipe", direction: "Right", timing: "Additive", durationFrames: 24))
let roundTripEvent = transitionRoundTrip.appendEvent(uuid: "rec", inFrame: 0, outFrame: 24, name: "tr")
_ = transitionRoundTrip.appendClip(
    playlistId: roundTripPlaylist,
    event: transitionRoundTrip.events.first(where: { $0.id == roundTripEvent })!,
    angleId: 1)
let inherited = transitionRoundTrip.clips(playlistId: roundTripPlaylist).first?.transition
check("clip inherits default kind", inherited?.kind == "Wipe")
check("clip inherits default direction", inherited?.direction == "Right")
check("clip inherits default timing", inherited?.timing == "Additive")
check("clip inherits default frames", inherited?.durationFrames == 24)

let clipUid = transitionRoundTrip.clips(playlistId: roundTripPlaylist).first!.uid
transitionRoundTrip.setClipTransition(
    playlistId: roundTripPlaylist,
    uid: clipUid,
    transition: ReplayTransitionInfo(kind: "Wipe", direction: "Down", timing: "Overwrap", durationFrames: 12))
let edited = transitionRoundTrip.clips(playlistId: roundTripPlaylist).first?.transition
check("clip transition direction edit", edited?.direction == "Down")
check("clip transition timing edit", edited?.timing == "Overwrap")
check("clip transition frames edit", edited?.durationFrames == 12)

var cutDoc = ReplayDocument.empty(format: format)
let cutPlaylist = cutDoc.ensurePlaylist()
let cutEvent = cutDoc.appendEvent(uuid: "rec", inFrame: 0, outFrame: 10, name: "c")
_ = cutDoc.appendClip(playlistId: cutPlaylist, event: cutDoc.events[0], angleId: 1)
let cutUid = cutDoc.clips(playlistId: cutPlaylist).first!.uid
cutDoc.setClipTransition(
    playlistId: cutPlaylist, uid: cutUid,
    transition: ReplayTransitionInfo(kind: "Cut", timing: "Overwrap", durationFrames: 30))
check("cut transition has no frames", cutDoc.clips(playlistId: cutPlaylist).first?.transition.isCut == true)


// MARK: - Channel clamp after external edits

let channelsA = [
    ReplayChannelInfo(index: 0, inputId: 1),
    ReplayChannelInfo(index: 1, inputId: 2),
]
check("existing channel kept", ReplayDeckMath.channelToShow(channels: channelsA, current: 1) == 1)
check("missing channel falls back", ReplayDeckMath.channelToShow(channels: channelsA, current: 7) == 0)
check("empty channels keep current", ReplayDeckMath.channelToShow(channels: [], current: 7) == 7)


// MARK: - Duplicate storage paths

let duplicateStorageA = ReplayDocument.StorageSpec(path: "/tmp/replay", angleIds: [1])
let duplicateStorageB = ReplayDocument.StorageSpec(path: " /tmp/REPLAY ", angleIds: [2])
let duplicateAngles = [
    ReplayAngleInfo(id: 1, name: "A", source: "Scene", sceneId: 1, inputId: 0, audioUnitId: 0, encoder: "Cpu"),
    ReplayAngleInfo(id: 2, name: "B", source: "Scene", sceneId: 2, inputId: 0, audioUnitId: 0, encoder: "Cpu"),
]
check("duplicate storage refused",
      ReplayDeckMath.setupProblem(angles: duplicateAngles, storages: [duplicateStorageA, duplicateStorageB]) == .duplicateStorage(" /tmp/REPLAY "))


// MARK: - Live hold target

check("live pause uses head when playhead is stale",
      ReplayDeckMath.liveHoldTarget(playhead: 0, recordingHead: 1500) == 1500)
check("live pause keeps playhead when ahead",
      ReplayDeckMath.liveHoldTarget(playhead: 1600, recordingHead: 1500) == 1600)
check("live target both zero", ReplayDeckMath.liveHoldTarget(playhead: 0, recordingHead: 0) == 0)


// MARK: - Seek clamping

check("seek target inside range", ReplayDeckMath.clampedFrame(50, range: 0...100) == 50)
check("seek target below range", ReplayDeckMath.clampedFrame(0, range: 10...100) == 10)
check("seek target above range", ReplayDeckMath.clampedFrame(500, range: 10...100) == 100)


// MARK: - Localisation table integrity

func l10nEntries(_ language: String, source: String) -> [(key: String, value: String)]? {
    let marker = "private static let \(language): [String: String] = ["
    guard let start = source.range(of: marker)?.upperBound,
          let end = source.range(of: "\n    ]", range: start..<source.endIndex)
    else { return nil }
    let body = String(source[start..<end.lowerBound])
    let regex = try! NSRegularExpression(pattern: "\"([^\"]+)\": \"((?:[^\"\\\\]|\\\\.)*)\"")
    let range = NSRange(body.startIndex..<body.endIndex, in: body)
    return regex.matches(in: body, range: range).compactMap { match in
        guard let keyRange = Range(match.range(at: 1), in: body),
              let valueRange = Range(match.range(at: 2), in: body)
        else { return nil }
        return (String(body[keyRange]), String(body[valueRange]))
    }
}

let l10nPath = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .appendingPathComponent("Sources/EivizMac/L10n.swift")
if let source = try? String(contentsOf: l10nPath, encoding: .utf8),
   let english = l10nEntries("en", source: source),
   let japanese = l10nEntries("ja", source: source) {
    let englishKeys = english.map(\.key)
    let japaneseKeys = japanese.map(\.key)
    check("l10n en has no duplicate keys", Set(englishKeys).count == englishKeys.count)
    check("l10n ja has no duplicate keys", Set(japaneseKeys).count == japaneseKeys.count)
    check("l10n key sets match", Set(englishKeys) == Set(japaneseKeys))
    let englishValues = Dictionary(uniqueKeysWithValues: english)
    let japaneseValues = Dictionary(uniqueKeysWithValues: japanese)
    func placeholders(_ value: String) -> Set<String> {
        let regex = try! NSRegularExpression(pattern: #"\{(\d+)\}"#)
        let range = NSRange(value.startIndex..<value.endIndex, in: value)
        return Set(regex.matches(in: value, range: range).compactMap { match in
            Range(match.range(at: 1), in: value).map { String(value[$0]) }
        })
    }
    let mismatched = englishKeys.filter {
        placeholders(englishValues[$0] ?? "") != placeholders(japaneseValues[$0] ?? "")
    }
    check("l10n placeholders match", mismatched.isEmpty)
} else {
    check("l10n source readable", false)
}


// MARK: - Real core session fixture

let fixturePath = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("session-fixture.json")
if let data = try? Data(contentsOf: fixturePath) {
    do {
        let session = try JSONDecoder().decode(MixerSessionData.self, from: data)
        check("core session fixture decodes", true)
        check("core session keeps replay input", session.inputs.contains { $0.id == 524289 })
        check("core session keeps replay document", session.replay != nil)
        check("core session keeps next input id", session.nextInputId == 10)
    } catch {
        print("fixture decode error: \(error)")
        check("core session fixture decodes", false)
    }
} else {
    check("core session fixture readable", false)
}


// MARK: - Localisation call sites

if let source = try? String(contentsOf: l10nPath, encoding: .utf8),
   let english = l10nEntries("en", source: source) {
    let englishKeys = Set(english.map(\.key))
    let sourcesDirectory = l10nPath
        .deletingLastPathComponent()
        .deletingLastPathComponent()
    var missing: [String] = []
    if let files = try? FileManager.default.contentsOfDirectory(
        at: sourcesDirectory.appendingPathComponent("Sources/EivizMac"),
        includingPropertiesForKeys: nil
    ) {
        let regex = try! NSRegularExpression(pattern: #"L10n\.(?:t|format)\("([^"]+)""#)
        for file in files where file.pathExtension == "swift" {
            guard let text = try? String(contentsOf: file, encoding: .utf8) else { continue }
            let range = NSRange(text.startIndex..<text.endIndex, in: text)
            for match in regex.matches(in: text, range: range) {
                guard let keyRange = Range(match.range(at: 1), in: text) else { continue }
                let key = String(text[keyRange])
                // Interpolated call sites build the key at runtime; they are not literals.
                if key.contains("\\(") { continue }
                if !englishKeys.contains(key) { missing.append(key) }
            }
        }
    }
    check("l10n call sites use defined keys", missing.isEmpty)
    if !missing.isEmpty { print("undefined l10n keys: \(missing)") }
} else {
    check("l10n call site source readable", false)
}


// MARK: - Angle source normalization

var sourceDoc = ReplayDocument.empty(format: format)
sourceDoc.setAngles([
    ReplayAngleInfo(id: 1, name: "input", source: "Input", sceneId: 7, inputId: 524289, audioUnitId: 0, encoder: "Cpu"),
    ReplayAngleInfo(id: 2, name: "scene", source: "Scene", sceneId: 3, inputId: 99, audioUnitId: 0, encoder: "Cpu"),
])
let normalized = sourceDoc.angles
check("input angle clears scene id", normalized.first?.sceneId == 0 && normalized.first?.inputId == 524289)
check("scene angle clears input id", normalized.last?.sceneId == 3 && normalized.last?.inputId == 0)


// MARK: - Normalising saved angle sources

var staleDoc = ReplayDocument(raw: [
    "angles": .array([
        .object(["id": .number(1), "name": .string("a"), "source": .string("Input"), "sceneId": .number(3), "inputId": .number(524289), "audio": .string("Silent"), "audioUnitId": .number(0), "encoder": .string("Cpu")]),
        .object(["id": .number(2), "name": .string("b"), "source": .string("Scene"), "sceneId": .number(4), "inputId": .number(9), "audio": .string("Silent"), "audioUnitId": .number(0), "encoder": .string("Cpu")]),
    ])
])
check("stale sources need repair", staleDoc.normalizeAngleSources())
let repaired = staleDoc.angles
check("repaired input scene id", repaired.first?.sceneId == 0)
check("repaired scene input id", repaired.last?.inputId == 0)
check("repair is idempotent", !staleDoc.normalizeAngleSources())

if failures > 0 {
    print("\n\(failures) check(s) failed")
    exit(1)
}
print("\nall replay core checks passed")
