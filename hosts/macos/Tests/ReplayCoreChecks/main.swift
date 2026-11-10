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

if failures > 0 {
    print("\n\(failures) check(s) failed")
    exit(1)
}
print("\nall replay core checks passed")
