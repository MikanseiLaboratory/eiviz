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

final class AppPrefs: ObservableObject {
    nonisolated(unsafe) static let shared = AppPrefs()

    @Published var language: AppLanguage
    @Published var theme: AppThemeMode
    @Published var recentSessions: [String]
    @Published var recentStills: [String]
    @Published var recentVideos: [String]
    @Published var localeRevision = 0

    private static var storeURL: URL {
        let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        return root.appendingPathComponent("eiviz", isDirectory: true).appendingPathComponent("prefs.json")
    }

    private init() {
        let loaded = Self.load()
        language = loaded.language
        theme = loaded.theme
        recentSessions = loaded.recentSessions
        recentStills = loaded.recentStills
        recentVideos = loaded.recentVideos
    }

    func save() {
        var dto = Dto()
        dto.language = language
        dto.theme = theme
        dto.recentSessions = recentSessions
        dto.recentStills = recentStills
        dto.recentVideos = recentVideos
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
        var recentSessions: [String] = []
        var recentStills: [String] = []
        var recentVideos: [String] = []
    }
}
