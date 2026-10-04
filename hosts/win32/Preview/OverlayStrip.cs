using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;

namespace Eiviz.Host.Preview;

internal sealed class OverlayStrip : StackPanel
{
    public OverlayStrip(string name, bool enabled, Action<bool> changed)
    {
        Width = 56;
        Margin = new Thickness(0, 0, 10, 0);
        Orientation = Orientation.Vertical;
        Children.Add(new TextBlock
        {
            Text = name,
            FontSize = 11,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = Brushes.Silver,
            Margin = new Thickness(0, 0, 0, 4)
        });
        var button = new ToggleButton
        {
            IsChecked = enabled,
            Height = 88,
            Width = 36,
            Padding = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Center,
            Content = enabled ? "ON" : "OFF",
            FontSize = 11
        };
        var ready = false;
        button.Checked += (_, _) =>
        {
            button.Content = "ON";
            if (ready)
                changed(true);
        };
        button.Unchecked += (_, _) =>
        {
            button.Content = "OFF";
            if (ready)
                changed(false);
        };
        Children.Add(button);
        Loaded += (_, _) => ready = true;
    }
}
