using System.Windows;
using System.Windows.Media;

namespace Eiviz.Host;

internal static class HostDialog
{
    public static void Apply(Window window)
    {
        window.Background = Brush("DialogBackground");
        window.Foreground = Brush("TextForeground");
    }

    public static Brush Brush(string key) =>
        Application.Current?.TryFindResource(key) as Brush ?? Brushes.Gray;
}
