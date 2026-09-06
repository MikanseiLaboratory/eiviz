using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Navigation;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Dialogs;

public partial class PreferencesWindow : Window
{
    private readonly AppLanguage _originalLanguage = AppPrefs.Current.Language;
    private readonly AppThemeMode _originalTheme = AppPrefs.Current.Theme;
    private readonly GpuRenderer _originalRenderer = AppPrefs.Current.Renderer;
    private bool _accepted;
    private bool _suppress;

    public PreferencesWindow()
    {
        InitializeComponent();
        _suppress = true;
        SelectTag(LanguageBox, AppPrefs.Current.Language.ToString());
        SelectTag(ThemeBox, AppPrefs.Current.Theme.ToString());
        SelectTag(RendererBox, AppPrefs.Current.Renderer.ToString());
        _suppress = false;
        AboutVersion.Text = $"Version {HostVersion.Display}";
        BindDocsLink();
        Closed += (_, _) =>
        {
            if (!_accepted)
                Apply(_originalLanguage, _originalTheme, _originalRenderer);
        };
    }

    public bool RendererChanged => AppPrefs.Current.Renderer != _originalRenderer;

    private void PrefsChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress)
            return;
        Apply(ReadLanguage(), ReadTheme(), AppPrefs.Current.Renderer);
        BindDocsLink();
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        Apply(ReadLanguage(), ReadTheme(), ReadRenderer());
        _accepted = true;
        DialogResult = true;
    }

    private AppLanguage ReadLanguage()
    {
        if (LanguageBox.SelectedItem is ComboBoxItem item && item.Tag is string tag
            && Enum.TryParse<AppLanguage>(tag, out var language))
            return language;
        return AppPrefs.Current.Language;
    }

    private AppThemeMode ReadTheme()
    {
        if (ThemeBox.SelectedItem is ComboBoxItem item && item.Tag is string tag
            && Enum.TryParse<AppThemeMode>(tag, out var mode))
            return mode;
        return AppPrefs.Current.Theme;
    }

    private GpuRenderer ReadRenderer()
    {
        if (RendererBox.SelectedItem is ComboBoxItem item && item.Tag is string tag
            && Enum.TryParse<GpuRenderer>(tag, out var renderer))
            return renderer;
        return AppPrefs.Current.Renderer;
    }

    private static void Apply(AppLanguage language, AppThemeMode theme, GpuRenderer renderer)
    {
        AppPrefs.Current.Language = language;
        AppPrefs.Current.Theme = theme;
        AppPrefs.Current.Renderer = renderer;
        AppPrefs.Current.Save();
        Loc.Apply(language);
        ThemeService.Apply(theme);
    }

    private void BindDocsLink()
    {
        var url = Loc.T("prefs.docsUrl");
        DocsLink.NavigateUri = new Uri(url);
        DocsLinkText.Text = url;
    }

    private void AboutLink_RequestNavigate(object sender, RequestNavigateEventArgs e)
    {
        Process.Start(new ProcessStartInfo(e.Uri.AbsoluteUri) { UseShellExecute = true });
        e.Handled = true;
    }

    private void OpenNotices_Click(object sender, RoutedEventArgs e)
    {
        var path = System.IO.Path.Combine(AppContext.BaseDirectory, "THIRD_PARTY_NOTICES.md");
        if (!System.IO.File.Exists(path))
            return;
        Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
    }

    private static void SelectTag(ComboBox box, string tag)
    {
        foreach (ComboBoxItem item in box.Items)
        {
            if (Equals(item.Tag, tag))
            {
                box.SelectedItem = item;
                return;
            }
        }
    }
}
