import AppKit
import EivizMixer
import SwiftUI

final class MetalSurfaceView: NSView {
    var unitId: UInt64 = MixerSession.unitId
    var kind: UInt32 = EIVIZ_OUTPUT_PROGRAM
    private var attached = false
    private var attachedWidth: UInt32 = 0
    private var attachedHeight: UInt32 = 0

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = false
        autoresizingMask = [.width, .height]
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        wantsLayer = false
        autoresizingMask = [.width, .height]
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
        detach()
    }

    func attachIfNeeded() {
        guard window != nil else { return }
        let (width, height) = pixelSize()
        let handle = nativeHandle()
        if !attached {
            let code = mixer_unit_attach_native(
                unitId,
                kind,
                EIVIZ_NATIVE_APPKIT_NSVIEW,
                handle,
                width,
                height
            )
            attached = code == EIVIZ_OK
            attachedWidth = width
            attachedHeight = height
            return
        }
        if width != attachedWidth || height != attachedHeight {
            _ = mixer_unit_resize_native(
                unitId,
                kind,
                EIVIZ_NATIVE_APPKIT_NSVIEW,
                handle,
                width,
                height
            )
            attachedWidth = width
            attachedHeight = height
        }
    }

    private func detach() {
        guard attached else { return }
        _ = mixer_unit_detach_native(unitId, kind, EIVIZ_NATIVE_APPKIT_NSVIEW, nativeHandle())
        attached = false
    }

    private func nativeHandle() -> Int {
        Int(bitPattern: Unmanaged.passUnretained(self).toOpaque())
    }

    private func pixelSize() -> (UInt32, UInt32) {
        let scale = window?.backingScaleFactor ?? 2
        let width = UInt32(max(2, (bounds.width * scale).rounded()))
        let height = UInt32(max(2, (bounds.height * scale).rounded()))
        return (width, height)
    }
}

struct MetalPreviewRepresentable: NSViewRepresentable {
    let kind: UInt32

    func makeNSView(context: Context) -> MetalSurfaceView {
        let view = MetalSurfaceView(frame: .zero)
        view.kind = kind
        return view
    }

    func updateNSView(_ nsView: MetalSurfaceView, context: Context) {
        nsView.kind = kind
        nsView.attachIfNeeded()
    }
}
