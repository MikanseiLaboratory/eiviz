using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Shapes;

namespace Eiviz.Host.Preview;

internal sealed class MeterStrip : StackPanel
{
    private readonly Rectangle _left = MakeBar();
    private readonly Rectangle _right = MakeBar();
    private float _leftPeak;
    private float _rightPeak;
    private const double BarHeight = 88;

    public MeterStrip(string name)
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
        var row = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center };
        row.Children.Add(Track(_left));
        row.Children.Add(Track(_right));
        Children.Add(row);
    }

    public void SetLevels(float left, float right)
    {
        _leftPeak = Math.Max(_leftPeak * 0.82f, left);
        _rightPeak = Math.Max(_rightPeak * 0.82f, right);
        _left.Height = Math.Max(1, BarHeight * _leftPeak);
        _right.Height = Math.Max(1, BarHeight * _rightPeak);
    }

    public void Decay() => SetLevels(0, 0);

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
