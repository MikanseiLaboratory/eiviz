using System.Globalization;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace Eiviz.Host.Media;

internal static class LabelRaster
{
    internal const int Width = 256;
    internal const int Height = 32;

    internal static byte[] Bgra(string text)
    {
        var visual = new DrawingVisual();
        using (var dc = visual.RenderOpen())
        {
            dc.DrawRectangle(new SolidColorBrush(Color.FromArgb(200, 0, 0, 0)), null, new Rect(0, 0, Width, Height));
            var formatted = new FormattedText(
                string.IsNullOrWhiteSpace(text) ? " " : text,
                CultureInfo.CurrentUICulture,
                FlowDirection.LeftToRight,
                new Typeface("Segoe UI"),
                16,
                Brushes.White,
                1.0);
            formatted.MaxTextWidth = Width - 12;
            formatted.MaxTextHeight = Height - 4;
            formatted.Trimming = TextTrimming.CharacterEllipsis;
            dc.DrawText(formatted, new Point(6, Math.Max(0, (Height - formatted.Height) / 2)));
        }
        var bitmap = new RenderTargetBitmap(Width, Height, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(visual);
        var pixels = new byte[Width * Height * 4];
        bitmap.CopyPixels(pixels, Width * 4, 0);
        return pixels;
    }
}
