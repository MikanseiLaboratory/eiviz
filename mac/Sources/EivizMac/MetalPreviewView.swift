import AppKit
import EivizMixer
import SwiftUI

final class MetalSurfaceView: NSView {
    var role: SurfaceRole = .unit(unitId: 1, kind: EIVIZ_OUTPUT_PROGRAM) {
        didSet { attachIfNeeded() }
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
        wantsLayer = true
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
                _ = mixer_set_monitor_present_interval(monitorId, 1)
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
        if width != attachedWidth || height != attachedHeight {
            switch role {
            case .unit(let unitId, let kind):
                _ = mixer_unit_resize_native(unitId, kind, EIVIZ_NATIVE_APPKIT_NSVIEW, handle, width, height)
            case .monitor(let monitorId, _):
                _ = mixer_resize_monitor(monitorId, width, height)
            }
            attachedWidth = width
            attachedHeight = height
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

struct MetalPreviewRepresentable: NSViewRepresentable {
    let role: SurfaceRole

    func makeNSView(context: Context) -> NSView {
        // Host must not be layer-backed: SwiftUI paints layer-backed representable
        // views and covers the child CAMetalLayer (scene tiles stay black).
        let host = NSView(frame: .zero)
        host.wantsLayer = false
        let surface = MetalSurfaceView(frame: .zero)
        surface.autoresizingMask = [.width, .height]
        surface.role = role
        host.addSubview(surface)
        return host
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        nsView.wantsLayer = false
        guard let surface = nsView.subviews.first as? MetalSurfaceView else { return }
        surface.frame = nsView.bounds
        surface.role = role
        surface.attachIfNeeded()
    }
}
