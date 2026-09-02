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
    private static readonly FontFamily IconFont = new("Segoe MDL2 Assets");
    private const string IconSpeaker = "\uE767";
    private const string IconMute = "\uE74F";
    private const string IconHeadphone = "\uE7F6";

    private readonly Rectangle _left = MakeBar();
    private readonly Rectangle _right = MakeBar();
    private readonly Slider _fader;
    private readonly ToggleButton _mute;
    private readonly TextBlock _dbText;
    private readonly WrapPanel _routes = new()
    {
        Margin = new Thickness(0, 4, 0, 0),
        HorizontalAlignment = HorizontalAlignment.Center,
        ItemHeight = 22
    };
    private float _leftPeak;
    private float _rightPeak;
    private float _leftDb = float.NegativeInfinity;
    private float _rightDb = float.NegativeInfinity;
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
        Width = 108;
        Margin = new Thickness(0, 0, 10, 0);
        Orientation = Orientation.Vertical;
        VerticalAlignment = VerticalAlignment.Top;
        Children.Add(new TextBlock
        {
            Text = name,
            FontSize = 11,
            Height = 16,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = Brushes.Silver,
            Margin = new Thickness(0, 0, 0, 4)
        });
        var row = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center };
        row.Children.Add(Scale());
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
        _dbText = new TextBlock
        {
            Text = "−∞ dB",
            FontSize = 10,
            Foreground = Brushes.Silver,
            HorizontalAlignment = HorizontalAlignment.Center,
            Margin = new Thickness(0, 2, 0, 0)
        };
        Children.Add(_dbText);
        _mute = new ToggleButton
        {
            Content = MuteGlyph(mute),
            FontFamily = IconFont,
            FontSize = 12,
            Width = 26,
            Height = 22,
            Margin = new Thickness(0, 0, 2, 0),
            IsChecked = mute,
            ToolTip = "Mute"
        };
        _mute.Click += (_, _) =>
        {
            Mute = _mute.IsChecked == true;
            _mute.Content = MuteGlyph(Mute);
            FaderChanged?.Invoke(TargetId, Gain, Mute);
        };
        Children.Add(_routes);
        _routes.Children.Add(_mute);
    }

    public void SetLevels(float left, float right)
    {
        _leftDb = ToDb(left);
        _rightDb = ToDb(right);
        _leftPeak = Math.Max(_leftPeak * 0.82f, ToMeter(left));
        _rightPeak = Math.Max(_rightPeak * 0.82f, ToMeter(right));
        _left.Height = Math.Max(1, BarHeight * _leftPeak);
        _right.Height = Math.Max(1, BarHeight * _rightPeak);
        var peak = Math.Max(_leftDb, _rightDb);
        _dbText.Text = float.IsNegativeInfinity(peak) ? "−∞ dB" : $"{peak:0} dB";
    }

    private static float ToMeter(float linear)
    {
        if (linear <= 1e-5f)
            return 0;
        var db = 20f * MathF.Log10(linear);
        return Math.Clamp((db + 60f) / 60f, 0f, 1f);
    }

    private static float ToDb(float linear) =>
        linear <= 1e-5f ? float.NegativeInfinity : 20f * MathF.Log10(linear);

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
        if (Kind == MeterKind.Input)
        {
            foreach (var bus in buses)
            {
                var bit = bus.Bit;
                var button = new ToggleButton
                {
                    Width = 26,
                    Height = 22,
                    FontSize = 10,
                    Margin = new Thickness(0, 0, 2, 0),
                    VerticalAlignment = VerticalAlignment.Center,
                    IsChecked = (mask & (1u << (int)bit)) != 0,
                    ToolTip = bus.Name
                };
                if (bus.Role == AudioBusRole.Master)
                {
                    button.Content = "M";
                }
                else if (bus.Role == AudioBusRole.Headphone)
                {
                    button.Content = IconHeadphone;
                    button.FontFamily = IconFont;
                    button.FontSize = 12;
                }
                else
                {
                    button.Content = bus.Name.StartsWith("Bus ", StringComparison.Ordinal) && bus.Name.Length > 4
                        ? bus.Name[4].ToString()
                        : bus.Name.Length == 0 ? "?" : bus.Name[..1];
                }
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
        _routes.Children.Add(_mute);
    }

    private static object MuteGlyph(bool muted) => muted ? IconMute : IconSpeaker;

    private static Grid Scale()
    {
        var grid = new Grid { Width = 22, Height = BarHeight, Margin = new Thickness(0, 0, 2, 0) };
        grid.Children.Add(Tick(0, "0"));
        grid.Children.Add(Tick(-20, "−20"));
        return grid;
    }

    private static Canvas Tick(float db, string label)
    {
        var y = BarHeight * (1.0 - ((db + 60.0) / 60.0));
        var canvas = new Canvas { Width = 22, Height = BarHeight };
        var line = new Line
        {
            X1 = 14,
            X2 = 22,
            Y1 = y,
            Y2 = y,
            Stroke = new SolidColorBrush(Color.FromRgb(0x88, 0x88, 0x88)),
            StrokeThickness = 1
        };
        var text = new TextBlock
        {
            Text = label,
            FontSize = 9,
            Foreground = new SolidColorBrush(Color.FromRgb(0xAA, 0xAA, 0xAA))
        };
        Canvas.SetTop(text, Math.Max(0, y - 7));
        canvas.Children.Add(line);
        canvas.Children.Add(text);
        return canvas;
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
        var zero = MarkerLine(0);
        var twenty = MarkerLine(-20);
        canvas.Children.Add(zero);
        canvas.Children.Add(twenty);
        track.Child = canvas;
        return track;
    }

    private static Line MarkerLine(float db)
    {
        var y = BarHeight * (1.0 - ((db + 60.0) / 60.0));
        return new Line
        {
            X1 = 0,
            X2 = 10,
            Y1 = y,
            Y2 = y,
            Stroke = new SolidColorBrush(Color.FromArgb(0xA0, 0xEE, 0xEE, 0xEE)),
            StrokeThickness = 1,
            StrokeDashArray = db == 0 ? null : new DoubleCollection { 1, 1 }
        };
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
