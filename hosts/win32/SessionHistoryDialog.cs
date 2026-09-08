using System.Globalization;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.I18n;

namespace Eiviz.Host;

internal sealed class SessionHistoryDialog : Window
{
    public uint? HistoryIndex { get; private set; }

    public SessionHistoryDialog(IReadOnlyList<SessionHistoryEntry> entries)
    {
        Title = Loc.T("history.title");
        Width = 420;
        Height = 320;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ResizeMode = ResizeMode.NoResize;
        var list = new ListBox { Margin = new Thickness(12) };
        list.Items.Add(new ListBoxItem { Content = Loc.T("history.latest"), Tag = null });
        foreach (var entry in entries)
        {
            var when = DateTimeOffset.FromUnixTimeMilliseconds((long)entry.UnixMs)
                .ToLocalTime()
                .ToString("g", CultureInfo.CurrentCulture);
            list.Items.Add(new ListBoxItem
            {
                Content = Loc.Format("history.entry", entry.Index, when, entry.Revision),
                Tag = entry.Index
            });
        }
        list.SelectedIndex = 0;
        var ok = new Button { Content = Loc.T("history.open"), IsDefault = true, Width = 88, Margin = new Thickness(0, 0, 8, 0) };
        var cancel = new Button { Content = Loc.T("history.cancel"), IsCancel = true, Width = 88 };
        ok.Click += (_, _) =>
        {
            HistoryIndex = (list.SelectedItem as ListBoxItem)?.Tag is uint index ? index : null;
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
        var root = new DockPanel();
        DockPanel.SetDock(buttons, Dock.Bottom);
        root.Children.Add(buttons);
        root.Children.Add(list);
        Content = root;
    }

    public static IReadOnlyList<SessionHistoryEntry> Parse(string json)
    {
        if (string.IsNullOrWhiteSpace(json) || json == "[]")
            return [];
        try
        {
            return JsonSerializer.Deserialize<List<SessionHistoryEntry>>(json, new JsonSerializerOptions
            {
                PropertyNamingPolicy = JsonNamingPolicy.CamelCase
            }) ?? [];
        }
        catch (JsonException)
        {
            return [];
        }
    }
}

internal sealed class SessionHistoryEntry
{
    public uint Index { get; set; }
    public ulong UnixMs { get; set; }
    public ulong Revision { get; set; }
}
