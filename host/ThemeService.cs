using System.Windows;
using Microsoft.Win32;

namespace Eiviz.Host;

internal static class ThemeService
{
    private static ResourceDictionary? _theme;

    public static bool IsDark => Resolve(AppPrefs.Current.Theme) == AppThemeMode.Dark;

    public static void Apply(AppThemeMode mode)
    {
        var resolved = Resolve(mode);
        var uri = new Uri(
            resolved == AppThemeMode.Light ? "Themes/Light.xaml" : "Themes/Dark.xaml",
            UriKind.Relative);
        var next = new ResourceDictionary { Source = uri };
        var resources = Application.Current.Resources;
        for (var i = resources.MergedDictionaries.Count - 1; i >= 0; i--)
        {
            var source = resources.MergedDictionaries[i].Source?.OriginalString ?? "";
            if (source.Contains("Themes/", StringComparison.OrdinalIgnoreCase))
                resources.MergedDictionaries.RemoveAt(i);
        }
        resources.MergedDictionaries.Add(next);
        _theme = next;
    }

    public static AppThemeMode Resolve(AppThemeMode mode)
    {
        if (mode != AppThemeMode.System)
            return mode;
        try
        {
            var value = Registry.GetValue(
                @"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "AppsUseLightTheme",
                0);
            return value is int light && light != 0 ? AppThemeMode.Light : AppThemeMode.Dark;
        }
        catch
        {
            return AppThemeMode.Dark;
        }
    }
}
