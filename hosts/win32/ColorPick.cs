using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace Eiviz.Host;

internal static class ColorPick
{
    public static Button Swatch(Func<(float R, float G, float B)> get, Action<float, float, float> set)
    {
        var btn = new Button
        {
            Height = 26,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Margin = new Thickness(0, 0, 0, 4)
        };
        void Paint()
        {
            var (r, g, b) = get();
            var color = ToColor(r, g, b);
            btn.Background = new SolidColorBrush(color);
            btn.Content = $"  #{color.R:X2}{color.G:X2}{color.B:X2}";
            btn.Foreground = color.R + color.G + color.B > 380 ? Brushes.Black : Brushes.White;
        }
        Paint();
        btn.Click += (_, _) =>
        {
            var (r, g, b) = get();
            var win = new Window
            {
                Title = "Color",
                Width = 280,
                Height = 200,
                WindowStartupLocation = WindowStartupLocation.CenterOwner,
                Owner = Window.GetWindow(btn),
                Background = new SolidColorBrush(Color.FromRgb(0x2B, 0x2B, 0x2B)),
                Foreground = Brushes.White,
                ResizeMode = ResizeMode.NoResize
            };
            var red = new Slider { Minimum = 0, Maximum = 1, Value = Clamp01(r) };
            var green = new Slider { Minimum = 0, Maximum = 1, Value = Clamp01(g) };
            var blue = new Slider { Minimum = 0, Maximum = 1, Value = Clamp01(b) };
            var preview = new Border { Height = 28, Margin = new Thickness(0, 8, 0, 8) };
            void RefreshPreview() =>
                preview.Background = new SolidColorBrush(ToColor((float)red.Value, (float)green.Value, (float)blue.Value));
            red.ValueChanged += (_, _) => RefreshPreview();
            green.ValueChanged += (_, _) => RefreshPreview();
            blue.ValueChanged += (_, _) => RefreshPreview();
            RefreshPreview();
            var ok = new Button { Content = "OK", Width = 72, Height = 26, IsDefault = true };
            ok.Click += (_, _) =>
            {
                set((float)red.Value, (float)green.Value, (float)blue.Value);
                Paint();
                win.Close();
            };
            var cancel = new Button { Content = "Cancel", Width = 72, Height = 26, Margin = new Thickness(8, 0, 0, 0), IsCancel = true };
            var buttons = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right };
            buttons.Children.Add(ok);
            buttons.Children.Add(cancel);
            var stack = new StackPanel { Margin = new Thickness(12) };
            stack.Children.Add(Row("R", red));
            stack.Children.Add(Row("G", green));
            stack.Children.Add(Row("B", blue));
            stack.Children.Add(preview);
            stack.Children.Add(buttons);
            win.Content = stack;
            win.ShowDialog();
        };
        return btn;
    }

    private static DockPanel Row(string label, Slider slider)
    {
        slider.Width = 160;
        var box = new TextBox { Width = 40, Margin = new Thickness(8, 0, 0, 0), Text = ByteText(slider.Value) };
        slider.ValueChanged += (_, _) => box.Text = ByteText(slider.Value);
        box.LostFocus += (_, _) =>
        {
            if (int.TryParse(box.Text, out var n))
                slider.Value = Math.Clamp(n, 0, 255) / 255.0;
        };
        var row = new DockPanel { Margin = new Thickness(0, 0, 0, 4) };
        row.Children.Add(new TextBlock { Text = label, Width = 18, VerticalAlignment = VerticalAlignment.Center });
        row.Children.Add(slider);
        row.Children.Add(box);
        return row;
    }

    private static string ByteText(double value) => ((int)Math.Round(Clamp01((float)value) * 255)).ToString();

    private static float Clamp01(float value) => Math.Clamp(value, 0, 1);

    private static Color ToColor(float r, float g, float b) =>
        Color.FromRgb(
            (byte)Math.Clamp(r * 255f, 0, 255),
            (byte)Math.Clamp(g * 255f, 0, 255),
            (byte)Math.Clamp(b * 255f, 0, 255));
}
