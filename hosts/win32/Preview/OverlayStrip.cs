using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;

namespace Eiviz.Host.Preview;

internal enum StripLamp
{
    Off,
    On,
    Warn,
    Fault
}

internal sealed class OverlayStrip : StackPanel
{
    static readonly SolidColorBrush WarnBrush = Freeze(Color.FromRgb(0xF9, 0xA8, 0x25));
    static readonly SolidColorBrush FaultBrush = Freeze(Color.FromRgb(0xC6, 0x28, 0x28));
    static readonly SolidColorBrush WarnText = Freeze(Color.FromRgb(0x11, 0x11, 0x11));
    static readonly SolidColorBrush FaultText = Freeze(Colors.White);

    readonly ToggleButton _button;
    StripLamp _lamp = StripLamp.On;
    string? _tip;

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
        _button = new ToggleButton
        {
            IsChecked = enabled,
            Height = 88,
            Width = 36,
            Padding = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Center,
            Content = enabled ? "ON" : "OFF",
            FontSize = 11,
            Style = LampStyle()
        };
        var ready = false;
        _button.Checked += (_, _) =>
        {
            _button.Content = "ON";
            Paint();
            if (ready)
                changed(true);
        };
        _button.Unchecked += (_, _) =>
        {
            _button.Content = "OFF";
            Paint();
            if (ready)
                changed(false);
        };
        Children.Add(_button);
        Loaded += (_, _) =>
        {
            ready = true;
            Paint();
        };
        Paint();
    }

    public void SetLamp(StripLamp lamp, string? tip)
    {
        if (_lamp == lamp && _tip == tip)
            return;
        _lamp = lamp;
        _tip = tip;
        _button.ToolTip = string.IsNullOrWhiteSpace(tip) ? null : tip;
        Paint();
    }

    void Paint()
    {
        var on = _button.IsChecked == true && _lamp != StripLamp.Off;
        if (!on)
        {
            _button.Background = Brush("ToggleOffBackground");
            _button.BorderBrush = Brush("ToggleOffBorder");
            _button.Foreground = Brush("ToggleOffForeground");
            _button.FontWeight = FontWeights.Normal;
            return;
        }
        _button.FontWeight = FontWeights.Bold;
        switch (_lamp)
        {
            case StripLamp.Warn:
                _button.Background = WarnBrush;
                _button.BorderBrush = WarnBrush;
                _button.Foreground = WarnText;
                break;
            case StripLamp.Fault:
                _button.Background = FaultBrush;
                _button.BorderBrush = FaultBrush;
                _button.Foreground = FaultText;
                break;
            default:
                _button.Background = Brush("ToggleOnBackground");
                _button.BorderBrush = Brush("ToggleOnBorder");
                _button.Foreground = Brush("ToggleOnForeground");
                break;
        }
    }

    static Brush Brush(string key)
    {
        if (Application.Current?.TryFindResource(key) is Brush brush)
            return brush;
        return Brushes.Gray;
    }

    static SolidColorBrush Freeze(Color color)
    {
        var brush = new SolidColorBrush(color);
        brush.Freeze();
        return brush;
    }

    static Style LampStyle()
    {
        var border = new FrameworkElementFactory(typeof(Border));
        border.SetValue(Border.BorderThicknessProperty, new Thickness(1));
        border.SetBinding(Border.BackgroundProperty, new System.Windows.Data.Binding(nameof(Control.Background))
        {
            RelativeSource = new System.Windows.Data.RelativeSource(System.Windows.Data.RelativeSourceMode.TemplatedParent)
        });
        border.SetBinding(Border.BorderBrushProperty, new System.Windows.Data.Binding(nameof(Control.BorderBrush))
        {
            RelativeSource = new System.Windows.Data.RelativeSource(System.Windows.Data.RelativeSourceMode.TemplatedParent)
        });
        var presenter = new FrameworkElementFactory(typeof(ContentPresenter));
        presenter.SetValue(ContentPresenter.HorizontalAlignmentProperty, HorizontalAlignment.Center);
        presenter.SetValue(ContentPresenter.VerticalAlignmentProperty, VerticalAlignment.Center);
        border.AppendChild(presenter);
        var template = new ControlTemplate(typeof(ToggleButton)) { VisualTree = border };
        var disabled = new Trigger { Property = UIElement.IsEnabledProperty, Value = false };
        disabled.Setters.Add(new Setter(UIElement.OpacityProperty, 0.5));
        template.Triggers.Add(disabled);
        var style = new Style(typeof(ToggleButton));
        style.Setters.Add(new Setter(Control.TemplateProperty, template));
        style.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(0)));
        return style;
    }
}
