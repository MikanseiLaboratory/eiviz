import AppKit
import EivizMixer
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
    let onSnapshot: (() -> Void)?

    @State private var appeared = false
    @State private var animOpen = false

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
        HStack(spacing: 0) {
            tile
            if animOpen {
                SceneAnimPanel(sceneId: sceneId)
                    .frame(width: 128, height: 140)
            }
        }
    }

    private var tile: some View {
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

    private var animToggle: some View {
        Button(animOpen ? "◂" : "▸") { animOpen.toggle() }
            .buttonStyle(MixerTileButtonStyle())
            .help(L10n.t("anim.panel"))
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
                stateChip("Loop", on: hasVideo && loopOn, action: onLoop)
                    .disabled(!hasVideo)
                    .opacity(hasVideo ? 1 : 0.35)
                chip(playing ? "❚❚" : "▶", action: onPlay)
                    .disabled(!hasVideo)
                stateChip("Aud", on: !muted, action: onAudio)
                if showThumb {
                    chip("Prev", action: onOpenPreview)
                }
                if let onSnapshot {
                    TileSetButton(title: "Set", onLeft: onEdit, onRight: onSnapshot)
                } else {
                    chip("Set", action: onEdit)
                }
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
            animToggle
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
            animToggle
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

    private func stateChip(_ title: String, on: Bool, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .buttonStyle(OnOffButtonStyle(on: on, compact: true))
    }
}

/// Go To / Play buttons shown beside a scene tile.
struct SceneAnimPanel: View {
    @EnvironmentObject private var mixer: MixerController
    let sceneId: UInt64
    @State private var live = SceneAnimLive.idle

    private let tally = Timer.publish(every: 0.2, on: .main, in: .common).autoconnect()
    private static let liveFill = Color(red: 0x1E / 255, green: 0x6B / 255, blue: 0x3A / 255)

    private var scene: SceneEntry? {
        mixer.session.scenes.first { $0.id == sceneId }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                if let scene {
                    content(scene)
                }
            }
            .padding(4)
        }
        .background(EivizTheme.chrome)
        .overlay(Rectangle().stroke(EivizTheme.stroke, lineWidth: 1))
        .onReceive(tally) { _ in
            if let scene {
                live = mixer.sceneAnimLive(scene)
            }
        }
    }

    @ViewBuilder
    private func content(_ scene: SceneEntry) -> some View {
        if scene.states.isEmpty {
            Text(L10n.t("anim.empty"))
                .font(.system(size: 10))
                .foregroundStyle(EivizTheme.dim)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            heading(L10n.t("anim.states"))
            ForEach(scene.states) { state in
                button(label(state.name, state.id), on: live.litState == state.id) {
                    mixer.sceneGo(to: state.id, scene: scene)
                }
            }
            button(L10n.t("anim.saved"), on: live.litState == 0, dim: true) {
                mixer.sceneGo(to: 0, scene: scene)
            }
            .help(L10n.t("anim.savedHelp"))
            if !scene.sequences.isEmpty {
                heading(L10n.t("anim.sequences"))
                ForEach(scene.sequences) { sequence in
                    HStack(spacing: 2) {
                        button("▶ " + label(sequence.name, sequence.id), on: live.sequenceId == sequence.id) {
                            mixer.sceneSequence(sequence.id, op: EIVIZ_SCENE_SEQ_PLAY, scene: scene)
                        }
                        Button("■") {
                            mixer.sceneSequence(sequence.id, op: EIVIZ_SCENE_SEQ_STOP, scene: scene)
                        }
                        .buttonStyle(MixerTileButtonStyle())
                        .frame(width: 22)
                        .help(L10n.t("anim.stop"))
                    }
                }
            }
        }
    }

    private func heading(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10))
            .foregroundStyle(EivizTheme.dim)
            .padding(.vertical, 2)
    }

    private func button(_ title: String, on: Bool, dim: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 11))
                .lineLimit(1)
                .truncationMode(.tail)
                .foregroundStyle(dim ? EivizTheme.dim : EivizTheme.text)
                .padding(.horizontal, 4)
                .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
                .background(on ? Self.liveFill : EivizTheme.list)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(title)
    }

    private func label(_ name: String, _ id: UInt64) -> String {
        name.trimmingCharacters(in: .whitespaces).isEmpty ? "\(id)" : name
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
