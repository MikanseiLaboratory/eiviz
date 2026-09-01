import AppKit
import SwiftUI

enum EivizTheme {
    static var isDark: Bool {
        switch AppPrefs.shared.theme {
        case .dark: return true
        case .light: return false
        case .system:
            return NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        }
    }

    static var background: Color { isDark ? rgb(26, 26, 26) : rgb(240, 240, 240) }
    static var chrome: Color { isDark ? rgb(37, 37, 37) : rgb(228, 228, 228) }
    static var panel: Color { isDark ? rgb(22, 22, 22) : rgb(234, 234, 234) }
    static var list: Color { isDark ? rgb(34, 34, 34) : rgb(255, 255, 255) }
    static var statusBar: Color { isDark ? rgb(17, 17, 17) : rgb(224, 224, 224) }
    static var videoBar: Color { isDark ? rgb(20, 20, 20) : rgb(232, 232, 232) }
    static var dialog: Color { isDark ? rgb(43, 43, 43) : rgb(245, 245, 245) }
    static var dialogSide: Color { isDark ? rgb(30, 30, 30) : rgb(232, 232, 232) }
    static var text: Color { isDark ? rgb(238, 238, 238) : rgb(17, 17, 17) }
    static var dim: Color { isDark ? rgb(170, 170, 170) : rgb(102, 102, 102) }
    static var status: Color { isDark ? rgb(124, 252, 124) : rgb(46, 125, 50) }
    static var warn: Color { isDark ? rgb(255, 183, 77) : rgb(230, 81, 0) }
    static var hud: Color { isDark ? rgb(156, 204, 101) : rgb(85, 139, 47) }
    static var preview = Color(red: 0, green: 1, blue: 0)
    static var program = Color(red: 1, green: 0, blue: 0)
    static var button: Color { isDark ? rgb(58, 58, 58) : rgb(221, 221, 221) }
    static var stroke: Color { isDark ? Color(white: 0.27) : Color(white: 0.73) }

    private static func rgb(_ r: Double, _ g: Double, _ b: Double) -> Color {
        Color(red: r / 255, green: g / 255, blue: b / 255)
    }
}

extension RgbColor {
    var color: Color {
        Color(red: Double(r) / 255, green: Double(g) / 255, blue: Double(b) / 255)
    }

    var headerForeground: Color {
        let luma = 0.299 * Double(r) + 0.587 * Double(g) + 0.114 * Double(b)
        return luma >= 140
            ? Color(red: 17 / 255, green: 17 / 255, blue: 17 / 255)
            : Color.white
    }

    init(_ color: Color) {
        let ns = NSColor(color).usingColorSpace(.sRGB) ?? NSColor(color)
        var red: CGFloat = 0
        var green: CGFloat = 0
        var blue: CGFloat = 0
        var alpha: CGFloat = 0
        ns.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        self.init(
            r: UInt8(clamping: Int((red * 255).rounded())),
            g: UInt8(clamping: Int((green * 255).rounded())),
            b: UInt8(clamping: Int((blue * 255).rounded()))
        )
    }
}

/// Keeps every child alive (unlike LazyVGrid) so Metal scene tiles stay attached.
struct WrapFlowLayout: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(in: proposal.width ?? 176, subviews: subviews).0
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let origins = arrange(in: bounds.width, subviews: subviews).1
        for (index, subview) in subviews.enumerated() {
            let size = subview.sizeThatFits(.unspecified)
            subview.place(
                at: CGPoint(x: bounds.minX + origins[index].x, y: bounds.minY + origins[index].y),
                proposal: ProposedViewSize(size)
            )
        }
    }

    private func arrange(in width: CGFloat, subviews: Subviews) -> (CGSize, [CGPoint]) {
        let limit = max(176, width)
        var origins: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        var maxX: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > limit {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            origins.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
            maxX = max(maxX, x - spacing)
        }
        return (CGSize(width: max(limit, maxX), height: y + rowHeight), origins)
    }
}

struct MixerButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(EivizTheme.text)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(EivizTheme.button)
            .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

struct MixerTileButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 10))
            .foregroundStyle(EivizTheme.text)
            .padding(.horizontal, 3)
            .frame(minWidth: 26, minHeight: 18)
            .background(EivizTheme.button)
            .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

func parseUInt32(_ text: String) -> UInt32? {
    UInt32(text.filter(\.isNumber))
}

func formatMixerFloat(_ value: Float) -> String {
    let formatter = NumberFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.usesGroupingSeparator = false
    formatter.minimumFractionDigits = 0
    formatter.maximumFractionDigits = 4
    return formatter.string(from: NSNumber(value: value)) ?? "0"
}

private let mixerFieldLineHeight: CGFloat = 22

@MainActor
func mixerTextField(
    _ text: Binding<String>,
    placeholder: String = "",
    onSubmit: (() -> Void)? = nil
) -> some View {
    MixerTextField(placeholder: placeholder, text: text, onSubmit: onSubmit)
        .frame(height: mixerFieldLineHeight)
        .fixedSize(horizontal: false, vertical: true)
}

@MainActor
func mixerUintField(_ value: Binding<UInt32>, minimum: UInt32 = 1) -> some View {
    MixerUintField(value: value, minimum: minimum)
        .frame(height: mixerFieldLineHeight)
        .fixedSize(horizontal: false, vertical: true)
}

@MainActor
func mixerInt32Field(_ value: Binding<Int32>) -> some View {
    MixerInt32Field(value: value)
        .frame(height: mixerFieldLineHeight)
        .fixedSize(horizontal: false, vertical: true)
}

@MainActor
func mixerFloatField(_ value: Binding<Float>, onSubmit: (() -> Void)? = nil) -> some View {
    MixerFloatField(value: value, onSubmit: onSubmit)
        .frame(height: mixerFieldLineHeight)
        .fixedSize(horizontal: false, vertical: true)
}

final class MixerFieldView: NSView {
    let field = FocusableTextField(string: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.borderWidth = 1
        field.isBezeled = false
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 13)
        field.lineBreakMode = .byTruncatingTail
        field.cell?.wraps = false
        field.cell?.isScrollable = true
        field.isAutomaticTextCompletionEnabled = false
        field.translatesAutoresizingMaskIntoConstraints = false
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        field.setContentHuggingPriority(.required, for: .vertical)
        field.setContentCompressionResistancePriority(.required, for: .vertical)
        addSubview(field)
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            field.centerYAnchor.constraint(equalTo: centerYAnchor)
        ])
        setContentHuggingPriority(.required, for: .vertical)
        setContentCompressionResistancePriority(.required, for: .vertical)
        applyChrome()
    }

    func applyChrome() {
        let dark = EivizTheme.isDark
        layer?.backgroundColor = (dark ? NSColor(white: 0.16, alpha: 1) : NSColor(white: 1, alpha: 1)).cgColor
        layer?.borderColor = (dark ? NSColor(white: 0.27, alpha: 1) : NSColor(white: 0.73, alpha: 1)).cgColor
        field.textColor = dark ? NSColor(white: 238 / 255, alpha: 1) : NSColor(white: 17 / 255, alpha: 1)
    }

    required init?(coder: NSCoder) {
        nil
    }

    override var intrinsicContentSize: NSSize {
        NSSize(width: NSView.noIntrinsicMetric, height: mixerFieldLineHeight)
    }
}

final class FocusableTextField: NSTextField {
    override var acceptsFirstResponder: Bool { true }

    override func mouseDown(with event: NSEvent) {
        window?.makeKeyAndOrderFront(nil)
        window?.makeFirstResponder(self)
        super.mouseDown(with: event)
    }
}

struct MixerTextField: NSViewRepresentable {
    var placeholder: String = ""
    @Binding var text: String
    var onSubmit: (() -> Void)?

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> MixerFieldView {
        let view = MixerFieldView(frame: .zero)
        view.field.placeholderString = placeholder
        view.field.stringValue = text
        view.field.delegate = context.coordinator
        return view
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: MixerFieldView, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 80, height: mixerFieldLineHeight)
    }

    func updateNSView(_ view: MixerFieldView, context: Context) {
        context.coordinator.parent = self
        view.applyChrome()
        view.field.placeholderString = placeholder
        if view.field.currentEditor() == nil, view.field.stringValue != text {
            view.field.stringValue = text
        }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: MixerTextField

        init(_ parent: MixerTextField) {
            self.parent = parent
        }

        func controlTextDidChange(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            parent.text = field.stringValue
        }

        func controlTextDidEndEditing(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            parent.text = field.stringValue
            parent.onSubmit?()
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
            if commandSelector == #selector(NSResponder.insertNewline(_:)) {
                parent.text = (control as? NSTextField)?.stringValue ?? parent.text
                parent.onSubmit?()
                control.window?.makeFirstResponder(nil)
                return true
            }
            return false
        }
    }
}

struct MixerUintField: NSViewRepresentable {
    @Binding var value: UInt32
    var minimum: UInt32 = 1

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> MixerFieldView {
        let view = MixerFieldView(frame: .zero)
        view.field.stringValue = String(value)
        view.field.delegate = context.coordinator
        return view
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: MixerFieldView, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 80, height: mixerFieldLineHeight)
    }

    func updateNSView(_ view: MixerFieldView, context: Context) {
        context.coordinator.parent = self
        view.applyChrome()
        if view.field.currentEditor() == nil, view.field.stringValue != String(value) {
            view.field.stringValue = String(value)
        }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: MixerUintField

        init(_ parent: MixerUintField) {
            self.parent = parent
        }

        func controlTextDidChange(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            if let parsed = parseUInt32(field.stringValue), parsed >= parent.minimum {
                parent.value = parsed
            }
        }

        func controlTextDidEndEditing(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            if let parsed = parseUInt32(field.stringValue), parsed >= parent.minimum {
                parent.value = parsed
                field.stringValue = String(parsed)
            } else {
                field.stringValue = String(parent.value)
            }
        }
    }
}

struct MixerInt32Field: NSViewRepresentable {
    @Binding var value: Int32

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> MixerFieldView {
        let view = MixerFieldView(frame: .zero)
        view.field.stringValue = String(value)
        view.field.delegate = context.coordinator
        return view
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: MixerFieldView, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 80, height: mixerFieldLineHeight)
    }

    func updateNSView(_ view: MixerFieldView, context: Context) {
        context.coordinator.parent = self
        view.applyChrome()
        if view.field.currentEditor() == nil, view.field.stringValue != String(value) {
            view.field.stringValue = String(value)
        }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: MixerInt32Field

        init(_ parent: MixerInt32Field) {
            self.parent = parent
        }

        func controlTextDidChange(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            let filtered = field.stringValue.filter { $0.isNumber || $0 == "-" }
            if let parsed = Int32(filtered) {
                parent.value = parsed
            }
        }

        func controlTextDidEndEditing(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            let filtered = field.stringValue.filter { $0.isNumber || $0 == "-" }
            if let parsed = Int32(filtered) {
                parent.value = parsed
                field.stringValue = String(parsed)
            } else {
                field.stringValue = String(parent.value)
            }
        }
    }
}

struct MixerFloatField: NSViewRepresentable {
    @Binding var value: Float
    var onSubmit: (() -> Void)?

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> MixerFieldView {
        let view = MixerFieldView(frame: .zero)
        view.field.stringValue = formatMixerFloat(value)
        view.field.delegate = context.coordinator
        return view
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: MixerFieldView, context: Context) -> CGSize? {
        CGSize(width: proposal.width ?? 80, height: mixerFieldLineHeight)
    }

    func updateNSView(_ view: MixerFieldView, context: Context) {
        context.coordinator.parent = self
        view.applyChrome()
        if view.field.currentEditor() == nil {
            let shown = formatMixerFloat(value)
            if view.field.stringValue != shown {
                view.field.stringValue = shown
            }
        }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: MixerFloatField

        init(_ parent: MixerFloatField) {
            self.parent = parent
        }

        func controlTextDidChange(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            let text = field.stringValue.replacingOccurrences(of: ",", with: ".")
            if let parsed = Float(text) {
                parent.value = parsed
            }
        }

        func controlTextDidEndEditing(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            let text = field.stringValue.replacingOccurrences(of: ",", with: ".")
            if let parsed = Float(text) {
                parent.value = parsed
                field.stringValue = formatMixerFloat(parsed)
            } else {
                field.stringValue = formatMixerFloat(parent.value)
            }
            parent.onSubmit?()
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
            if commandSelector == #selector(NSResponder.insertNewline(_:)) {
                controlTextDidEndEditing(Notification(name: NSControl.textDidEndEditingNotification, object: control))
                control.window?.makeFirstResponder(nil)
                return true
            }
            return false
        }
    }
}
