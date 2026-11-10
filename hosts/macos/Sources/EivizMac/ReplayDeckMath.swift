import Foundation

/// The arithmetic the replay deck does between a widget and the mixer. Kept out of the
/// view so a regression can be caught without opening a window.
enum ReplayDeckMath {
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
