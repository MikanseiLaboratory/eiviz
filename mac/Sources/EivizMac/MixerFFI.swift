import EivizMixer
import Foundation

enum MixerFFI {
    static func preloadRamWarning() -> String? {
        switch lastErrorText() {
        case "preload-ram-overflow": L10n.t("error.videoPreloadRam")
        case "preload-ram-failed": L10n.t("error.videoPreloadFailed")
        default: nil
        }
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

    static func discover(_ fn: (UnsafeMutablePointer<UInt8>?, Int) -> Int32) -> [String] {
        var buffer = [UInt8](repeating: 0, count: 8192)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            fn(ptr.baseAddress, ptr.count)
        }
        guard n > 0, let text = String(bytes: buffer.prefix(Int(n)), encoding: .utf8) else {
            return []
        }
        return text.split(whereSeparator: \.isNewline).map(String.init).filter { !$0.isEmpty }
    }

    static func withCString<T>(_ string: String, _ body: (UnsafePointer<CChar>) -> T) -> T {
        string.withCString(body)
    }

    static func emptyState() -> EivizUnitState { zeroed() }
    static func emptyOverlay() -> EivizOverlayDesc { zeroed() }

    static func setOverlay(_ state: inout EivizUnitState, index: Int, _ desc: EivizOverlayDesc) {
        guard (0..<8).contains(index) else { return }
        withUnsafeMutableBytes(of: &state.overlays) { raw in
            raw.bindMemory(to: EivizOverlayDesc.self)[index] = desc
        }
    }

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
}
