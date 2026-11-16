import SwiftUI

/// The largest flip surface the host asks the mixer for. Follows the session's
/// master format: the flip draws the composed frame at that size, and anything
/// larger only costs present time while CoreAnimation scales the layer anyway.
@MainActor
final class EivizSurfaceCap {
    static let shared = EivizSurfaceCap()
    private(set) var size = CGSize(width: 1920, height: 1080)

    /// Called wherever the session (re)loads or the settings change.
    func update(width: UInt32, height: UInt32) {
        if width > 0, height > 0, size != CGSize(width: CGFloat(width), height: CGFloat(height)) {
            size = CGSize(width: CGFloat(width), height: CGFloat(height))
        }
    }
}
