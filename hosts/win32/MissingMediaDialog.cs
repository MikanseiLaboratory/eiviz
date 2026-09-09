using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using Eiviz.Host.I18n;

namespace Eiviz.Host;

internal sealed class MissingMediaDialog : Window
{
    private readonly Session _session;
    private readonly ListBox _list = new() { Height = 180, Margin = new Thickness(12, 8, 12, 8) };

    public MissingMediaDialog(Session session)
    {
        _session = session;
        Title = Loc.T("input.missingTitle");
        HostDialog.Apply(this);
        Width = 620;
        Height = 380;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ResizeMode = ResizeMode.NoResize;
        var folder = new Button
        {
            Content = Loc.T("input.relinkFolder"),
            MinWidth = 160,
            Padding = new Thickness(12, 4, 12, 4),
            Margin = new Thickness(0, 0, 8, 0)
        };
        folder.Click += (_, _) => RelinkFolder();
        var file = new Button
        {
            Content = Loc.T("input.relinkFile"),
            MinWidth = 140,
            Padding = new Thickness(12, 4, 12, 4),
            Margin = new Thickness(0, 0, 8, 0)
        };
        file.Click += (_, _) => RelinkSelectedFile();
        var close = new Button { Content = Loc.T("dialog.ok"), IsDefault = true, Width = 88 };
        close.Click += (_, _) => DialogResult = true;
        var buttons = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(12, 0, 12, 12)
        };
        buttons.Children.Add(folder);
        buttons.Children.Add(file);
        buttons.Children.Add(close);
        var root = new DockPanel();
        DockPanel.SetDock(buttons, Dock.Bottom);
        root.Children.Add(buttons);
        root.Children.Add(new TextBlock
        {
            Text = Loc.T("input.missingHint"),
            Margin = new Thickness(12, 12, 12, 0),
            TextWrapping = TextWrapping.Wrap
        });
        DockPanel.SetDock(root.Children[root.Children.Count - 1], Dock.Top);
        root.Children.Add(_list);
        Content = root;
        _list.MouseDoubleClick += OnRowDoubleClick;
        Refresh();
    }

    private void OnRowDoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (e.ChangedButton == MouseButton.Left)
            RelinkSelectedFile();
    }

    private void Refresh()
    {
        var selected = SelectedInput()?.Id;
        _list.Items.Clear();
        foreach (var input in SessionStore.MissingMedia(_session))
        {
            var row = new Row(input);
            _list.Items.Add(row);
            if (selected is ulong id && input.Id == id)
                _list.SelectedItem = row;
        }
        if (_list.SelectedItem is null && _list.Items.Count > 0)
            _list.SelectedIndex = 0;
    }

    private InputEntry? SelectedInput() => _list.SelectedItem is Row row ? row.Input : null;

    private void RelinkFolder()
    {
        var dialog = new Microsoft.Win32.OpenFolderDialog { Title = Loc.T("input.relinkFolder") };
        if (dialog.ShowDialog(this) != true)
            return;
        SessionStore.RelinkMissingMedia(_session, [dialog.FolderName]);
        SessionStore.Publish(_session);
        Refresh();
    }

    private void RelinkSelectedFile()
    {
        if (SelectedInput() is not InputEntry input || Owner is not MainWindow owner)
            return;
        var path = owner.PickRelinkFile(input, this);
        if (string.IsNullOrEmpty(path))
            return;
        if (!SessionStore.RelinkInput(input, path))
            return;
        SessionStore.Publish(_session);
        Refresh();
    }

    private sealed class Row(InputEntry input)
    {
        public InputEntry Input { get; } = input;
        public override string ToString() => $"{Input.ListLabel} — {Input.PathOrAddress}";
    }
}
