import EivizMixer
import Foundation

enum MixerFFI {
    static func withCString<R>(_ string: String, _ body: (UnsafePointer<CChar>) -> R) -> R {
        string.withCString(body)
    }

    static func listenOwnerText() -> String {
        var buffer = [UInt8](repeating: 0, count: 256)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_api_listen_owner(ptr.baseAddress, ptr.count)
        }
        guard n > 0 else { return "" }
        return String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
    }

    static func tcpListenOwnerText() -> String {
        var buffer = [UInt8](repeating: 0, count: 256)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_tcp_listen_owner(ptr.baseAddress, ptr.count)
        }
        guard n > 0 else { return "" }
        return String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
    }

    static func wsListenOwnerText() -> String {
        var buffer = [UInt8](repeating: 0, count: 256)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_ws_listen_owner(ptr.baseAddress, ptr.count)
        }
        guard n > 0 else { return "" }
        return String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
    }

    static func sourceStatus(_ id: UInt64) -> (connected: Bool, hasVideo: Bool) {
        var status = EivizSourceStatus(connected: 0, has_video: 0)
        let code = mixer_source_status(id, &status)
        guard code == EIVIZ_OK else { return (false, false) }
        return (status.connected != 0, status.has_video != 0)
    }

    static func sourceErrorText(_ id: UInt64) -> String {
        var buffer = [UInt8](repeating: 0, count: 1024)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_source_copy_error(id, ptr.baseAddress, ptr.count)
        }
        guard n > 0 else { return "" }
        return String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
    }

    static func lastErrorText() -> String {
        var buffer = [UInt8](repeating: 0, count: 1024)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_last_error(ptr.baseAddress, ptr.count)
        }
        guard n > 0 else { return "" }
        return String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
    }

    static func takeFatalText() -> String {
        var buffer = [UInt8](repeating: 0, count: 1024)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_take_fatal(ptr.baseAddress, ptr.count)
        }
        guard n > 0 else { return "" }
        return String(bytes: buffer.prefix(Int(n)), encoding: .utf8) ?? ""
    }

    static func lastError(_ action: String) -> String {
        let detail = lastErrorText()
        return detail.isEmpty ? L10n.error(action, -1) : "\(L10n.error(action, -1)): \(detail)"
    }

    static func check(_ code: Int32, _ action: String) -> String? {
        code == EIVIZ_OK ? nil : L10n.error(action, code)
    }

    nonisolated static func discoverOmt() -> [String] { discover(mixer_omt_discover) }

    nonisolated static func discoverNdi() -> [String] { discover(mixer_ndi_discover) }

    nonisolated static func discover(_ fn: (UnsafeMutablePointer<UInt8>?, Int) -> Int32) -> [String] {
        var buffer = [UInt8](repeating: 0, count: 8192)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            fn(ptr.baseAddress, ptr.count)
        }
        guard n > 0, let text = String(bytes: buffer.prefix(Int(n)), encoding: .utf8) else {
            return []
        }
        return text.split(whereSeparator: \.isNewline).map(String.init).filter { !$0.isEmpty }
    }

    static func copyUtf8(
        startCap: Int = 1 << 20,
        maxCap: Int = 16 << 20,
        _ body: (UnsafeMutablePointer<UInt8>?, Int) -> Int32
    ) -> (Int32, [UInt8]) {
        var cap = startCap
        while true {
            var buffer = [UInt8](repeating: 0, count: cap)
            let n = buffer.withUnsafeMutableBufferPointer { ptr in
                body(ptr.baseAddress, ptr.count)
            }
            if n >= 0 {
                return (n, Array(buffer.prefix(Int(n))))
            }
            if n == -1 && cap < maxCap {
                cap *= 2
                continue
            }
            return (n, [])
        }
    }

    static func sessionCurrentPath() -> String? {
        let (n, bytes) = copyUtf8(startCap: 4096) { mixer_session_current_path($0, $1) }
        guard n > 0 else { return nil }
        return String(bytes: bytes, encoding: .utf8)
    }

    static func sessionHasAssets(_ path: String) -> Bool {
        withCString(path) { mixer_session_has_assets($0) > 0 }
    }

    static func emptyState() -> EivizUnitState { zeroed() }
    static func emptyOverlay() -> EivizOverlayDesc { zeroed() }

    static func setMv(_ state: inout EivizUnitState, index: Int, _ id: UInt64) {
        guard (0..<16).contains(index) else { return }
        withUnsafeMutableBytes(of: &state.mv_slots) { raw in
            raw.bindMemory(to: UInt64.self)[index] = id
        }
    }

    static func videoCaptureModes(deviceId: String) -> [CaptureMode] {
        var buffer = [EivizVideoCaptureMode](repeating: zeroed(), count: 64)
        let n = withCString(deviceId) { cstr in
            buffer.withUnsafeMutableBufferPointer { ptr in
                mixer_video_enum_capture_modes(cstr, ptr.baseAddress, UInt32(ptr.count))
            }
        }
        guard n > 0 else { return [] }
        var seen = Set<String>()
        return buffer.prefix(Int(n)).compactMap { mode in
            guard mode.width > 0, mode.height > 0, mode.fps_den > 0 else { return nil }
            let item = CaptureMode(
                width: mode.width,
                height: mode.height,
                fpsNum: mode.fps_num,
                fpsDen: mode.fps_den,
                format: mode.format
            )
            return seen.insert(item.id).inserted ? item : nil
        }
    }

    static func videoCaptures() -> [VideoCaptureDevice] {
        var buffer = [EivizVideoCaptureInfo](repeating: zeroed(), count: 64)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_video_enum_captures(ptr.baseAddress, UInt32(ptr.count))
        }
        guard n > 0 else { return [] }
        return buffer.prefix(Int(n)).compactMap { info in
            let id = cString(info.id)
            let name = cString(info.name)
            guard !id.isEmpty, !name.isEmpty else { return nil }
            return VideoCaptureDevice(id: id, name: name)
        }
    }

    static func audioDevices() -> [AudioDevice] {
        var buffer = [EivizAudioDeviceInfo](repeating: zeroed(), count: 64)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_audio_enum_devices(0, ptr.baseAddress, UInt32(ptr.count))
        }
        guard n > 0 else { return [] }
        return buffer.prefix(Int(n)).map { info in
            AudioDevice(
                kind: info.kind,
                channels: info.channels,
                id: cString(info.id),
                name: cString(info.name)
            )
        }
    }

    private static func cString<T>(_ value: T) -> String {
        withUnsafePointer(to: value) { ptr in
            ptr.withMemoryRebound(to: UInt8.self, capacity: MemoryLayout<T>.size) { bytes in
                let buffer = UnsafeBufferPointer(start: bytes, count: MemoryLayout<T>.size)
                let n = buffer.firstIndex(of: 0) ?? buffer.count
                return String(decoding: buffer.prefix(n), as: UTF8.self)
            }
        }
    }

    static func zeroed<T>() -> T {
        let pointer = UnsafeMutableRawPointer.allocate(
            byteCount: MemoryLayout<T>.stride,
            alignment: MemoryLayout<T>.alignment
        )
        defer { pointer.deallocate() }
        pointer.initializeMemory(as: UInt8.self, repeating: 0, count: MemoryLayout<T>.stride)
        return pointer.load(as: T.self)
    }

    /// Live replay state. The session document says what is configured; this
    /// says what is actually playing.
    static func replayStats() -> EivizReplayStats? {
        var stats = EivizReplayStats()
        guard mixer_replay_copy_stats(&stats, nil, 0) == EIVIZ_OK else { return nil }
        return stats
    }

    /// Replay stats with the per-angle breakdown, so the operator can see which
    /// stage lost frames.
    static func replayStatsWithAngles(
        _ capacity: Int = 8
    ) -> (stats: EivizReplayStats, angles: [EivizReplayAngleStats])? {
        var stats = EivizReplayStats()
        var angles = [EivizReplayAngleStats](repeating: EivizReplayAngleStats(), count: capacity)
        let status = angles.withUnsafeMutableBufferPointer { pointer in
            mixer_replay_copy_stats(&stats, UnsafeMutableRawPointer(pointer.baseAddress), UInt32(pointer.count))
        }
        guard status == EIVIZ_OK else { return nil }
        let count = min(Int(stats.angle_count), angles.count)
        return (stats, Array(angles.prefix(count)))
    }

    static func replayConfigure(_ json: String) -> Int32 {
        withCString(json) { text in mixer_replay_configure(text) }
    }

    static func replayRecordingUuid() -> String {
        let (n, bytes) = copyUtf8(startCap: 128) { mixer_replay_copy_recording_uuid($0, $1) }
        guard n > 0 else { return "" }
        return String(bytes: bytes, encoding: .utf8) ?? ""
    }

    static func replayRecord(_ on: Bool) -> Int32 {
        mixer_replay_record(on ? 1 : 0)
    }

    static func replayMarkIn() -> UInt64? {
        var frame: UInt64 = 0
        guard mixer_replay_mark_in(&frame) == EIVIZ_OK else { return nil }
        return frame
    }

    static func replayMarkOut() -> (uuid: String, inFrame: UInt64, outFrame: UInt64)? {
        var inFrame: UInt64 = 0
        var outFrame: UInt64 = 0
        var uuid = [UInt8](repeating: 0, count: 80)
        let code = uuid.withUnsafeMutableBufferPointer { ptr in
            mixer_replay_mark_out(&inFrame, &outFrame, ptr.baseAddress, ptr.count)
        }
        guard code == EIVIZ_OK else { return nil }
        let end = uuid.firstIndex(of: 0) ?? uuid.count
        let text = String(decoding: uuid.prefix(end), as: UTF8.self)
        return (text, inFrame, outFrame)
    }

    static func replayCue(channel: UInt32, angleId: UInt32, inFrame: UInt64, outFrame: UInt64, recordingUuid: String) -> Int32 {
        withCString(recordingUuid) { uuid in
            mixer_replay_channel_cue(channel, angleId, inFrame, outFrame, uuid)
        }
    }

    static func replayTransport(channel: UInt32, playing: Bool, speedMilli: Int32, seekFrame: Int64) -> Int32 {
        mixer_replay_channel_transport(channel, playing ? 1 : 0, speedMilli, seekFrame)
    }

    static func replayJumpToNow(channel: UInt32) -> Int32 {
        mixer_replay_channel_jump_to_now(channel)
    }

    /// Moves a channel by `delta` frames inside the clip it is playing.
    static func replayStep(channel: UInt32, delta: Int32) -> Int32 {
        mixer_replay_channel_step(channel, delta)
    }

    /// Loops the channel's playlist at its end.
    static func replayLoop(channel: UInt32, on: Bool) -> Int32 {
        mixer_replay_loop(channel, on ? 1 : 0)
    }

    /// Per-channel live state, so the deck shows what the mixer is doing. Empty when the
    /// mixer reports nothing.
    static func replayChannels() -> [EivizReplayChannelState] {
        var buffer = [EivizReplayChannelState](
            repeating: EivizReplayChannelState(), count: replayChannelLimit)
        let count = buffer.withUnsafeMutableBufferPointer { pointer in
            mixer_replay_copy_channels(pointer.baseAddress, UInt32(pointer.count))
        }
        guard count > 0 else { return [] }
        return Array(buffer.prefix(Int(min(count, UInt32(buffer.count)))))
    }

    /// Channels A-Z. Matches the core's own limit, so the buffer is never short.
    private static let replayChannelLimit = 26

    /// Recorded frame range of one angle, or nil when it has no frames.
    static func replayAngleRange(angleId: UInt32) -> (first: UInt64, last: UInt64)? {
        var first: UInt64 = 0
        var last: UInt64 = 0
        guard mixer_replay_angle_range(angleId, &first, &last) != 0 else { return nil }
        return (first, last)
    }

    /// Starts an MP4 export. The JSON names the file and its segments.
    static func replayExportStart(_ json: String) -> Int32 {
        withCString(json) { text in mixer_replay_export_start(text) }
    }

    /// Progress of the running export as JSON, or an empty string when the mixer
    /// has nothing to report.
    static func replayExportProgress() -> String {
        var buffer = [UInt8](repeating: 0, count: 512)
        var written = 0
        let status = buffer.withUnsafeMutableBufferPointer { pointer in
            mixer_replay_export_progress(pointer.baseAddress, pointer.count, &written)
        }
        guard status == EIVIZ_OK, written > 0 else { return "" }
        return String(decoding: buffer.prefix(written), as: UTF8.self)
    }

    static func replayExportCancel() -> Int32 {
        mixer_replay_export_cancel()
    }

    /// Buckets per second the replay waveform is stored at.
    static func replayPeakRate() -> UInt32 {
        mixer_replay_peak_rate()
    }

    /// Waveform of one angle: the bucket count and the buckets themselves.
    static func replayPeaks(angleId: UInt32) -> (mins: [Float], maxes: [Float], dropped: UInt32) {
        var dropped: UInt32 = 0
        let count = mixer_replay_peak_count(angleId, &dropped)
        guard count > 0 else { return ([], [], dropped) }
        var mins = [Float](repeating: 0, count: Int(count))
        var maxes = [Float](repeating: 0, count: Int(count))
        let copied = mins.withUnsafeMutableBufferPointer { minBuffer in
            maxes.withUnsafeMutableBufferPointer { maxBuffer in
                mixer_replay_copy_peaks(
                    angleId,
                    0,
                    minBuffer.baseAddress,
                    maxBuffer.baseAddress,
                    count
                )
            }
        }
        guard copied > 0 else { return ([], [], dropped) }
        let written = Int(copied)
        return (Array(mins.prefix(written)), Array(maxes.prefix(written)), dropped)
    }

    static func deckLinkDevices() -> (devices: [DeckLinkDeviceInfo], error: String) {
        var buffer = [UInt8](repeating: 0, count: 1 << 16)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_decklink_enum_devices(ptr.baseAddress, ptr.count)
        }
        let decoded: (items: [DeckLinkDeviceInfo], error: String) = decodeDeckLink(buffer, n)
        return (decoded.items, decoded.error)
    }

    static func deckLinkModes(deviceId: String) -> (modes: [DeckLinkModeInfo], error: String) {
        var buffer = [UInt8](repeating: 0, count: 1 << 16)
        let n = withCString(deviceId) { device in
            buffer.withUnsafeMutableBufferPointer { ptr in
                mixer_decklink_enum_modes(device, ptr.baseAddress, ptr.count)
            }
        }
        let decoded: (items: [DeckLinkModeInfo], error: String) = decodeDeckLink(buffer, n)
        return (decoded.items, decoded.error)
    }

    private static func decodeDeckLink<T: Decodable>(_ buffer: [UInt8], _ n: Int32) -> (items: [T], error: String) {
        if n >= 2, n <= buffer.count,
           let items = try? JSONDecoder().decode([T].self, from: Data(buffer.prefix(Int(n)))) {
            return (items, "")
        }
        let detail = lastErrorText()
        return ([], detail.isEmpty ? "DeckLink enumerate failed" : detail)
    }
}

struct DeckLinkDeviceInfo: Decodable, Identifiable, Hashable {
    var id: String
    var model: String = ""
    var name: String = ""
    var capture: Bool = false
    var playback: Bool = false
    var label: String { name.isEmpty ? (model.isEmpty ? id : model) : name }
}

struct DeckLinkModeInfo: Decodable, Identifiable, Hashable {
    var id: String
    var name: String = ""
    var width: UInt32 = 0
    var height: UInt32 = 0
    var fpsNum: UInt32 = 0
    var fpsDen: UInt32 = 1
    var label: String { name.isEmpty ? "\(width)x\(height)" : name }
}
