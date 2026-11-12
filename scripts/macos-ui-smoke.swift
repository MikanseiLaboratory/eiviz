#!/usr/bin/env swift
// Dumps the macOS app's window list and accessibility tree, and optionally
// presses one button by description. Run it against a locally built app:
//
//   swift scripts/macos-ui-smoke.swift <pid> [output-dir]
//   swift scripts/macos-ui-smoke.swift <pid> --press "設定"
//
// The script is intentionally not part of CI: TCC prompts and a logged-in
// WindowServer are required for a real GUI run.
import ApplicationServices
import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count >= 2, let pid = pid_t(args[1]) else {
    FileHandle.standardError.write(Data("usage: macos-ui-smoke.swift <pid> [output-dir|--press <description>]\n".utf8))
    exit(2)
}
let app = AXUIElementCreateApplication(pid)

func attr(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

func text(_ element: AXUIElement, _ name: String) -> String {
    attr(element, name) as? String ?? ""
}

func enabled(_ element: AXUIElement) -> String {
    (attr(element, kAXEnabledAttribute) as? Bool).map { $0 ? "enabled" : "disabled" } ?? ""
}

func dump(_ element: AXUIElement, _ depth: Int, _ out: inout [String]) {
    if depth > 12 { return }
    let role = text(element, kAXRoleAttribute)
    let title = text(element, kAXTitleAttribute)
    let value = text(element, kAXValueAttribute)
    let desc = text(element, kAXDescriptionAttribute)
    let help = text(element, kAXHelpAttribute)
    let ident = text(element, kAXIdentifierAttribute)
    out.append(String(repeating: "  ", count: depth)
        + "\(role) title=\(title) value=\(value) desc=\(desc) help=\(help) id=\(ident) \(enabled(element))")
    if let children = attr(element, kAXChildrenAttribute) as? [AXUIElement] {
        for child in children { dump(child, depth + 1, &out) }
    }
}

func find(_ element: AXUIElement, _ wanted: String, _ depth: Int) -> AXUIElement? {
    if depth > 12 { return nil }
    if text(element, kAXDescriptionAttribute) == wanted || text(element, kAXTitleAttribute) == wanted {
        return element
    }
    if let children = attr(element, kAXChildrenAttribute) as? [AXUIElement] {
        for child in children {
            if let found = find(child, wanted, depth + 1) { return found }
        }
    }
    return nil
}

if !AXIsProcessTrusted() {
    FileHandle.standardError.write(Data("Accessibility permission is required for the terminal running this script.\n".utf8))
    exit(3)
}

if let pressIndex = args.firstIndex(of: "--press"), pressIndex + 1 < args.count {
    let wanted = args[pressIndex + 1]
    guard let element = find(app, wanted, 0) else {
        FileHandle.standardError.write(Data("not found: \(wanted)\n".utf8))
        exit(4)
    }
    let error = AXUIElementPerformAction(element, kAXPressAction as CFString)
    print("press \(wanted) AXError=\(error.rawValue)")
    exit(error == .success ? 0 : 5)
}

let outputDir = args.count >= 3 ? URL(fileURLWithPath: args[2], isDirectory: true) : URL(fileURLWithPath: "target/ui-smoke", isDirectory: true)
    .appendingPathComponent(ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-"), isDirectory: true)
try? FileManager.default.createDirectory(at: outputDir, withIntermediateDirectories: true)

var windows: [String] = []
var screenshots: [(Int, String)] = []
if let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] {
    for window in list where (window[kCGWindowOwnerPID as String] as? Int32) == pid {
        let number = window[kCGWindowNumber as String] as? Int ?? 0
        windows.append("window=\(number) name=\(window[kCGWindowName as String] as? String ?? "") bounds=\(window[kCGWindowBounds as String] ?? "") layer=\(window[kCGWindowLayer as String] ?? "")")
        if let bounds = window[kCGWindowBounds as String] as? [String: Any],
           let height = bounds["Height"] as? Double, height > 40
        {
            screenshots.append((number, "window-\(number).png"))
        }
    }
}
try windows.joined(separator: "\n").write(to: outputDir.appendingPathComponent("windows.txt"), atomically: true, encoding: .utf8)
for (number, name) in screenshots {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    process.arguments = ["-x", "-l", "\(number)", outputDir.appendingPathComponent(name).path]
    try? process.run()
    process.waitUntilExit()
}

var lines: [String] = []
dump(app, 0, &lines)
try lines.joined(separator: "\n").write(to: outputDir.appendingPathComponent("ax.txt"), atomically: true, encoding: .utf8)
print("wrote \(outputDir.path) (\(windows.count) windows, \(lines.count) AX rows, \(screenshots.count) screenshots)")
