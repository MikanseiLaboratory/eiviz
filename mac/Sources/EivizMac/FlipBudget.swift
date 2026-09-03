import AppKit
import Foundation

enum GpuPresentStore {
    private struct Dto: Codable {
        var observedCeiling: Int
    }

    nonisolated(unsafe) private(set) static var observedCeiling: Int?

    static func load() {
        let url = storeURL()
        guard FileManager.default.fileExists(atPath: url.path),
              let data = try? Data(contentsOf: url),
              let dto = try? JSONDecoder().decode(Dto.self, from: data),
              (2...16).contains(dto.observedCeiling)
        else { return }
        observedCeiling = dto.observedCeiling
    }

    static func save(_ ceiling: Int) {
        guard ceiling >= 2 else { return }
        observedCeiling = ceiling
        let url = storeURL()
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            let data = try JSONEncoder().encode(Dto(observedCeiling: ceiling))
            try data.write(to: url)
        } catch {
            HostLog.write("WARN", "gpu-present save failed: \(error.localizedDescription)")
        }
    }

    private static func storeURL() -> URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("eiviz", isDirectory: true)
            .appendingPathComponent("gpu-present.json")
    }
}

enum FlipBudget {
    static let autoDefault = 6
    private static let learnWindowMs: UInt64 = 3000

    nonisolated(unsafe) private static var limitSetting: UInt32 = 0
    nonisolated(unsafe) private static var ceiling = autoDefault
    nonisolated(unsafe) private static var attached = 0
    nonisolated(unsafe) private static weak var lastAttach: MetalSurfaceView?
    nonisolated(unsafe) private static var attachTick: UInt64 = 0
    nonisolated(unsafe) private static var seenLost: UInt64 = 0

    static func configure(_ limit: UInt32) {
        limitSetting = isAllowed(limit) ? limit : 0
        ceiling = limitSetting == 0
            ? min(GpuPresentStore.observedCeiling ?? autoDefault, autoDefault)
            : Int(limitSetting)
    }

    static func tryOpen(_ surfaces: Int) -> Bool {
        guard surfaces > 0 else { return true }
        if attached + surfaces <= effectiveMax() {
            return true
        }
        showRefuse()
        return false
    }

    static func tryBegin(_ host: MetalSurfaceView) -> Bool {
        if attached >= effectiveMax() {
            return false
        }
        attached += 1
        lastAttach = host
        attachTick = nowMs()
        return true
    }

    static func cancel(_ host: MetalSurfaceView) {
        if attached > 0 {
            attached -= 1
        }
        if lastAttach === host {
            lastAttach = nil
        }
    }

    static func end(_ host: MetalSurfaceView) {
        cancel(host)
    }

    static func observeLost(_ total: UInt64) {
        guard total > seenLost else { return }
        seenLost = total
        guard let victim = lastAttach, attached > 2 else { return }
        guard nowMs() &- attachTick <= learnWindowMs else { return }
        victim.releaseNative()
        if limitSetting == 0, attached >= 2 {
            GpuPresentStore.save(attached)
        }
        ceiling = max(2, attached)
    }

    private static func effectiveMax() -> Int {
        limitSetting == 0 ? ceiling : Int(limitSetting)
    }

    private static func isAllowed(_ limit: UInt32) -> Bool {
        limit == 0 || limit == 4 || limit == 6 || limit == 8 || limit == 10 || limit == 12 || limit == 16
    }

    private static func showRefuse() {
        let alert = NSAlert()
        alert.messageText = L10n.t("msg.flipBudget")
        alert.alertStyle = .informational
        alert.runModal()
    }

    private static func nowMs() -> UInt64 {
        DispatchTime.now().uptimeNanoseconds / 1_000_000
    }
}
