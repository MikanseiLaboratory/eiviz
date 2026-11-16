import AppKit
import EivizMixer
import SwiftUI

final class MetalSurfaceView: NSView {
    var presentInterval: UInt32 = 1
    var surfaceEpoch: UInt64 = 0
    /// The largest surface the mixer should render. The master format is the
    /// picture's own size: a bigger layer only enlarges the present draw and the
    /// backing store while CoreAnimation scales the layer for display anyway.
    var maxSurfaceSize = CGSize(width: 1920, height: 1080)
    var role: SurfaceRole = .unit(unitId: 1, kind: EIVIZ_OUTPUT_PROGRAM) {
        didSet {
            if role != oldValue {
                attachIfNeeded()
            }
        }
    }
    private var attached = false
    private var budgeted = false
    private var attachedWidth: UInt32 = 0
    private var attachedHeight: UInt32 = 0
    private var attachedKey: String = ""
    nonisolated(unsafe) private var detachUnitId: UInt64 = 1
    nonisolated(unsafe) private var detachKind: UInt32 = 0
    nonisolated(unsafe) private var detachMonitorId: UInt64 = 0
    nonisolated(unsafe) private var detachIsMonitor = false

    override var isOpaque: Bool { true }
    override var wantsUpdateLayer: Bool { false }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        configureLayer()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        configureLayer()
    }

    private func configureLayer() {
        // Do not pre-create a generic CALayer. wgpu attaches CAMetalLayer to this NSView.
        layerContentsRedrawPolicy = .never
        clipsToBounds = true
        autoresizingMask = [.width, .height]
    }

    override func layout() {
        super.layout()
        attachIfNeeded()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        attachIfNeeded()
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        attachIfNeeded()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        attachIfNeeded()
    }

    override func mouseUp(with event: NSEvent) {
        (superview as? PreviewHostView)?.mouseUp(with: event)
    }

    deinit {
        let wasAttached = attached
        let wasBudgeted = budgeted
        if wasAttached || wasBudgeted {
            _ = detachMixerOnly()
        }
        if wasBudgeted {
            FlipBudget.end(self)
        }
    }

    func dismantle() {
        let retained = Unmanaged.passRetained(self)
        defer { retained.release() }
        releaseNative()
    }

    func releaseNative() {
        guard attached || budgeted else { return }
        let code = detachMixerOnly()
        guard code == EIVIZ_OK else { return }
        attached = false
        attachedKey = ""
        if budgeted {
            FlipBudget.end(self)
            budgeted = false
        }
    }

    nonisolated private func detachMixerOnly() -> Int32 {
        let handle = Int(bitPattern: Unmanaged.passUnretained(self).toOpaque())
        if detachIsMonitor {
            return mixer_detach_monitor(detachMonitorId)
        }
        return mixer_unit_detach_native(detachUnitId, detachKind, EIVIZ_NATIVE_APPKIT_NSVIEW, handle)
    }

    func attachIfNeeded() {
        guard window != nil else { return }
        if case .monitor(let monitorId, let sourceId) = role, monitorId == 0 || sourceId == 0 {
            return
        }
        let (width, height) = pixelSize()
        guard width >= 16, height >= 16 else { return }
        let handle = Int(bitPattern: Unmanaged.passUnretained(self).toOpaque())
        let key = "\(role.key)#\(surfaceEpoch)"
        if !attached || attachedKey != key {
            if attached || budgeted {
                releaseNative()
            }
            // A refused begin leaves this surface unattached: the deck shows the
            // "limit reached" overlay for a tile it could not hand a monitor to, and
            // the refusal is counted so the deck can say the pictures are missing.
            guard FlipBudget.tryBegin(self) else {
                FlipBudget.noteRefusal()
                return
            }
            budgeted = true
            let code: Int32
            switch role {
            case .unit(let unitId, let kind):
                code = mixer_unit_attach_native(unitId, kind, EIVIZ_NATIVE_APPKIT_NSVIEW, handle, width, height)
            case .monitor(let monitorId, let sourceId):
                code = mixer_attach_monitor_native(monitorId, sourceId, EIVIZ_NATIVE_APPKIT_NSVIEW, handle, width, height)
                if code == EIVIZ_OK {
                    _ = mixer_monitor_set_source(monitorId, sourceId)
                }
            }
            if code != EIVIZ_OK {
                FlipBudget.cancel(self)
                budgeted = false
                return
            }
            attached = true
            if case .monitor(let monitorId, _) = role {
                _ = mixer_set_monitor_present_interval(monitorId, max(1, min(8, presentInterval)))
            }
            attachedWidth = width
            attachedHeight = height
            attachedKey = key
            switch role {
            case .unit(let unitId, let kind):
                detachIsMonitor = false
                detachUnitId = unitId
                detachKind = kind
            case .monitor(let monitorId, _):
                detachIsMonitor = true
                detachMonitorId = monitorId
            }
            return
        }
        // Meter ticks reflow SwiftUI by a point or two; ignore that so Metal
        // is not reconfigured every 50 ms (Outdated / Lost looks frozen).
        if abs(Int(width) - Int(attachedWidth)) > 2 || abs(Int(height) - Int(attachedHeight)) > 2 {
            switch role {
            case .unit(let unitId, let kind):
                _ = mixer_unit_resize_native(unitId, kind, EIVIZ_NATIVE_APPKIT_NSVIEW, handle, width, height)
            case .monitor(let monitorId, _):
                _ = mixer_resize_monitor(monitorId, width, height)
            }
            attachedWidth = width
            attachedHeight = height
        }
        if case .monitor(let monitorId, _) = role {
            _ = mixer_set_monitor_present_interval(monitorId, max(1, min(8, presentInterval)))
        }
    }

    private func pixelSize() -> (UInt32, UInt32) {
        let scale = window?.backingScaleFactor ?? 2
        var width = Double(max(2, (bounds.width * scale).rounded()))
        var height = Double(max(2, (bounds.height * scale).rounded()))
        // The flip draws the master-format texture into this surface: past the
        // picture's own size the draw and the backing store only get bigger while
        // CoreAnimation would scale the presented layer for display anyway. Measured
        // on the reference machine at fullscreen the raw present cost grew 2-4x with
        // the layer size, which is what pushed the render loop past its budget.
        let maxW = max(2, maxSurfaceSize.width)
        let maxH = max(2, maxSurfaceSize.height)
        if width > maxW || height > maxH {
            let shrink = max(width / maxW, height / maxH)
            width = (width / shrink).rounded(.down)
            height = (height / shrink).rounded(.down)
        }
        return (UInt32(width), UInt32(height))
    }
}

enum SurfaceRole: Equatable {
    case unit(unitId: UInt64, kind: UInt32)
    case monitor(monitorId: UInt64, sourceId: UInt64)

    var key: String {
        switch self {
        case .unit(let unitId, let kind): return "u-\(unitId)-\(kind)"
        case .monitor(let monitorId, let sourceId): return "m-\(monitorId)-\(sourceId)"
        }
    }
}

final class PreviewHostView: NSView {
    var onClick: (() -> Void)?
    var onDoubleClick: (() -> Void)?

    override var isOpaque: Bool { true }

    // SwiftUI will try to layer-back a representable and then snapshot it,
    // covering the child CAMetalLayer. Refuse that so Metal can keep presenting.
    override var wantsLayer: Bool {
        get { false }
        set {}
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.black.setFill()
        bounds.fill()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func hitTest(_ point: NSPoint) -> NSView? {
        bounds.contains(point) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeKeyAndOrderFront(nil)
    }

    override func mouseUp(with event: NSEvent) {
        if event.clickCount >= 2, onDoubleClick != nil {
            onDoubleClick?()
        } else {
            onClick?()
        }
    }
}

private struct MixerSurfaceEpochKey: EnvironmentKey {
    static let defaultValue: UInt64 = 0
}

extension EnvironmentValues {
    var mixerSurfaceEpoch: UInt64 {
        get { self[MixerSurfaceEpochKey.self] }
        set { self[MixerSurfaceEpochKey.self] = newValue }
    }
}

struct MetalPreviewRepresentable: NSViewRepresentable {
    @Environment(\.mixerSurfaceEpoch) private var surfaceEpoch
    let role: SurfaceRole
    var presentInterval: UInt32 = 1
    /// Read once per makeNSView from the mixer's own session, so a master format
    /// change (resolutions are editable) reaches new surfaces without plumbing. The
    /// environment object lives on the class; the struct reads a static per-app copy.
    var maxSurfaceSize: CGSize = EivizSurfaceCap.shared.size
    var onClick: (() -> Void)? = nil
    var onDoubleClick: (() -> Void)? = nil

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    final class Coordinator {
        var onClick: (() -> Void)?
        var onDoubleClick: (() -> Void)?
    }

    func makeNSView(context: Context) -> PreviewHostView {
        context.coordinator.onClick = onClick
        context.coordinator.onDoubleClick = onDoubleClick
        let host = PreviewHostView(frame: .zero)
        host.onClick = { context.coordinator.onClick?() }
        host.onDoubleClick = { context.coordinator.onDoubleClick?() }
        let surface = MetalSurfaceView(frame: .zero)
        surface.autoresizingMask = [.width, .height]
        surface.presentInterval = presentInterval
        surface.maxSurfaceSize = maxSurfaceSize
        surface.role = role
        host.addSubview(surface)
        return host
    }

    func updateNSView(_ nsView: PreviewHostView, context: Context) {
        context.coordinator.onClick = onClick
        context.coordinator.onDoubleClick = onDoubleClick
        nsView.onClick = { context.coordinator.onClick?() }
        nsView.onDoubleClick = { context.coordinator.onDoubleClick?() }
        guard let surface = nsView.subviews.first as? MetalSurfaceView else { return }
        if surface.frame != nsView.bounds {
            surface.frame = nsView.bounds
        }
        if surface.presentInterval != presentInterval {
            surface.presentInterval = presentInterval
        }
        if surface.maxSurfaceSize != maxSurfaceSize {
            surface.maxSurfaceSize = maxSurfaceSize
        }
        if surface.surfaceEpoch != surfaceEpoch {
            surface.surfaceEpoch = surfaceEpoch
        }
        if surface.role != role {
            surface.role = role
        }
        surface.attachIfNeeded()
    }

    func dismantleNSView(_ nsView: PreviewHostView, coordinator: Coordinator) {
        _ = coordinator
        guard let surface = nsView.subviews.first as? MetalSurfaceView else { return }
        surface.dismantle()
    }
}
