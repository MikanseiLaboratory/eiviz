import Foundation

enum TcpListenOwner {
    static func name(port: UInt32) -> String? {
        guard port > 0, port <= 65_535 else { return nil }
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/sbin/lsof")
        proc.arguments = ["-nP", "-iTCP:\(port)", "-sTCP:LISTEN"]
        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = FileHandle.nullDevice
        do {
            try proc.run()
            proc.waitUntilExit()
        } catch {
            return nil
        }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard let text = String(data: data, encoding: .utf8) else { return nil }
        let selfPid = ProcessInfo.processInfo.processIdentifier
        for line in text.split(whereSeparator: \.isNewline).dropFirst() {
            let cols = line.split(whereSeparator: \.isWhitespace)
            guard cols.count >= 2, let pid = Int32(cols[1]), pid != selfPid else { continue }
            let raw = String(cols[0])
            if !raw.isEmpty {
                return friendlyName(raw)
            }
        }
        return nil
    }

    private static func friendlyName(_ raw: String) -> String {
        switch raw.lowercased() {
        case "vmix", "vmix64", "vmix64bit":
            return "vMix"
        case "httpd", "apache", "apache2":
            return "Apache HTTP Server"
        case "nginx":
            return "nginx"
        case "w3wp", "iisexpress":
            return "IIS"
        default:
            return raw
        }
    }
}
