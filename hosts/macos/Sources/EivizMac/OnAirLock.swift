import AppKit

/// Operator-controlled guard against closing eiviz while it is on air.
///
/// This is intentionally a manual switch, matching the Windows host. It is not
/// inferred from the program bus: a show can be live through an output without a
/// scene being on air in the UI.
enum OnAirLock {
    nonisolated(unsafe) private(set) static var active = false
    static let changed = Notification.Name("eiviz.onAirLock.changed")

    static func set(_ enabled: Bool) {
        guard active != enabled else { return }
        active = enabled
        NotificationCenter.default.post(name: changed, object: nil)
    }

    static func cancelClose(_ window: NSWindow) -> Bool {
        guard active else { return false }
        let alert = NSAlert()
        alert.messageText = L10n.t("chrome.onAirLockOn")
        alert.informativeText = L10n.t("msg.onAirLock")
        alert.alertStyle = .warning
        alert.addButton(withTitle: L10n.t("dialog.ok"))
        alert.beginSheetModal(for: window)
        return true
    }
}

/// SwiftUI's WindowGroup has no per-window delegate hook, so the app installs this
/// on the main window when it becomes key. Closing the deck while On Air Lock is on
/// must be refused exactly like the Windows host's OnClosing.
final class OnAirWindowDelegate: NSObject, NSWindowDelegate {
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        !OnAirLock.cancelClose(sender)
    }
}

final class EivizAppDelegate: NSObject, NSApplicationDelegate {
    private let closeGuard = OnAirWindowDelegate()
    private var observer: NSObjectProtocol?

    /// AppKit reads this before it starts restoring windows, and SwiftUI's own
    /// restore path does not honour the delegate answers. A window the previous
    /// run left mid-restore makes the launch wait forever, so the switch is set
    /// here, during launch, and the session decides the windows instead.
    func applicationWillFinishLaunching(_ notification: Notification) {
        UserDefaults.standard.set(false, forKey: "NSQuitAlwaysKeepsWindows")
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        observer = NotificationCenter.default.addObserver(
            forName: NSWindow.didBecomeKeyNotification,
            object: nil,
            queue: .main
        ) { [weak self] note in
            guard let self, let window = note.object as? NSWindow,
                  window.level == .normal,
                  window.delegate !== self.closeGuard
            else { return }
            window.delegate = self.closeGuard
        }
    }

    /// Window state restoration is off for this app.
    ///
    /// The session is what decides the windows and the layout, and AppKit's
    /// automatic restore runs before the session is applied. On a machine where a
    /// previous run was killed, that restore waits for a SwiftUI window that is
    /// never going to come back and the app hangs before the first frame: the
    /// operator sees a launch that never finishes. Nothing here needs the
    /// restoration, so it is refused outright.
    func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
        false
    }

    func applicationShouldSaveSecureApplicationState(_ app: NSApplication) -> Bool {
        false
    }

    /// The pre-Secure-Coding pair. AppKit still asks these first on a plain
    /// launch, and it is the pair that decides whether the restore that hung
    /// `_batchMinimizeWindows` runs at all.
    func applicationShouldSaveApplicationState(_ app: NSApplication) -> Bool {
        false
    }

    func applicationShouldRestoreApplicationState(_ app: NSApplication) -> Bool {
        false
    }

    func applicationShouldRestoreSecureApplicationState(_ app: NSApplication) -> Bool {
        false
    }

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
