using System.IO;
using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;

namespace Eiviz.Host;

internal sealed class ImportExportDialog : Window
{
    public string SessionPath { get; private set; } = "";
    public string MediaDirectory { get; private set; } = "";

    public ImportExportDialog(string exportPath)
    {
        Title = Loc.T("chrome.importExport");
        HostDialog.Apply(this);
        Width = 560;
        Height = 240;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ResizeMode = ResizeMode.NoResize;
        var stem = Path.GetFileNameWithoutExtension(exportPath);
        var parent = Path.GetDirectoryName(exportPath) ?? "";
        var sessionBox = new TextBox
        {
            Text = Path.Combine(parent, stem + ".eivz"),
            Margin = new Thickness(12, 4, 12, 8)
        };
        var mediaBox = new TextBox
        {
            Text = Path.Combine(parent, stem + ".media"),
            Margin = new Thickness(12, 4, 12, 8)
        };
        var sessionBrowse = new Button { Content = Loc.T("dialog.browse"), Width = 88, Margin = new Thickness(0, 0, 12, 8) };
        sessionBrowse.Click += (_, _) =>
        {
            var dialog = new Microsoft.Win32.SaveFileDialog
            {
                Filter = Loc.T("filter.sessionSave"),
                FileName = Path.GetFileName(sessionBox.Text)
            };
            if (dialog.ShowDialog(this) == true)
                sessionBox.Text = dialog.FileName;
        };
        var mediaBrowse = new Button { Content = Loc.T("dialog.browse"), Width = 88, Margin = new Thickness(0, 0, 12, 8) };
        mediaBrowse.Click += (_, _) =>
        {
            var dialog = new Microsoft.Win32.OpenFolderDialog { Title = Loc.T("chrome.importMedia") };
            if (dialog.ShowDialog(this) == true)
                mediaBox.Text = dialog.FolderName;
        };
        var ok = new Button { Content = Loc.T("dialog.ok"), IsDefault = true, Width = 88, Margin = new Thickness(0, 0, 8, 0) };
        var cancel = new Button { Content = Loc.T("dialog.cancel"), IsCancel = true, Width = 88 };
        ok.Click += (_, _) =>
        {
            SessionPath = sessionBox.Text.Trim();
            MediaDirectory = mediaBox.Text.Trim();
            if (string.IsNullOrEmpty(SessionPath) || string.IsNullOrEmpty(MediaDirectory))
                return;
            DialogResult = true;
        };
        var buttons = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(12, 0, 12, 12)
        };
        buttons.Children.Add(ok);
        buttons.Children.Add(cancel);
        var sessionRow = new DockPanel();
        DockPanel.SetDock(sessionBrowse, Dock.Right);
        sessionRow.Children.Add(sessionBrowse);
        sessionRow.Children.Add(sessionBox);
        var mediaRow = new DockPanel();
        DockPanel.SetDock(mediaBrowse, Dock.Right);
        mediaRow.Children.Add(mediaBrowse);
        mediaRow.Children.Add(mediaBox);
        var root = new StackPanel();
        root.Children.Add(new TextBlock
        {
            Text = Loc.T("chrome.importExportHint"),
            Margin = new Thickness(12, 12, 12, 8),
            TextWrapping = TextWrapping.Wrap
        });
        root.Children.Add(new TextBlock { Text = Loc.T("chrome.importSession"), Margin = new Thickness(12, 0, 12, 0) });
        root.Children.Add(sessionRow);
        root.Children.Add(new TextBlock { Text = Loc.T("chrome.importMedia"), Margin = new Thickness(12, 0, 12, 0) });
        root.Children.Add(mediaRow);
        root.Children.Add(buttons);
        Content = root;
    }
}
