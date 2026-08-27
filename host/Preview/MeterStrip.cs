using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Shapes;

namespace Eiviz.Host.Preview;

internal enum MeterKind
{
    Bus,
    Input
}

internal sealed class MeterStrip : StackPanel
{
    private readonly Rectangle _left = MakeBar();
    private readonly Rectangle _right = MakeBar();
    private readonly Slider _fader;
    private readonly ToggleButton _mute;
    private readonly WrapPanel _routes = new() { Margin = new Thickness(0, 4, 0, 0) };
    private float _leftPeak;
    private float _rightPeak;
    private const double BarHeight = 88;

    public MeterKind Kind { get; }
    public ulong TargetId { get; }
    public uint BusMask { get; private set; }
    public float Gain { get; private set; } = 1;
    public bool Mute { get; private set; }
    public event Action<ulong, uint>? BusMaskChanged;
    public event Action<ulong, float, bool>? FaderChanged;

    public MeterStrip(MeterKind kind, ulong targetId, string name, float gain, bool mute)
    {
        Kind = kind;
        TargetId = targetId;
        Gain = gain < 0 ? 1 : gain;
        Mute = mute;
        Width = 92;
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
        var row = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center };
        row.Children.Add(Track(_left));
        row.Children.Add(Track(_right));
        _fader = new Slider
        {
            Orientation = Orientation.Vertical,
            Minimum = 0,
            Maximum = 1,
            Value = GainToSlider(Gain),
            Height = BarHeight,
            Width = 18,
            Margin = new Thickness(4, 0, 0, 0),
            Foreground = Brushes.Silver
        };
        _fader.ValueChanged += (_, _) =>
        {
            Gain = SliderToGain(_fader.Value);
            FaderChanged?.Invoke(TargetId, Gain, Mute);
        };
        row.Children.Add(_fader);
        Children.Add(row);
        _mute = new ToggleButton
        {
            Content = "M",
            Width = 22,
            Height = 20,
            FontSize = 10,
            Margin = new Thickness(0, 4, 0, 0),
            HorizontalAlignment = HorizontalAlignment.Center,
            IsChecked = mute,
            ToolTip = "Mute"
        };
        _mute.Click += (_, _) =>
        {
            Mute = _mute.IsChecked == true;
            FaderChanged?.Invoke(TargetId, Gain, Mute);
        };
        Children.Add(_mute);
        Children.Add(_routes);
    }

    public void SetLevels(float left, float right)
    {
        _leftPeak = Math.Max(_leftPeak * 0.82f, ToMeter(left));
        _rightPeak = Math.Max(_rightPeak * 0.82f, ToMeter(right));
        _left.Height = Math.Max(1, BarHeight * _leftPeak);
        _right.Height = Math.Max(1, BarHeight * _rightPeak);
    }

    private static float ToMeter(float linear)
    {
        if (linear <= 1e-5f)
            return 0;
        var db = 20f * MathF.Log10(linear);
        return Math.Clamp((db + 60f) / 60f, 0f, 1f);
    }

    public void Decay() => SetLevels(0, 0);

    public static float SliderToGain(double slider)
    {
        if (slider <= 0.001)
            return 0;
        var db = (float)(slider * 72.0 - 60.0);
        return MathF.Pow(10f, db / 20f);
    }

    public static double GainToSlider(float gain)
    {
        if (gain <= 1e-6f)
            return 0;
        var db = 20f * MathF.Log10(gain);
        return Math.Clamp((db + 60.0) / 72.0, 0, 1);
    }

    public void SetBuses(IReadOnlyList<AudioBusEntry> buses, uint mask)
    {
        BusMask = mask;
        _routes.Children.Clear();
        if (Kind != MeterKind.Input)
            return;
        foreach (var bus in buses)
        {
            var bit = bus.Bit;
            var label = bus.Role switch
            {
                AudioBusRole.Master => "M",
                AudioBusRole.Headphone => "HP",
                _ => bus.Name.StartsWith("Bus ", StringComparison.Ordinal) && bus.Name.Length > 4
                    ? bus.Name[4].ToString()
                    : bus.Name.Length == 0 ? "?" : bus.Name[..1]
            };
            var button = new ToggleButton
            {
                Content = label,
                Width = 22,
                Height = 20,
                FontSize = 10,
                Margin = new Thickness(0, 0, 2, 2),
                IsChecked = (mask & (1u << (int)bit)) != 0,
                ToolTip = bus.Name
            };
            button.Click += (_, _) =>
            {
                var flag = 1u << (int)bit;
                if (button.IsChecked == true)
                    BusMask |= flag;
                else
                    BusMask &= ~flag;
                BusMaskChanged?.Invoke(TargetId, BusMask);
            };
            _routes.Children.Add(button);
        }
    }

    private static Border Track(Rectangle fill)
    {
        var track = new Border
        {
            Width = 10,
            Height = BarHeight,
            Background = new SolidColorBrush(Color.FromRgb(0x22, 0x22, 0x22)),
            Margin = new Thickness(2, 0, 2, 0),
            VerticalAlignment = VerticalAlignment.Bottom
        };
        var canvas = new Canvas { Width = 10, Height = BarHeight };
        fill.Width = 10;
        Canvas.SetBottom(fill, 0);
        canvas.Children.Add(fill);
        track.Child = canvas;
        return track;
    }

    private static LinearGradientBrush MeterFill()
    {
        var brush = new LinearGradientBrush
        {
            StartPoint = new Point(0, 1),
            EndPoint = new Point(0, 0)
        };
        brush.GradientStops.Add(new GradientStop(Color.FromRgb(0x43, 0xA0, 0x47), 0));
        brush.GradientStops.Add(new GradientStop(Color.FromRgb(0xF9, 0xA8, 0x25), 0.72));
        brush.GradientStops.Add(new GradientStop(Color.FromRgb(0xC6, 0x28, 0x28), 1));
        return brush;
    }

    private static Rectangle MakeBar() => new()
    {
        Fill = MeterFill(),
        Height = 1,
        VerticalAlignment = VerticalAlignment.Bottom
    };
}
