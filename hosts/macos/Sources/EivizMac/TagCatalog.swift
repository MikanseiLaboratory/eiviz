import AppKit
import SwiftUI

enum TagCatalog {
    static func normalize(_ name: String?) -> String? {
        let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    static func normalizeList(_ tags: [String]?) -> [String] {
        var result: [String] = []
        for tag in tags ?? [] {
            guard let name = normalize(tag) else { continue }
            if !contains(result, name) {
                result.append(name)
            }
        }
        return result
    }

    static func mergeInto(_ catalog: inout [String], _ tags: [String]?) {
        for tag in normalizeList(tags) where !contains(catalog, tag) {
            catalog.append(tag)
        }
    }

    @discardableResult
    static func tryAdd(_ catalog: inout [String], _ name: String?) -> (ok: Bool, normalized: String) {
        let value = normalize(name) ?? ""
        if value.isEmpty {
            return (false, "")
        }
        if contains(catalog, value) {
            return (false, value)
        }
        catalog.append(value)
        return (true, value)
    }

    @discardableResult
    static func rename(_ catalog: inout [String], owners: inout [[String]], current: String, next: String?) -> Bool {
        guard let name = normalize(next) else { return false }
        guard let index = catalog.firstIndex(where: { $0 == current }) else { return false }
        if name != current && contains(catalog, name) {
            return false
        }
        catalog[index] = name
        for i in owners.indices {
            for j in owners[i].indices where owners[i][j] == current {
                owners[i][j] = name
            }
        }
        return true
    }

    static func remove(_ catalog: inout [String], owners: inout [[String]], name: String) {
        catalog.removeAll { $0 == name }
        for i in owners.indices {
            owners[i].removeAll { $0 == name }
        }
    }

    static func contains(_ tags: [String], _ name: String) -> Bool {
        tags.contains { $0 == name }
    }

    static func replace(_ target: inout [String], _ tags: [String]?) {
        target = normalizeList(tags)
    }
}

enum ListFilterMode {
    case all
    case tag
    case kind
}

struct ListFilter: Equatable {
    var mode: ListFilterMode = .all
    var tag: String?
    var kind: InputKind?

    static let all = ListFilter()

    static func tag(_ tag: String) -> ListFilter {
        ListFilter(mode: .tag, tag: tag)
    }

    static func kind(_ kind: InputKind) -> ListFilter {
        ListFilter(mode: .kind, kind: kind)
    }

    func matchesInput(_ input: InputEntry) -> Bool {
        switch mode {
        case .tag:
            guard let tag else { return true }
            return TagCatalog.contains(input.tags, tag)
        case .kind:
            guard let kind else { return true }
            return input.kind.sameCategory(as: kind)
        case .all:
            return true
        }
    }

    func matchesScene(_ scene: SceneEntry) -> Bool {
        switch mode {
        case .tag:
            guard let tag else { return true }
            return TagCatalog.contains(scene.tags, tag)
        default:
            return true
        }
    }
}

enum TextPrompt {
    @MainActor
    static func ask(title: String, prompt: String, initial: String) -> String? {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = prompt
        alert.addButton(withTitle: L10n.t("dialog.ok"))
        alert.addButton(withTitle: L10n.t("dialog.cancel"))
        let field = NSTextField(string: initial)
        field.frame = NSRect(x: 0, y: 0, width: 280, height: 24)
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        AppKitDialog.elevate(alert)
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        return TagCatalog.normalize(field.stringValue)
    }

    @MainActor
    static func confirm(title: String, message: String) -> Bool {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: L10n.t("tag.delete"))
        alert.addButton(withTitle: L10n.t("dialog.cancel"))
        AppKitDialog.elevate(alert)
        return alert.runModal() == .alertFirstButtonReturn
    }
}

enum AppKitDialog {
    static func elevate(_ alert: NSAlert) {
        NSApp.activate(ignoringOtherApps: true)
        alert.window.appearance = NSApp.appearance
        alert.window.level = .modalPanel
        alert.window.backgroundColor = EivizTheme.nsBackground
    }

    static func apply(_ panel: NSSavePanel) {
        panel.appearance = NSApp.appearance
    }

    static func toast(_ message: String) {
        let window = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 320, height: 48),
            styleMask: [.titled, .fullSizeContentView, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isFloatingPanel = true
        window.level = .statusBar
        window.appearance = NSApp.appearance
        window.backgroundColor = EivizTheme.nsBackground
        window.isReleasedWhenClosed = false
        let label = NSTextField(labelWithString: message)
        label.alignment = .center
        label.lineBreakMode = .byWordWrapping
        label.maximumNumberOfLines = 3
        label.translatesAutoresizingMaskIntoConstraints = false
        let wrap = NSView()
        wrap.translatesAutoresizingMaskIntoConstraints = false
        wrap.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: wrap.leadingAnchor, constant: 20),
            label.trailingAnchor.constraint(equalTo: wrap.trailingAnchor, constant: -20),
            label.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 12),
            label.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -12)
        ])
        window.contentView = wrap
        if let parent = NSApp.keyWindow ?? NSApp.mainWindow {
            let parentFrame = parent.frame
            let size = wrap.fittingSize
            let width = max(220, min(420, size.width + 40))
            let height = max(48, size.height)
            window.setContentSize(NSSize(width: width, height: height))
            window.setFrameOrigin(NSPoint(
                x: parentFrame.midX - width / 2,
                y: parentFrame.minY + 48
            ))
        } else {
            window.center()
        }
        window.orderFrontRegardless()
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.6) {
            window.close()
        }
    }
}
}

struct CatalogTabBar: View {
    @EnvironmentObject private var mixer: MixerController
    let input: Bool

    private var catalog: [String] {
        input ? mixer.session.inputTags : mixer.session.sceneTags
    }

    private var selected: ListFilter {
        input ? mixer.inputFilter : mixer.sceneFilter
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                tab(L10n.t("tag.all"), .all)
                ForEach(catalog, id: \.self) { name in
                    tab(name, .tag(name))
                }
                if input {
                    ForEach(InputKind.tabKinds, id: \.self) { kind in
                        tab(kind.category, .kind(kind))
                    }
                }
            }
        }
    }

    private func tab(_ title: String, _ filter: ListFilter) -> some View {
        let on = selected.sameAs(filter)
        return Button(title) {
            if input {
                mixer.inputFilter = filter
            } else {
                mixer.sceneFilter = filter
            }
        }
        .buttonStyle(.plain)
        .font(.system(size: 11, weight: on ? .semibold : .regular))
        .foregroundStyle(on ? Color.white : Color.secondary)
        .padding(.bottom, 2)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(on ? mixer.session.settings.previewColor.color : Color.clear)
                .frame(height: 2)
        }
        .contextMenu {
            Button(L10n.t("tag.add")) { mixer.addCatalogTag(input: input) }
            if filter.mode == .tag, let tag = filter.tag {
                Button(L10n.t("tag.rename")) { mixer.renameCatalogTag(input: input, current: tag) }
                Button(L10n.t("tag.delete"), role: .destructive) {
                    mixer.deleteCatalogTag(input: input, name: tag)
                }
            }
        }
    }
}

struct TagCheckView: View {
    @EnvironmentObject private var mixer: MixerController
    let input: Bool
    @Binding var selected: [String]

    private var catalog: [String] {
        input ? mixer.session.inputTags : mixer.session.sceneTags
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(L10n.t("tag.tags"))
            HStack(alignment: .top, spacing: 8) {
                WrapFlowLayout(spacing: 8) {
                    ForEach(catalog, id: \.self) { tag in
                        Toggle(tag, isOn: binding(for: tag))
                            .toggleStyle(.checkbox)
                    }
                }
                Button("+") {
                    if let name = mixer.promptTagForCheck(input: input) {
                        if !TagCatalog.contains(selected, name) {
                            selected.append(name)
                        }
                    }
                }
                .buttonStyle(MixerButtonStyle())
            }
        }
    }

    private func binding(for tag: String) -> Binding<Bool> {
        Binding(
            get: { TagCatalog.contains(selected, tag) },
            set: { on in
                if on {
                    if !TagCatalog.contains(selected, tag) {
                        selected.append(tag)
                    }
                } else {
                    selected.removeAll { $0 == tag }
                }
            }
        )
    }
}

private extension ListFilter {
    func sameAs(_ other: ListFilter) -> Bool {
        switch (mode, other.mode) {
        case (.all, .all):
            return true
        case (.tag, .tag):
            return tag == other.tag
        case (.kind, .kind):
            guard let left = kind, let right = other.kind else { return false }
            return left.sameCategory(as: right)
        default:
            return false
        }
    }
}
