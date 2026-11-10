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
