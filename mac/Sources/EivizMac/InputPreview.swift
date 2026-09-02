import AppKit

func makeInputPreviewContent(monitorId: UInt64, sourceId: UInt64, frame: NSRect) -> NSView {
    let content = PreviewHostView(frame: frame)
    content.autoresizingMask = [.width, .height]
    let surface = MetalSurfaceView(frame: content.bounds)
    surface.autoresizingMask = [.width, .height]
    surface.presentInterval = 1
    surface.role = .monitor(monitorId: monitorId, sourceId: sourceId)
    content.addSubview(surface)
    return content
}

final class InputPreviewHostWindow: NSWindow {
    var contentAspect: CGFloat = 16.0 / 9.0

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 103 {
            toggleFullScreen(nil)
            return
        }
        if event.keyCode == 53, styleMask.contains(.fullScreen) {
            toggleFullScreen(nil)
            return
        }
        super.keyDown(with: event)
    }
}
