import Foundation

enum AppLanguage: String, Codable {
    case en
    case ja

    static var systemDefault: AppLanguage {
        Locale.preferredLanguages.first?.hasPrefix("ja") == true ? .ja : .en
    }
}

enum AppThemeMode: String, Codable {
    case dark
    case light
    case system
}

enum HostConnectionMode: String, Codable {
    case local
    case remote
}

final class AppPrefs: ObservableObject {
    nonisolated(unsafe) static let shared = AppPrefs()

    @Published var language: AppLanguage
    @Published var theme: AppThemeMode
    @Published var renderer: GpuRenderer
    @Published var recentSessions: [String]
    @Published var recentStills: [String]
    @Published var recentVideos: [String]
    @Published var connectionMode: HostConnectionMode
    @Published var remoteUrl: String
    @Published var nativeApiEnabled: Bool
    @Published var nativeApiBind: String
    @Published var nativeApiPort: UInt32
    @Published var nativeApiRole: String
    @Published var mediaDirectory: String
    @Published var localeRevision = 0

    static var defaultMediaDirectory: String {
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        return root.appendingPathComponent("eiviz", isDirectory: true)
            .appendingPathComponent("media", isDirectory: true).path
    }

    var resolvedMediaDirectory: String {
        let trimmed = mediaDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? Self.defaultMediaDirectory : trimmed
    }

    private static var storeURL: URL {
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let name = isRemoteProcess ? "remote-prefs.json" : "prefs.json"
        return root.appendingPathComponent("eiviz", isDirectory: true).appendingPathComponent(name)
    }

    static var isRemoteProcess: Bool {
        let name = ProcessInfo.processInfo.processName
        return name.caseInsensitiveCompare("eiviz-remote") == .orderedSame
            || name.caseInsensitiveCompare("Eiviz.Remote") == .orderedSame
    }

    private init() {
        let loaded = Self.load()
        language = loaded.language
        theme = loaded.theme
        renderer = loaded.renderer
        recentSessions = loaded.recentSessions
        recentStills = loaded.recentStills
        recentVideos = loaded.recentVideos
        connectionMode = loaded.connectionMode
        remoteUrl = loaded.remoteUrl
        nativeApiEnabled = loaded.nativeApiEnabled
        nativeApiBind = loaded.nativeApiBind
        nativeApiPort = loaded.nativeApiPort
        nativeApiRole = loaded.nativeApiRole
        mediaDirectory = loaded.mediaDirectory
    }

    func save() {
        var dto = Dto()
        dto.language = language
        dto.theme = theme
        dto.renderer = renderer
        dto.recentSessions = recentSessions
        dto.recentStills = recentStills
        dto.recentVideos = recentVideos
        dto.connectionMode = connectionMode
        dto.remoteUrl = remoteUrl
        dto.nativeApiEnabled = nativeApiEnabled
        dto.nativeApiBind = nativeApiBind
        dto.nativeApiPort = nativeApiPort
        dto.nativeApiRole = nativeApiRole
        dto.mediaDirectory = resolvedMediaDirectory
        do {
            try FileManager.default.createDirectory(at: Self.storeURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(dto).write(to: Self.storeURL, options: .atomic)
        } catch {}
    }

    func rememberSession(_ path: String) {
        recentSessions = remember(recentSessions, path, cap: 12)
        save()
    }

    func rememberStill(_ path: String) {
        recentStills = remember(recentStills, path, cap: 24)
        save()
    }

    func rememberVideo(_ path: String) {
        recentVideos = remember(recentVideos, path, cap: 24)
        save()
    }

    func existingSessions() -> [String] {
        let keep = recentSessions.filter { FileManager.default.fileExists(atPath: $0) }
        if keep.count != recentSessions.count {
            recentSessions = keep
            save()
        }
        return keep
    }

    private func remember(_ list: [String], _ path: String, cap: Int) -> [String] {
        var next = list.filter { $0 != path }
        next.insert(path, at: 0)
        if next.count > cap { next = Array(next.prefix(cap)) }
        return next
    }

    private static func load() -> Dto {
        guard let data = try? Data(contentsOf: storeURL),
              let dto = try? JSONDecoder().decode(Dto.self, from: data)
        else {
            return Dto()
        }
        return dto
    }

    private struct Dto: Codable {
        var language: AppLanguage = .systemDefault
        var theme: AppThemeMode = .dark
        var renderer: GpuRenderer = .auto
        var recentSessions: [String] = []
        var recentStills: [String] = []
        var recentVideos: [String] = []
        var connectionMode: HostConnectionMode = .local
        var remoteUrl: String = "ws://127.0.0.1:9400"
        var nativeApiEnabled: Bool = true
        var nativeApiBind: String = "127.0.0.1"
        var nativeApiPort: UInt32 = 9400
        var nativeApiRole: String = "admin"
        var mediaDirectory: String = AppPrefs.defaultMediaDirectory

        init() {}

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            language = try container.decodeIfPresent(AppLanguage.self, forKey: .language) ?? .systemDefault
            theme = try container.decodeIfPresent(AppThemeMode.self, forKey: .theme) ?? .dark
            renderer = try container.decodeIfPresent(GpuRenderer.self, forKey: .renderer) ?? .auto
            recentSessions = try container.decodeIfPresent([String].self, forKey: .recentSessions) ?? []
            recentStills = try container.decodeIfPresent([String].self, forKey: .recentStills) ?? []
            recentVideos = try container.decodeIfPresent([String].self, forKey: .recentVideos) ?? []
            connectionMode = try container.decodeIfPresent(HostConnectionMode.self, forKey: .connectionMode) ?? .local
            remoteUrl = try container.decodeIfPresent(String.self, forKey: .remoteUrl) ?? "ws://127.0.0.1:9400"
            nativeApiEnabled = try container.decodeIfPresent(Bool.self, forKey: .nativeApiEnabled) ?? true
            nativeApiBind = try container.decodeIfPresent(String.self, forKey: .nativeApiBind) ?? "127.0.0.1"
            nativeApiPort = try container.decodeIfPresent(UInt32.self, forKey: .nativeApiPort) ?? 9400
            nativeApiRole = try container.decodeIfPresent(String.self, forKey: .nativeApiRole) ?? "admin"
            mediaDirectory = {
                let value = try container.decodeIfPresent(String.self, forKey: .mediaDirectory) ?? ""
                let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
                return trimmed.isEmpty ? AppPrefs.defaultMediaDirectory : trimmed
            }()
        }
    }
}
