import Foundation

/// Packed UYVY to the RGBA an `NSBitmapImageRep` takes.
///
/// The mixer hands the jog proxy out as packed UYVY — the layout it already has on the GPU —
/// and a bitmap rep with `deviceRGB` takes RGBA, not BGRA. The conversion is the same for
/// every tile of every angle, so it lives here rather than in the window: it has no state,
/// it is the part most likely to be wrong (a swapped U and V gives a plausible but wrong
/// picture, and the channel order has to match the colour space exactly), and it can be
/// reasoned about without a window open.
enum ReplayUyvy {
    /// Bytes per pixel of the destination.
    static let bytesPerPixel = 4

    /// Converts one UYVY picture into an RGBA surface.
    ///
    /// A texel carries two pixels that share a chroma pair, so the loop walks pairs. A
    /// picture with an odd width cannot be the last pair's partner: its final column keeps
    /// the opaque white the buffer starts as. The proxy is always a decoder's frame, so an
    /// odd width would mean the store handed back something that is not a picture, and an
    /// obvious blank column says that better than a guessed colour would.
    ///
    /// - Parameters:
    ///   - src: packed UYVY, `stride * height` bytes or more.
    ///   - width: picture width in pixels.
    ///   - height: picture height in pixels.
    ///   - stride: source bytes per row; a store may pad it past `width * 2`.
    /// - Returns: RGBA bytes, `width * height * 4`, or nil when the source cannot hold the
    ///   picture it claims.
    static func toRgba(_ src: [UInt8], width: Int, height: Int, stride: Int) -> [UInt8]? {
        // Two pixels share one texel, so a row of `width` pixels takes `(width + 1) / 2`
        // texels of four bytes each. Rounding up keeps an odd width safe.
        let rowBytes = ((width + 1) / 2) * 4
        guard width > 0, height > 0, stride >= rowBytes, src.count >= stride * height
        else { return nil }
        var dst = [UInt8](repeating: 255, count: width * height * bytesPerPixel)
        for y in 0..<height {
            var x = 0
            while x + 1 < width {
                let i = y * stride + x * 2
                guard i + 3 < src.count else { break }
                let u = Int(src[i]) - 128
                let y0 = Int(src[i + 1])
                let v = Int(src[i + 2]) - 128
                let y1 = Int(src[i + 3])
                write(&dst, width: width, x: x, y: y, luma: y0, u: u, v: v)
                write(&dst, width: width, x: x + 1, y: y, luma: y1, u: u, v: v)
                x += 2
            }
        }
        return dst
    }

    /// One pixel, from its luma and the pair's chroma. BT.601, matching the mixer.
    private static func write(
        _ dst: inout [UInt8], width: Int, x: Int, y: Int, luma: Int, u: Int, v: Int
    ) {
        let r = min(255, max(0, luma + (1402 * v) / 1000))
        let g = min(255, max(0, luma - (344 * u + 714 * v) / 1000))
        let b = min(255, max(0, luma + (1772 * u) / 1000))
        let p = (y * width + x) * bytesPerPixel
        guard p + 3 < dst.count else { return }
        // RGBA: the rep's colour space is deviceRGB, so red first.
        dst[p] = UInt8(r)
        dst[p + 1] = UInt8(g)
        dst[p + 2] = UInt8(b)
        dst[p + 3] = 255
    }
}
