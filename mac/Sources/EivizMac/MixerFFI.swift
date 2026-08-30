import EivizMixer
import Foundation

enum MixerFFI {
    static func lastError(_ action: String) -> String {
        var buffer = [UInt8](repeating: 0, count: 512)
        let n = buffer.withUnsafeMutableBufferPointer { ptr in
            mixer_last_error(ptr.baseAddress, ptr.count)
        }
        if n > 0, let detail = String(bytes: buffer.prefix(Int(n)), encoding: .utf8), !detail.isEmpty {
            return "\(action): \(detail)"
        }
        return "\(action) failed."
    }

    static func check(_ code: Int32, _ action: String) -> String? {
        code == EIVIZ_OK ? nil : lastError(action)
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
