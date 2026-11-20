import Foundation

/// The arithmetic the replay deck does between a widget and the mixer. Kept out of the
/// view so a regression can be caught without opening a window.
enum ReplayDeckMath {
    /// Whether a deck tick should replace the settings table's angle rows.
    ///
    /// A tick that only filed a clip must not: the table's ids differ from the
    /// document's because the operator deleted a row, and comparing the two directly
    /// put that row back on every such tick. The tick is adopted only when the
    /// document's angles changed since this window loaded them and the operator has
    /// no unsaved row edit of their own.
    static func shouldAdoptAngles(
        remote: Set<UInt32>,
        local: Set<UInt32>,
        loaded: Set<UInt32>
    ) -> Bool {
        remote != loaded && local == loaded
    }

    /// Where a step or jog lands, clamped to the marks when both exist, otherwise to the
    /// recording range. Steps are relative to the playhead, not to Mark In.
    static func stepTarget(
        current: UInt64,
        delta: Int64,
        markIn: UInt64?,
        markOut: UInt64?,
        range: ClosedRange<UInt64>
    ) -> UInt64 {
        let base = Int64(current)
        var target = max(0, base + delta)
        if let start = markIn, let end = markOut, end > start {
            target = min(max(target, Int64(start)), Int64(end))
        } else {
            target = min(max(target, Int64(range.lowerBound)), Int64(range.upperBound))
        }
        return UInt64(target)
    }

    /// The mark-last span the operator typed, in frames, or nil when the value is unusable.
    ///
    /// A nonsense span must be an error the operator reads, not a silent correction: a
    /// "10" they can see is fine, but an unbounded or NaN multiplier has to be refused
    /// before it reaches the frame arithmetic.
    static func markLastSpan(
        secondsText: String,
        fps: (num: UInt32, den: UInt32)
    ) -> UInt64? {
        let trimmed = secondsText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let seconds = Double(trimmed), seconds.isFinite, seconds > 0,
              seconds <= markLastSecondsLimit
        else { return nil }
        let rate = ReplayTime.rate(fps)
        guard rate.isFinite, rate > 0 else { return nil }
        // The span in frames: bounded by the seconds limit it came from, so the
        // multiply cannot reach the integer ceiling.
        let frames = (seconds * rate).rounded()
        guard frames.isFinite, frames >= 1, frames <= maxSpanFrames else { return nil }
        return UInt64(frames)
    }

    /// The operator's second cap for "mark last": a day is more than any cut asks for.
    static let markLastSecondsLimit: Double = 86_400
    /// Frames a single command may address, well under `UInt64.max`.
    static let maxSpanFrames: Double = 1e15

    /// Why a typed timecode was refused.
    enum TimecodeParseError: Error, Equatable {
        /// Not the `HH:MM:SS:FF` shape.
        case malformed
        /// An element is outside its digit count (minutes past 59, frames past the rate).
        case fieldRange
        /// The text resolves to more frames than the deck can name.
        case tooLarge
    }

    /// Elements a timecode is allowed to carry: a recording longer than 999 hours is
    /// not a thing this engine writes, and capping the elements keeps the multiply
    /// inside safe whole numbers.
    static let timecodeHoursLimit: UInt64 = 999

    /// Reads `HH:MM:SS:FF` back into frames the way `ReplayTime.timecode` writes it.
    ///
    /// The parse is the inverse of the display, including the rate the display uses:
    /// the seconds part is converted with the real rate (59.94, not the rounded 60),
    /// so a round trip through text lands the playhead on the frame it showed. Every
    /// element and the total are checked, because a `FF` digit of a hundred million
    /// or a past-midnight multiply have both been crashes here.
    static func parseTimecode(
        _ text: String,
        fps: (num: UInt32, den: UInt32)
    ) -> Result<UInt64, TimecodeParseError> {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let parts = trimmed.split(separator: ":")
        guard parts.count == 4 else { return .failure(.malformed) }
        let rate = ReplayTime.rate(fps)
        guard rate.isFinite, rate > 0, rate <= ReplayTime.rateDisplayLimit else {
            return .failure(.tooLarge)
        }
        let perSecond = max(1, ReplayTime.frameDigitsPerSecond(rate))
        var elements: [UInt64] = []
        for (index, part) in parts.enumerated() {
            guard let value = UInt64(part) else { return .failure(.malformed) }
            let limit: UInt64 = index == 0
                ? timecodeHoursLimit
                : index == 3
                    ? perSecond - 1
                    : 59
            guard value >= 0, value <= limit else { return .failure(.fieldRange) }
            elements.append(value)
        }
        let seconds = elements[0] * 3600 + elements[1] * 60 + elements[2]
        guard seconds * UInt64(rate.rounded(.awayFromZero)) <= UInt64(ReplayTime.maxDisplayFrames) else {
            return .failure(.tooLarge)
        }
        guard let boundary = ReplayTime.secondsBoundary(seconds, in: rate) else {
            return .failure(.tooLarge)
        }
        let frames = boundary + elements[3]
        guard frames <= UInt64(ReplayTime.maxDisplayFrames) else { return .failure(.tooLarge) }
        return .success(frames)
    }

    /// The words a refused timecode is reported with.
    static func timecodeIssueKey(_ issue: TimecodeParseError) -> String {
        switch issue {
        case .malformed: return "replay.tcMalformed"
        case .fieldRange: return "replay.tcFieldRange"
        case .tooLarge: return "replay.tcTooLarge"
        }
    }

    /// Frames left in the clip on air. The channel's own out point is preferred; a mark
    /// out is only a fallback for a channel with no clip.
    static func remainingClip(position: UInt64, channelOut: UInt64, markOut: UInt64?) -> UInt64? {
        if channelOut > position { return channelOut - position }
        if let markOut, markOut > position { return markOut - position }
        return nil
    }

    /// Frames left in the playlist from `position`: the rest of the current clip plus
    /// every clip after it. A frame before the first clip counts the whole list; a frame
    /// past the end counts nothing.
    static func remainingPlaylist(clips: [ReplayClipInfo], position: UInt64) -> UInt64 {
        var left: UInt64 = 0
        for clip in clips {
            guard clip.outFrame > position else { continue }
            if position < clip.inFrame {
                left += clip.outFrame - clip.inFrame
            } else {
                left += clip.outFrame - position
            }
        }
        return left
    }

    /// An event range is only usable when it advances. The engine refuses a backwards
    /// range, so the table checks before it publishes and says why.
    static func eventRangeValid(inFrame: UInt64, outFrame: UInt64) -> Bool {
        outFrame > inFrame
    }

    /// The recording frame under a point on the waveform. The waveform draws only the
    /// buckets the store still holds, so the dropped count is the offset.
    static func waveformFrame(
        ratio: Double,
        bucketRate: Double,
        buckets: Int,
        dropped: UInt32,
        fps: Double
    ) -> UInt64 {
        guard buckets > 0, bucketRate > 0, fps > 0 else { return 0 }
        let clamped = min(max(ratio, 0), 1)
        let bucket = Double(dropped) + clamped * Double(buckets)
        return UInt64(max(0, (bucket * fps / bucketRate).rounded()))
    }

    /// A GPU encoder keeps up with about four angles at 1080p60. More than that records
    /// with most frames missing, which Windows warns about before applying.
    static let gpuAnglesPerEncoder = 4

    static func gpuAnglesOverCapacity(_ angles: [ReplayAngleInfo]) -> Int? {
        let count = angles.filter { $0.encoder == "Gpu" }.count
        return count > gpuAnglesPerEncoder ? count : nil
    }

    enum SetupProblem: Equatable {
        case noStorage
        case noAngles
        case angleNeedsDrive(UInt32)
        case angleNeedsSource(UInt32)
        case angleDuplicate(UInt32, kind: String)
        case duplicateStorage(String)
    }

    /// Checks the setup the Windows settings window refuses: a drive, at least one angle,
    /// every angle on a drive with a path, every angle with a source, and no two angles
    /// recording the same source.
    static func setupProblem(
        angles: [ReplayAngleInfo],
        storages: [ReplayDocument.StorageSpec]
    ) -> SetupProblem? {
        let usable = storages.filter {
            !$0.path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
        guard !usable.isEmpty else { return .noStorage }
        var paths = Set<String>()
        for storage in usable {
            let key = storage.path
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .lowercased()
            if !paths.insert(key).inserted {
                return .duplicateStorage(storage.path)
            }
        }
        guard !angles.isEmpty else { return .noAngles }
        for angle in angles {
            let drive = storages.firstIndex { $0.angleIds.contains(angle.id) }
            guard let drive, drive < storages.count,
                  !storages[drive].path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            else { return .angleNeedsDrive(angle.id) }
        }
        for angle in angles {
            let source = angle.source == "Input" ? angle.inputId : angle.sceneId
            if source == 0 { return .angleNeedsSource(angle.id) }
        }
        var seen = Set<String>()
        for angle in angles {
            let source = angle.source == "Input" ? angle.inputId : angle.sceneId
            let key = "\(angle.source):\(source)"
            if !seen.insert(key).inserted {
                return .angleDuplicate(angle.id, kind: angle.source)
            }
        }
        return nil
    }

    /// Keeps a waveform/seek target inside the recorded range. Clicking past either end
    /// must clamp, not ask the mixer for a frame the recording does not have.
    static func clampedFrame(_ frame: UInt64, range: ClosedRange<UInt64>) -> UInt64 {
        min(max(frame, range.lowerBound), range.upperBound)
    }

    /// Right end of the whole-recording seek bar. The bar always spans the recording:
    /// the last recorded frame, the frame the recorder is writing, and the playhead (a
    /// clip can sit past the last frame while the recording is still running) all keep
    /// the thumb inside the range.
    static func seekUpper(last: UInt64, playhead: UInt64, recordingHead: UInt64) -> UInt64 {
        max(1, last, playhead, recordingHead)
    }

    /// True once the engine reports the frame a seek asked for. Until then the drag owns
    /// the bar, so the thumb does not snap back to where the channel used to be.
    static func seekLanded(reported: UInt64, target: Double, tolerance: Double = 1) -> Bool {
        abs(Double(reported) - target) <= tolerance
    }

    /// The channel the deck should show after an external session edit. A selected channel
    /// that no longer exists used to leave the picker and the transport on a channel the
    /// document never had.
    static func channelToShow(channels: [ReplayChannelInfo], current: UInt32) -> UInt32 {
        if channels.contains(where: { $0.index == current }) { return current }
        return channels.first?.index ?? current
    }

    /// FNV-1a, the hash the mixer reports for the clip a channel is on.
    static func clipUidHash(_ uid: String) -> UInt64 {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in uid.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        return hash
    }
}
