using System.Windows.Controls;
using System.Windows.Media;
using Eiviz.Host.Interop;

namespace Eiviz.Host;

internal static class BusTheme
{
    public static Color Preview(SessionSettings settings) => ToMedia(settings.PreviewColor ?? RgbColor.PreviewDefault);

    public static Color Program(SessionSettings settings) => ToMedia(settings.ProgramColor ?? RgbColor.ProgramDefault);

    public static Color Inactive(SessionSettings settings) => ToMedia(settings.InactiveColor ?? RgbColor.InactiveDefault);

    public static SolidColorBrush PreviewBrush(SessionSettings settings) => new(Preview(settings));

    public static SolidColorBrush ProgramBrush(SessionSettings settings) => new(Program(settings));

    public static SolidColorBrush InactiveBrush(SessionSettings settings) => new(Inactive(settings));

    public static SolidColorBrush ContrastText(Color background) =>
        0.299 * background.R + 0.587 * background.G + 0.114 * background.B >= 140
            ? new SolidColorBrush(Color.FromRgb(0x11, 0x11, 0x11))
            : Brushes.White;

    public static void Apply(
        SessionSettings settings,
        Border frame,
        Border header,
        TextBlock title,
        bool preview)
    {
        var color = preview ? Preview(settings) : Program(settings);
        var brush = new SolidColorBrush(color);
        frame.BorderBrush = brush;
        header.Background = brush;
        title.Foreground = ContrastText(color);
    }

    public static Color ToMedia(RgbColor color) => Color.FromRgb(color.R, color.G, color.B);

    public static SolidColorBrush Brush(RgbColor color) => new(ToMedia(color));

    public static void PushMixer(SessionSettings settings)
    {
        var preview = settings.PreviewColor ?? RgbColor.PreviewDefault;
        var program = settings.ProgramColor ?? RgbColor.ProgramDefault;
        var inactive = settings.InactiveColor ?? RgbColor.InactiveDefault;
        MixerNative.ThrowIfFailed(
            MixerNative.SetBusColors(
                preview.R, preview.G, preview.B,
                program.R, program.G, program.B,
                inactive.R, inactive.G, inactive.B),
            "Set bus colors");
    }
}
