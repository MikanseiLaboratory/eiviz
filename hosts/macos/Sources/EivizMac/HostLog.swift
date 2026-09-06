import AppKit
import Foundation

enum HostLog {
    static var directory: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/eiviz", isDirectory: true)
    }

    static func install() {
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        NSSetUncaughtExceptionHandler { exception in
            HostLog.write("ERROR", "uncaught \(exception.name.rawValue): \(exception.reason ?? "")")
        }
        write("INFO", "host log init")
    }

    static func write(_ level: String, _ message: String) {
        let line = "\(ISO8601DateFormatter().string(from: Date())) \(level) \(message)\n"
        let url = directory.appendingPathComponent("eiviz-host.log")
        guard let data = line.data(using: .utf8) else { return }
        if FileManager.default.fileExists(atPath: url.path),
           let handle = try? FileHandle(forWritingTo: url)
        {
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: data)
            return
        }
        try? data.write(to: url)
    }
}
