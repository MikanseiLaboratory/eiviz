import AppKit
import EivizMixer
import SwiftUI

final class MetalSurfaceView: NSView {
    var presentInterval: UInt32 = 1
    var role: SurfaceRole = .unit(unitId: 1, kind: EIVIZ_OUTPUT_PROGRAM) {
        didSet {
            if role != oldValue {
                attachIfNeeded()
            }
        }
    }
    private var attached = false
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
        let handle = Int(bitPattern: Unmanaged.passUnretained(self).toOpaque())
        if detachIsMonitor {
            _ = mixer_detach_monitor(detachMonitorId)
        } else {
            _ = mixer_unit_detach_native(detachUnitId, detachKind, EIVIZ_NATIVE_APPKIT_NSVIEW, handle)
        }
    }

    func attachIfNeeded() {
        guard window != nil else { return }
        let (width, height) = pixelSize()
        guard width >= 16, height >= 16 else { return }
        let handle = Int(bitPattern: Unmanaged.passUnretained(self).toOpaque())
        let key = role.key
        if !attached || attachedKey != key {
            if attached {
                if detachIsMonitor {
                    _ = mixer_detach_monitor(detachMonitorId)
                } else {
                    _ = mixer_unit_detach_native(detachUnitId, detachKind, EIVIZ_NATIVE_APPKIT_NSVIEW, handle)
                }
            }
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
            attached = code == EIVIZ_OK
            if attached, case .monitor(let monitorId, _) = role {
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
        let width = UInt32(max(2, (bounds.width * scale).rounded()))
        let height = UInt32(max(2, (bounds.height * scale).rounded()))
        return (width, height)
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

struct MetalPreviewRepresentable: NSViewRepresentable {
    let role: SurfaceRole
    var presentInterval: UInt32 = 1
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
        if surface.role != role {
            surface.role = role
        }
        surface.attachIfNeeded()
    }
}
