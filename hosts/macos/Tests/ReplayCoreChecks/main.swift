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

if failures > 0 {
    print("\n\(failures) check(s) failed")
    exit(1)
}
print("\nall replay core checks passed")
