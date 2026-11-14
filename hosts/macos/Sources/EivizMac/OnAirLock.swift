import AppKit

/// Operator-controlled guard against closing eiviz while it is on air.
///
/// This is intentionally a manual switch, matching the Windows host. It is not
/// inferred from the program bus: a show can be live through an output without a
/// scene being on air in the UI.
enum OnAirLock {
    nonisolated(unsafe) private(set) static var active = false

    static func set(_ enabled: Bool) {
        active = enabled
    }
}

final class EivizAppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard OnAirLock.active else { return .terminateNow }
        let alert = NSAlert()
        alert.messageText = L10n.t("chrome.onAirLockOn")
        alert.informativeText = L10n.t("msg.onAirLock")
        alert.alertStyle = .warning
        alert.addButton(withTitle: L10n.t("dialog.cancel"))
        alert.runModal()
        return .terminateCancel
    }
}
