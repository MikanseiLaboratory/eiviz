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
        var idle = new SolidColorBrush(Color.FromRgb(0x44, 0x44, 0x44));
        var on = new SolidColorBrush(Color.FromRgb(0x88, 0x88, 0x88));
        var bg = new SolidColorBrush(Color.FromRgb(0x22, 0x22, 0x22));
        var chrome = new FrameworkElementFactory(typeof(Border));
        chrome.Name = "Chrome";
        chrome.SetValue(Border.BackgroundProperty, bg);
        chrome.SetValue(Border.BorderBrushProperty, enabled ? on : idle);
        chrome.SetValue(Border.BorderThicknessProperty, new Thickness(1));
        var content = new FrameworkElementFactory(typeof(ContentPresenter));
        content.SetValue(ContentPresenter.HorizontalAlignmentProperty, HorizontalAlignment.Center);
        content.SetValue(ContentPresenter.VerticalAlignmentProperty, VerticalAlignment.Center);
        chrome.AppendChild(content);
        var template = new ControlTemplate(typeof(ToggleButton)) { VisualTree = chrome };
        var checkedTrigger = new Trigger { Property = ToggleButton.IsCheckedProperty, Value = true };
        checkedTrigger.Setters.Add(new Setter(Border.BorderBrushProperty, on, "Chrome"));
        var uncheckedTrigger = new Trigger { Property = ToggleButton.IsCheckedProperty, Value = false };
        uncheckedTrigger.Setters.Add(new Setter(Border.BorderBrushProperty, idle, "Chrome"));
        template.Triggers.Add(checkedTrigger);
        template.Triggers.Add(uncheckedTrigger);
        var button = new ToggleButton
        {
            IsChecked = enabled,
            Height = 88,
            Width = 36,
            HorizontalAlignment = HorizontalAlignment.Center,
            Content = enabled ? "ON" : "OFF",
            FontSize = 11,
            Foreground = Brushes.White,
            Template = template
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
