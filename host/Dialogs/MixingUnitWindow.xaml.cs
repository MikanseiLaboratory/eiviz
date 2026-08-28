using System.Windows;
using System.Windows.Controls;

namespace Eiviz.Host.Dialogs;

public partial class MixingUnitWindow : Window
{
    public MixingUnitWindow(MixingUnitEntry unit, IReadOnlyList<AudioBusEntry> buses)
    {
        InitializeComponent();
        NameBox.Text = unit.Name;
        WidthBox.Text = unit.Width.ToString();
        HeightBox.Text = unit.Height.ToString();
        var tag = $"{unit.FpsNum}/{unit.FpsDen}";
        foreach (ComboBoxItem item in FpsBox.Items)
        {
            if (Equals(item.Tag, tag))
                FpsBox.SelectedItem = item;
        }
        BusBox.ItemsSource = buses;
        BusBox.SelectedItem = buses.FirstOrDefault(item => item.Id == unit.AudioBusId) ?? buses.FirstOrDefault();
        foreach (ComboBoxItem item in LinkBox.Items)
        {
            if (Equals(item.Tag, unit.AudioLink == AudioLinkMode.Independent ? "independent" : "follow"))
                LinkBox.SelectedItem = item;
        }
        Result = new MixingUnitEntry
        {
            Id = unit.Id,
            Name = unit.Name,
            Width = unit.Width,
            Height = unit.Height,
            FpsNum = unit.FpsNum,
            FpsDen = unit.FpsDen,
            AudioBusId = unit.AudioBusId == 0 ? 1 : unit.AudioBusId,
            AudioLink = unit.AudioLink
        };
    }

    public MixingUnitEntry Result { get; }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        if (!uint.TryParse(WidthBox.Text, out var width) || width == 0)
            return;
        if (!uint.TryParse(HeightBox.Text, out var height) || height == 0)
            return;
        Result.Name = string.IsNullOrWhiteSpace(NameBox.Text) ? Result.Name : NameBox.Text.Trim();
        Result.Width = width;
        Result.Height = height;
        if (FpsBox.SelectedItem is ComboBoxItem item && item.Tag is string tag)
        {
            var parts = tag.Split('/');
            Result.FpsNum = uint.Parse(parts[0]);
            Result.FpsDen = uint.Parse(parts[1]);
        }
        if (BusBox.SelectedItem is AudioBusEntry bus)
            Result.AudioBusId = bus.Id;
        if (LinkBox.SelectedItem is ComboBoxItem link && link.Tag is string linkTag)
            Result.AudioLink = linkTag == "independent" ? AudioLinkMode.Independent : AudioLinkMode.Follow;
        DialogResult = true;
    }
}
