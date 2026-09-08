using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;

namespace Eiviz.Host;

internal sealed class RelinkMediaDialog : Window
{
    public string Directory { get; private set; } = "";

    public RelinkMediaDialog()
    {
        Title = Loc.T("input.relink");
        Width = 480;
        Height = 160;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ResizeMode = ResizeMode.NoResize;
        var box = new TextBox { Margin = new Thickness(12, 12, 12, 8) };
        var ok = new Button { Content = Loc.T("dialog.ok"), IsDefault = true, Width = 88, Margin = new Thickness(0, 0, 8, 0) };
        var cancel = new Button { Content = Loc.T("dialog.cancel"), IsCancel = true, Width = 88 };
        ok.Click += (_, _) =>
        {
            Directory = box.Text.Trim();
            if (string.IsNullOrEmpty(Directory))
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
        var hint = new TextBlock
        {
            Text = Loc.T("input.relinkDir"),
            Margin = new Thickness(12, 0, 12, 0)
        };
        var root = new DockPanel();
        DockPanel.SetDock(buttons, Dock.Bottom);
        DockPanel.SetDock(hint, Dock.Top);
        root.Children.Add(buttons);
        root.Children.Add(hint);
        root.Children.Add(box);
        Content = root;
        Loaded += (_, _) => box.Focus();
    }
}
