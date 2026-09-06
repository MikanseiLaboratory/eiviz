import AppKit

func makeInputPreviewContent(sourceId: UInt64, frame: NSRect) -> NSView {
    let content = ThumbHostView(frame: frame)
    content.autoresizingMask = [.width, .height]
    let thumb = ThumbImageView(frame: content.bounds)
    thumb.autoresizingMask = [.width, .height]
    thumb.autoSizeFromBounds = true
    let scale = NSScreen.main?.backingScaleFactor ?? 2
    let width = UInt32(min(960, max(2, (frame.width * scale).rounded())))
    let height = UInt32(min(540, max(2, (frame.height * scale).rounded())))
    thumb.bind(sourceId: sourceId, width: width, height: height, interval: 1)
    thumb.setWanted(true)
    content.addSubview(thumb)
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
