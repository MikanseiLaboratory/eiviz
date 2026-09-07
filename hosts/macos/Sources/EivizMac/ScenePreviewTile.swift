import AppKit
import SwiftUI

struct ScenePreviewTile: View, @MainActor Equatable {
    let sceneId: UInt64
    let gpuId: UInt64
    let name: String
    let number: Int
    let preview: Bool
    let program: Bool
    let selected: Bool
    let previewCollapsed: Bool
    let interval: UInt32
    let loopOn: Bool
    let playing: Bool
    let muted: Bool
    let hasVideo: Bool
    let previewColor: Color
    let programColor: Color
    let inactiveColor: Color
    let showThumb: Bool
    let onPreview: () -> Void
    let onCut: () -> Void
    let onLoop: () -> Void
    let onPlay: () -> Void
    let onAudio: () -> Void
    let onOpenPreview: () -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void
    let onCollapse: () -> Void
    let onSnapshot: () -> Void

    @State private var appeared = false

    static func == (lhs: ScenePreviewTile, rhs: ScenePreviewTile) -> Bool {
        lhs.sceneId == rhs.sceneId
            && lhs.gpuId == rhs.gpuId
            && lhs.name == rhs.name
            && lhs.number == rhs.number
            && lhs.preview == rhs.preview
            && lhs.program == rhs.program
            && lhs.selected == rhs.selected
            && lhs.previewCollapsed == rhs.previewCollapsed
            && lhs.interval == rhs.interval
            && lhs.loopOn == rhs.loopOn
            && lhs.playing == rhs.playing
            && lhs.muted == rhs.muted
            && lhs.hasVideo == rhs.hasVideo
            && lhs.showThumb == rhs.showThumb
    }

    private var wanted: Bool { !previewCollapsed && (preview || program || selected || appeared) }

    var body: some View {
        Group {
            if previewCollapsed {
                collapsedBody
            } else {
                expandedBody
            }
        }
        .background(Rectangle().stroke(
            program ? programColor : preview ? previewColor : inactiveColor,
            lineWidth: 2
        ))
        .background(TileRightClickCatcher(action: onCollapse))
        .onTapGesture(count: 2, perform: onEdit)
        .onAppear { appeared = true }
        .onDisappear { appeared = false }
    }

    private var expandedBody: some View {
        VStack(spacing: 0) {
            titleBar
            if showThumb {
                ThumbRepresentable(
                    sourceId: gpuId,
                    width: 176,
                    height: 90,
                    interval: interval,
                    wanted: wanted,
                    onClick: onPreview
                )
                .frame(width: 176, height: 90)
            } else {
                Color.black.frame(width: 176, height: 90)
                    .contentShape(Rectangle())
                    .onTapGesture(perform: onPreview)
            }
            HStack(spacing: 1) {
                chip("CUT", action: onCut)
                chip("Loop", action: onLoop)
                    .opacity(hasVideo ? (loopOn ? 1 : 0.55) : 0.35)
                    .disabled(!hasVideo)
                chip(playing ? "❚❚" : "▶", action: onPlay)
                    .disabled(!hasVideo)
                chip("Aud", action: onAudio)
                    .opacity(muted ? 0.45 : 1)
                if showThumb {
                    chip("Prev", action: onOpenPreview)
                }
                TileSetButton(title: "Set", onLeft: onEdit, onRight: onSnapshot)
            }
            .padding(2)
        }
        .frame(width: 176)
    }

    private var collapsedBody: some View {
        VStack(spacing: 4) {
            Button("X", action: onDelete)
                .buttonStyle(MixerTileButtonStyle())
            Text("\(number)")
                .font(.system(size: 11, weight: .bold))
            Text(name)
                .font(.system(size: 12))
                .lineLimit(1)
                .rotationEffect(.degrees(-90))
                .frame(maxHeight: .infinity)
        }
        .padding(.vertical, 4)
        .frame(width: 40, height: 140)
        .background(program ? programColor.opacity(0.28) : preview ? previewColor.opacity(0.22) : EivizTheme.chrome)
        .contentShape(Rectangle())
        .onTapGesture(perform: onPreview)
    }

    private var titleBar: some View {
        HStack(spacing: 6) {
            Text("\(number)")
                .font(.system(size: 11, weight: .bold))
            Text(name)
                .font(.system(size: 12))
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("X", action: onDelete)
                .buttonStyle(MixerTileButtonStyle())
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(EivizTheme.chrome)
        .contentShape(Rectangle())
        .onTapGesture(perform: onPreview)
    }

    private func chip(_ title: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .buttonStyle(MixerTileButtonStyle())
    }
}

struct TileSetButton: NSViewRepresentable {
    let title: String
    let onLeft: () -> Void
    let onRight: () -> Void

    func makeNSView(context: Context) -> TileSetNSButton {
        let button = TileSetNSButton()
        button.title = title
        button.bezelStyle = .smallSquare
        button.isBordered = true
        button.font = .systemFont(ofSize: 10)
        button.onLeft = onLeft
        button.onRight = onRight
        return button
    }

    func updateNSView(_ nsView: TileSetNSButton, context: Context) {
        nsView.title = title
        nsView.onLeft = onLeft
        nsView.onRight = onRight
    }
}

final class TileSetNSButton: NSButton {
    var onLeft: (() -> Void)?
    var onRight: (() -> Void)?

    override func mouseDown(with event: NSEvent) {
        onLeft?()
    }

    override func rightMouseDown(with event: NSEvent) {
        onRight?()
    }
}

fileprivate struct TileRightClickCatcher: NSViewRepresentable {
    let action: () -> Void

    func makeNSView(context: Context) -> TileRightClickNSView {
        let view = TileRightClickNSView()
        view.action = action
        return view
    }

    func updateNSView(_ nsView: TileRightClickNSView, context: Context) {
        nsView.action = action
    }
}

fileprivate final class TileRightClickNSView: NSView {
    var action: (() -> Void)?
    private var monitor: Any?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let monitor {
            NSEvent.removeMonitor(monitor)
            self.monitor = nil
        }
        guard window != nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .rightMouseDown) { [weak self] event in
            guard let self, let window = self.window, event.window == window else { return event }
            let loc = self.convert(event.locationInWindow, from: nil)
            guard self.bounds.contains(loc) else { return event }
            if self.window?.contentView?.hitTest(event.locationInWindow) is TileSetNSButton {
                return event
            }
            self.action?()
            return nil
        }
    }

    override func removeFromSuperview() {
        if let monitor {
            NSEvent.removeMonitor(monitor)
            self.monitor = nil
        }
        super.removeFromSuperview()
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        nil
    }
}
