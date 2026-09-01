using System.Windows;
using System.Windows.Media;

namespace Eiviz.Host.Dialogs;

public partial class ColorPickWindow : Window
{
    public ColorPickWindow(string title, RgbColor color)
    {
        InitializeComponent();
        Title = title;
        PromptText.Text = title;
        Result = color.Clone();
        RSlider.Value = color.R;
        GSlider.Value = color.G;
        BSlider.Value = color.B;
        Paint();
    }

    public RgbColor Result { get; private set; }

    private void Colour_Changed(object sender, RoutedPropertyChangedEventArgs<double> e) => Paint();

    private void Paint()
    {
        if (Swatch is null)
            return;
        Result = new RgbColor
        {
            R = (byte)RSlider.Value,
            G = (byte)GSlider.Value,
            B = (byte)BSlider.Value
        };
        Swatch.Background = new SolidColorBrush(Color.FromRgb(Result.R, Result.G, Result.B));
    }

    private void Ok_Click(object sender, RoutedEventArgs e) => DialogResult = true;
}
