import AppKit
import EivizMixer
import SwiftUI

struct InputPreviewView: View {
    let monitorId: UInt64
    let sourceId: UInt64
    let ratioWidth: UInt32
    let ratioHeight: UInt32

    var body: some View {
        let ratio = CGFloat(max(1, ratioWidth)) / CGFloat(max(1, ratioHeight))
        MetalPreviewRepresentable(role: .monitor(monitorId: monitorId, sourceId: sourceId))
            .background(Color.black)
            .aspectRatio(ratio, contentMode: .fit)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(white: 17 / 255))
    }
}

final class InputPreviewHostWindow: NSWindow {
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
