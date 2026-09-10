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
    private readonly bool _originalRemoteOmtUseGpu = AppPrefs.Current.RemoteOmtUseGpu;
    private bool _accepted;
    private bool _suppress;

    public PreferencesWindow()
    {
        InitializeComponent();
        _suppress = true;
        SelectTag(LanguageBox, AppPrefs.Current.Language.ToString());
        SelectTag(ThemeBox, AppPrefs.Current.Theme.ToString());
        SelectTag(RendererBox, AppPrefs.Current.Renderer.ToString());
        SelectTag(RemoteOmtDecodeBox, AppPrefs.Current.RemoteOmtUseGpu ? "Gpu" : "Cpu");
        RemoteFields.Visibility = Visibility.Collapsed;
        RemoteOmtFields.Visibility = HostRole.IsRemote ? Visibility.Visible : Visibility.Collapsed;
        HostListenFields.Visibility = HostRole.IsRemote ? Visibility.Collapsed : Visibility.Visible;
        RemoteUrlBox.Text = AppPrefs.Current.RemoteUrl;
        RemoteTokenBox.Password = CredentialStore.Load(AppPrefs.Current.RemoteUrl);
        ApiBindBox.Text = AppPrefs.Current.NativeApiBind;
        ApiPortBox.Text = AppPrefs.Current.NativeApiPort.ToString();
        ApiTokenBox.Password = CredentialStore.Load("listen");
        MediaDirBox.Text = AppPrefs.Current.ResolvedMediaDirectory;
        ConnectionHelpBlock.Text = Loc.T(HostRole.IsRemote ? "prefs.remoteHelp" : "prefs.hostHelp");
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
    public bool RemoteOmtDecodeChanged => AppPrefs.Current.RemoteOmtUseGpu != _originalRemoteOmtUseGpu;
    public bool ConnectionChanged => false;

    private void PrefsChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress)
            return;
        Apply(ReadLanguage(), ReadTheme(), AppPrefs.Current.Renderer);
        BindDocsLink();
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        var renderer = ReadRenderer();
        if (renderer != _originalRenderer)
        {
            var confirm = MessageBox.Show(
                this,
                Loc.T("prefs.rendererConfirm"),
                Loc.T("prefs.renderer"),
                MessageBoxButton.OKCancel,
                MessageBoxImage.Warning);
            if (confirm != MessageBoxResult.OK)
                return;
        }
        Apply(ReadLanguage(), ReadTheme(), renderer);
        if (HostRole.IsRemote)
            AppPrefs.Current.RemoteOmtUseGpu = ReadRemoteOmtUseGpu();
        if (!HostRole.IsRemote)
        {
            AppPrefs.Current.NativeApiBind = string.IsNullOrWhiteSpace(ApiBindBox.Text)
                ? "127.0.0.1"
                : ApiBindBox.Text.Trim();
            if (uint.TryParse(ApiPortBox.Text, out var port) && port is > 0 and <= 65535)
                AppPrefs.Current.NativeApiPort = port;
            CredentialStore.Save("listen", ApiTokenBox.Password);
            AppPrefs.Current.MediaDirectory = string.IsNullOrWhiteSpace(MediaDirBox.Text)
                ? AppPrefs.DefaultMediaDirectory
                : MediaDirBox.Text.Trim();
        }
        AppPrefs.Current.Save();
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

    private bool ReadRemoteOmtUseGpu()
    {
        if (RemoteOmtDecodeBox.SelectedItem is ComboBoxItem item && item.Tag is string tag)
            return tag == "Gpu";
        return AppPrefs.Current.RemoteOmtUseGpu;
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
