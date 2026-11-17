import CryptoKit
import Foundation
import SwiftUI

/// Locates or installs the OpenH264 dynamic library the Pro module loads.
///
/// The module dlopens `libopenh264` when a stream starts; it does not link it, so a
/// missing library is reported as `OPENH264_PATH is not set and
/// openh264-2.6.0-win64.dll is not next to the executable`. Cisco publishes a signed
/// macOS build, so the host fetches it on first use and keeps it in Application
/// Support. `OPENH264_PATH` wins over every candidate, so a package or CI run can pin
/// its own copy.
@MainActor
final class OpenH264Library: ObservableObject {
    static let shared = OpenH264Library()

    enum Source: String {
        case environment = "OPENH264_PATH"
        case bundled
        case downloaded
        case homebrew
    }

    enum State: Equatable {
        case checking
        case ready(path: String, source: String)
        case downloading
        case failed(String)
    }

    @Published private(set) var state: State = .checking

    private var task: Task<Void, Never>?

    private static let remoteBase = "https://ciscobinary.openh264.org"
    /// Cisco's binary for Apple Silicon. The module only accepts the 2.6.0 API.
    private static let remoteName = "libopenh264-2.6.0-mac-arm64.dylib"

    /// Starts a lookup in the background. Repeated calls while one is running are
    /// no-ops; the module reads the environment at stream start, so finishing after
    /// boot is soon enough.
    func prepare(force: Bool = false) {
        guard task == nil else { return }
        task = Task { [weak self] in
            await self?.locateOrDownload(force: force)
            self?.task = nil
        }
    }

    /// Where a copy would be installed.
    static var installDirectory: URL? {
        guard let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first else { return nil }
        return base.appendingPathComponent("eiviz/openh264", isDirectory: true)
    }

    static var installedPath: URL? {
        installDirectory?.appendingPathComponent("libopenh264.dylib")
    }

    private func locateOrDownload(force: Bool) async {
        if !force, let found = Self.existing() {
            adopt(found.path, source: found.source)
            return
        }
        state = .downloading
        do {
            let path = try await install()
            adopt(path, source: Source.downloaded.rawValue)
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    private struct Found {
        var path: String
        var source: String
    }

    /// Search order: an operator-supplied path, the bundle, an earlier download, then
    /// Homebrew (the usual place a developer machine already has one).
    private static func existing() -> Found? {
        let env = ProcessInfo.processInfo.environment
        if let path = env["OPENH264_PATH"], FileManager.default.fileExists(atPath: path) {
            return Found(path: path, source: Source.environment.rawValue)
        }
        if let executable = Bundle.main.executableURL {
            let bundled = executable.deletingLastPathComponent()
                .appendingPathComponent("libopenh264.dylib").path
            if FileManager.default.fileExists(atPath: bundled) {
                return Found(path: bundled, source: Source.bundled.rawValue)
            }
        }
        if let installed = installedPath?.path,
           FileManager.default.fileExists(atPath: installed) {
            return Found(path: installed, source: Source.downloaded.rawValue)
        }
        // A test or an operator that wants the pinned Cisco build can ask the host to
        // ignore whatever the machine happens to have installed.
        if env["EIVIZ_OPENH264_IGNORE_SYSTEM"] == nil {
            for path in [
                "/opt/homebrew/lib/libopenh264.dylib",
                "/usr/local/lib/libopenh264.dylib",
            ] where FileManager.default.fileExists(atPath: path) {
                return Found(path: path, source: Source.homebrew.rawValue)
            }
        }
        return nil
    }

    private func adopt(_ path: String, source: String) {
        setenv("OPENH264_PATH", path, 1)
        HostLog.write("INFO", "OpenH264 library: \(source) \(path)")
        state = .ready(path: path, source: source)
    }

    /// Downloads and verifies Cisco's build, then installs it next to the app's data.
    private func install() async throws -> String {
        guard let directory = Self.installDirectory else {
            throw OpenH264Error.message("Application Support is unavailable")
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let destination = directory.appendingPathComponent("libopenh264.dylib")

        let source = "\(Self.remoteBase)/\(Self.remoteName)"
        let digest = try await remoteDigest()
        let archive = try await fetch(URL(string: source + ".bz2")!)
        // Keep the `.bz2` name: `bzip2 -d` derives the output name by stripping it,
        // and it writes `<name>.out` instead when the suffix is missing.
        let unpacked = directory.appendingPathComponent(Self.remoteName + ".bz2")
        try archive.write(to: unpacked)
        defer { try? FileManager.default.removeItem(at: unpacked) }

        try runBzip2(unpacked)
        let product = directory.appendingPathComponent(Self.remoteName)
        defer { try? FileManager.default.removeItem(at: product) }
        let actual = try Self.md5(of: product)
        guard actual.caseInsensitiveCompare(digest) == .orderedSame else {
            throw OpenH264Error.message(
                "the downloaded library did not match Cisco's checksum"
            )
        }
        try? FileManager.default.removeItem(at: destination)
        try FileManager.default.moveItem(at: product, to: destination)
        try? FileManager.default.setAttributes(
            [.posixPermissions: 0o644],
            ofItemAtPath: destination.path
        )
        return destination.path
    }

    /// Cisco publishes `MD5 (name) = <hex>`; the file is fetched over the same TLS
    /// connection as the archive, so this catches a truncated or swapped download.
    private func remoteDigest() async throws -> String {
        let url = URL(string: "\(Self.remoteBase)/\(Self.remoteName).signed.md5.txt")!
        let data = try await fetch(url)
        guard let text = String(data: data, encoding: .utf8),
              let range = text.range(of: "= ")
        else {
            throw OpenH264Error.message("the checksum file could not be read")
        }
        let digest = text[range.upperBound...]
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard digest.count == 32 else {
            throw OpenH264Error.message("the checksum file did not name a digest")
        }
        return digest
    }

    private func fetch(_ url: URL) async throws -> Data {
        var request = URLRequest(url: url)
        request.timeoutInterval = 30
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw OpenH264Error.message("the download answered \(url.lastPathComponent)")
        }
        return data
    }

    /// `bzip2 -d` replaces `<name>.bz2` with `<name>`.
    private func runBzip2(_ archive: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/bzip2")
        process.arguments = ["-df", archive.path]
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw OpenH264Error.message("the archive could not be unpacked")
        }
    }

    private static func md5(of url: URL) throws -> String {
        let data = try Data(contentsOf: url)
        return Insecure.MD5.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
}

enum OpenH264Error: LocalizedError {
    case message(String)

    var errorDescription: String? {
        switch self {
        case let .message(text): text
        }
    }
}

/// The OpenH264 row on the Stream settings page: what the module will load, and a
/// way to fetch it again.
struct OpenH264StatusCard: View {
    @ObservedObject private var library = OpenH264Library.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("OpenH264").fontWeight(.bold)
                Spacer()
                Button(L10n.t("settings.openH264Retry")) {
                    library.prepare(force: true)
                }
                .disabled(isBusy)
            }
            Text(statusText)
                .font(.caption)
                .foregroundStyle(Color(white: 0.5))
                .fixedSize(horizontal: false, vertical: true)
            Text(L10n.t("settings.openH264Help"))
                .font(.caption2)
                .foregroundStyle(Color(white: 0.5))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(8)
        .background(
            Rectangle()
                .stroke(Color(white: 0.27), lineWidth: 1)
        )
        .onAppear { library.prepare() }
    }

    private var isBusy: Bool {
        switch library.state {
        case .checking, .downloading: true
        case .ready, .failed: false
        }
    }

    private var statusText: String {
        switch library.state {
        case .checking:
            L10n.t("settings.openH264Checking")
        case let .ready(path, source):
            L10n.format("settings.openH264Ready", source, path)
        case .downloading:
            L10n.t("settings.openH264Downloading")
        case let .failed(reason):
            L10n.format("settings.openH264Failed", reason)
        }
    }
}
