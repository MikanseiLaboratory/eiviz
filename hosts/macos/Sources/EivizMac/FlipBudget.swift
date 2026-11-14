import AppKit
import Foundation

/// Real-time preview surfaces used to share a DirectX flip-model slot budget.
/// Metal does not have that limit, so the budget only kept track of the count.
/// Windows sessions still carry `flipSwapchainLimit`; macOS ignores it.
enum FlipBudget {
    static func configure(_ limit: UInt32) {
        _ = limit
    }

    static func reset() {}

    static func tryOpen(_ surfaces: Int) -> Bool {
        _ = surfaces
        return true
    }

    static func tryBegin(_ host: MetalSurfaceView) -> Bool {
        _ = host
        return true
    }

    static func cancel(_ host: MetalSurfaceView) {
        _ = host
    }

    static func end(_ host: MetalSurfaceView) {
        _ = host
    }

    static func observeLost(_ total: UInt64) {
        _ = total
    }
}
