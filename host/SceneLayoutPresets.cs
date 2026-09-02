using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Shapes;

namespace Eiviz.Host;

internal static class SceneLayoutPresets
{
    public static readonly string[] BuiltIn =
    [
        "Full", "Split H", "Split V", "Quad", "PiP TR", "PiP TL", "PiP BR", "PiP BL"
    ];

    public static IReadOnlyList<(float X, float Y, float W, float H)> Boxes(string name) => name switch
    {
        "Full" => [(0, 0, 1, 1)],
        "Split H" => [(0, 0, 0.5f, 1), (0.5f, 0, 0.5f, 1)],
        "Split V" => [(0, 0, 1, 0.5f), (0, 0.5f, 1, 0.5f)],
        "Quad" => [(0, 0, 0.5f, 0.5f), (0.5f, 0, 0.5f, 0.5f), (0, 0.5f, 0.5f, 0.5f), (0.5f, 0.5f, 0.5f, 0.5f)],
        "PiP TR" => [(0, 0, 1, 1), (0.62f, 0.08f, 0.32f, 0.32f)],
        "PiP TL" => [(0, 0, 1, 1), (0.06f, 0.08f, 0.32f, 0.32f)],
        "PiP BR" => [(0, 0, 1, 1), (0.62f, 0.60f, 0.32f, 0.32f)],
        "PiP BL" => [(0, 0, 1, 1), (0.06f, 0.60f, 0.32f, 0.32f)],
        _ => []
    };

    public static FrameworkElement Mosaic(IReadOnlyList<(float X, float Y, float W, float H)> boxes, double width, double height)
    {
        var canvas = new Canvas
        {
            Width = width,
            Height = height,
            Background = Brushes.Black,
            ClipToBounds = true
        };
        for (var i = 0; i < boxes.Count; i++)
        {
            var (x, y, w, h) = boxes[i];
            var cellW = Math.Max(1, w * width - 1);
            var cellH = Math.Max(1, h * height - 1);
            var cell = new Grid
            {
                Width = cellW,
                Height = cellH
            };
            cell.Children.Add(new Rectangle
            {
                Fill = new SolidColorBrush(Color.FromRgb(0x4A, 0x4A, 0x4A)),
                Stroke = Brushes.Black,
                StrokeThickness = 1
            });
            cell.Children.Add(new TextBlock
            {
                Text = (i + 1).ToString(),
                Foreground = Brushes.White,
                FontWeight = FontWeights.Bold,
                FontSize = Math.Clamp(Math.Min(cellW, cellH) * 0.42, 7, 18),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center
            });
            Canvas.SetLeft(cell, x * width);
            Canvas.SetTop(cell, y * height);
            canvas.Children.Add(cell);
        }
        return canvas;
    }
}
