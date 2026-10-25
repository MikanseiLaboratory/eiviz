namespace Eiviz.Host.Dialogs;

/// <summary>
/// Packed UYVY to the BGRA the Windows imaging stack wants.
///
/// The mixer hands the jog proxy out as packed UYVY — the layout it already has on the GPU —
/// and a WPF <c>WriteableBitmap</c> takes BGRA. The conversion is the same for every tile of
/// every angle, so it lives here rather than in the window: it has no state, it is the part
/// most likely to be wrong (a swapped U and V gives a plausible but wrong picture), and it
/// can be reasoned about without a window open.
/// </summary>
internal static class ReplayUyvy
{
    /// <summary>Bytes per pixel of the destination.</summary>
    public const int BgraBytesPerPixel = 4;

    /// <summary>Bytes two pixels of UYVY take: one U, two Y, one V.</summary>
    public const int UyvyBytesPerPair = 4;

    /// <summary>
    /// Converts one UYVY picture into a BGRA surface.
    ///
    /// A texel carries two pixels that share a chroma pair, so the loop walks pairs. The
    /// last pixel of an odd-width picture has no partner and still gets written.
    /// </summary>
    /// <param name="source">Packed UYVY, `stride * height` bytes or more.</param>
    /// <param name="width">Picture width in pixels.</param>
    /// <param name="height">Picture height in pixels.</param>
    /// <param name="destination">BGRA destination, at least `stride * height` bytes.</param>
    /// <param name="stride">Destination bytes per row.</param>
    /// <param name="sourceStride">Source bytes per row; a store may pad it past `width * 2`.</param>
    /// <returns>False when the buffers are too small for the picture, so a caller does not
    /// read past either end.</returns>
    public static unsafe bool ToBgra(
        ReadOnlySpan<byte> source,
        int width,
        int height,
        Span<byte> destination,
        int stride,
        int sourceStride)
    {
        if (width <= 0 || height <= 0)
            return false;
        // Two pixels share one texel, so a row of `width` pixels takes `(width + 1) / 2`
        // texels of `UyvyBytesPerPair` bytes each. Rounding up keeps an odd width safe.
        var texelsPerRow = ((long)width + 1) / 2;
        var rowBytes = texelsPerRow * UyvyBytesPerPair;
        if (sourceStride < rowBytes)
            return false;
        if (source.Length < (long)sourceStride * height)
            return false;
        if (stride < 0 || stride < (long)width * BgraBytesPerPixel)
            return false;
        if (destination.Length < (long)stride * height)
            return false;

        fixed (byte* src = source)
        fixed (byte* dst = destination)
        {
            for (var y = 0; y < height; y++)
            {
                for (var x = 0; x < width; x += 2)
                {
                    var offset = y * sourceStride + x * 2;
                    var u = src[offset] - 128;
                    var y0 = src[offset + 1];
                    var v = src[offset + 2] - 128;
                    var y1 = src[offset + 3];
                    Write(dst, stride, x, y, y0, u, v);
                    if (x + 1 < width)
                        Write(dst, stride, x + 1, y, y1, u, v);
                }
            }
        }
        return true;
    }

    /// <summary>One pixel, from its luma and the pair's chroma.</summary>
    private static unsafe void Write(byte* dst, int stride, int x, int y, int luma, int u, int v)
    {
        // BT.601, the same matrix the mixer's own conversion uses.
        var red = luma + (int)(1.402 * v);
        var green = luma - (int)(0.344136 * u + 0.714136 * v);
        var blue = luma + (int)(1.772 * u);
        var pixel = dst + y * stride + x * BgraBytesPerPixel;
        pixel[0] = (byte)Math.Clamp(blue, 0, 255);
        pixel[1] = (byte)Math.Clamp(green, 0, 255);
        pixel[2] = (byte)Math.Clamp(red, 0, 255);
        // Opaque: WPF's composition wants a defined alpha, and a zero here is invisible.
        pixel[3] = 255;
    }
}
