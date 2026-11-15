import AppKit
import SwiftUI

/// Logs and resource telemetry used to be sheets hanging off the main window. They
/// are operator tools that stay open while the show runs, so they get their own
/// windows and do not cover the program/preview surfaces.
@MainActor
enum ToolWindows {
    private static var logs: NSWindow?
    private static var resources: NSWindow?

    static func showLogs() {
        if let logs {
            logs.makeKeyAndOrderFront(nil)
            return
        }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 920, height: 560),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = L10n.t("logs.title")
        window.contentView = NSHostingView(rootView: LogsView())
        window.center()
        window.isReleasedWhenClosed = false
        window.makeKeyAndOrderFront(nil)
        logs = window
    }

    static func showResources(_ mixer: MixerController) {
        if let resources {
            resources.makeKeyAndOrderFront(nil)
            return
        }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 820, height: 480),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = L10n.t("resources.title")
        window.contentView = NSHostingView(
            rootView: ResourcesView().environmentObject(mixer)
        )
        window.center()
        window.isReleasedWhenClosed = false
        window.makeKeyAndOrderFront(nil)
        resources = window
    }
}
