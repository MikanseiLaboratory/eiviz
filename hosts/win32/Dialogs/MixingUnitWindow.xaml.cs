using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;

namespace Eiviz.Host.Dialogs;

public partial class MixingUnitWindow : Window
{
    private readonly List<(uint Kind, string Id, string Name)> _devices;
    private bool _filling;

    public MixingUnitWindow(MixingUnitEntry unit)
    {
        InitializeComponent();
        _devices = AudioGraphSync.EnumerateDevices(0)
            .Where(device => device.Direction != 1)
            .Select(device => (device.Kind, device.Id, device.Name))
            .ToList();
        Result = new MixingUnitEntry
        {
            Id = unit.Id,
            Name = unit.Name,
            Width = unit.Width,
            Height = unit.Height,
            FpsNum = unit.FpsNum,
            FpsDen = unit.FpsDen,
            Audio = unit.Audio.Clone(),
            AudioLink = unit.AudioLink
        };
        NameBox.Text = unit.Name;
        WidthBox.Text = unit.Width.ToString();
        HeightBox.Text = unit.Height.ToString();
        var tag = $"{unit.FpsNum}/{unit.FpsDen}";
        foreach (ComboBoxItem item in FpsBox.Items)
        {
            if (Equals(item.Tag, tag))
                FpsBox.SelectedItem = item;
        }
        SelectTag(KindBox, KindTag(unit.Audio.DeviceKind));
        FillDevices(unit.Audio.DeviceId);
        FillMaps(unit.Audio.MapLeft, unit.Audio.MapRight);
        foreach (ComboBoxItem item in LinkBox.Items)
        {
            if (Equals(item.Tag, unit.AudioLink == AudioLinkMode.Independent ? "independent" : "follow"))
                LinkBox.SelectedItem = item;
        }
    }

    public MixingUnitEntry Result { get; }

    private void Kind_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_filling || DeviceBox is null)
            return;
        Result.Audio.DeviceKind = ReadKind();
        if (Result.Audio.DeviceKind == AudioDeviceKind.None)
            Result.Audio.DeviceId = "";
        FillDevices(Result.Audio.DeviceId);
        FillMaps(Result.Audio.MapLeft, Result.Audio.MapRight);
    }

    private void Device_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_filling || LeftBox is null)
            return;
        Result.Audio.DeviceId = ReadDeviceId();
        FillMaps(Result.Audio.MapLeft, Result.Audio.MapRight);
    }

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
        Result.Audio.DeviceKind = ReadKind();
        Result.Audio.DeviceId = Result.Audio.DeviceKind == AudioDeviceKind.None ? "" : ReadDeviceId();
        Result.Audio.MapLeft = ReadMap(LeftBox, Result.Audio.MapLeft);
        Result.Audio.MapRight = ReadMap(RightBox, Result.Audio.MapRight);
        if (LinkBox.SelectedItem is ComboBoxItem link && link.Tag is string linkTag)
            Result.AudioLink = linkTag == "independent" ? AudioLinkMode.Independent : AudioLinkMode.Follow;
        DialogResult = true;
    }

    private void FillDevices(string deviceId)
    {
        _filling = true;
        DeviceBox.Items.Clear();
        var kind = ReadKind();
        DeviceBox.IsEnabled = kind != AudioDeviceKind.None;
        DeviceBox.Items.Add(new ComboBoxItem
        {
            Content = kind is AudioDeviceKind.Wasapi or AudioDeviceKind.CoreAudio ? "Default" : "(none)",
            Tag = ""
        });
        if (kind != AudioDeviceKind.None)
        {
            foreach (var device in _devices.Where(item => Matches(kind, item.Kind)))
            {
                var label = string.IsNullOrWhiteSpace(device.Name) ? device.Id : device.Name;
                DeviceBox.Items.Add(new ComboBoxItem { Content = label, Tag = device.Id });
            }
        }
        DeviceBox.SelectedIndex = 0;
        for (var i = 0; i < DeviceBox.Items.Count; i++)
        {
            if (DeviceBox.Items[i] is ComboBoxItem item && Equals(item.Tag, deviceId ?? ""))
            {
                DeviceBox.SelectedIndex = i;
                break;
            }
        }
        _filling = false;
    }

    private void FillMaps(int mapLeft, int mapRight)
    {
        _filling = true;
        var kind = ReadKind();
        var channels = 0;
        if (kind != AudioDeviceKind.None)
            MixerNative.AudioDeviceIoChannels((uint)kind, ReadDeviceId(), out _, out channels);
        FillMap(LeftBox, channels, mapLeft);
        FillMap(RightBox, channels, mapRight);
        LeftBox.IsEnabled = channels > 0;
        RightBox.IsEnabled = channels > 0;
        _filling = false;
    }

    private static void FillMap(ComboBox box, int channels, int selected)
    {
        box.Items.Clear();
        for (var index = 0; index < channels; index++)
            box.Items.Add(new ComboBoxItem { Content = (index + 1).ToString(), Tag = index });
        if (box.Items.Count == 0)
            return;
        foreach (ComboBoxItem item in box.Items)
        {
            if (item.Tag is int value && value == selected)
            {
                box.SelectedItem = item;
                return;
            }
        }
        box.SelectedIndex = 0;
    }

    private AudioDeviceKind ReadKind() => KindBox.SelectedItem is ComboBoxItem { Tag: string tag }
        ? tag switch
        {
            "wasapi" => AudioDeviceKind.Wasapi,
            "asio" => AudioDeviceKind.Asio,
            "coreaudio" => AudioDeviceKind.CoreAudio,
            _ => AudioDeviceKind.None
        }
        : AudioDeviceKind.None;

    private string ReadDeviceId() =>
        DeviceBox.SelectedItem is ComboBoxItem { Tag: string id } ? id : "";

    private static int ReadMap(ComboBox box, int current) =>
        box.SelectedItem is ComboBoxItem { Tag: int value } ? value : current;

    private static string KindTag(AudioDeviceKind kind) => kind switch
    {
        AudioDeviceKind.Wasapi => "wasapi",
        AudioDeviceKind.Asio => "asio",
        AudioDeviceKind.CoreAudio => "coreaudio",
        _ => "none"
    };

    private static bool Matches(AudioDeviceKind kind, uint deviceKind) =>
        deviceKind == (uint)kind
        || (kind == AudioDeviceKind.CoreAudio && deviceKind == (uint)AudioDeviceKind.Wasapi)
        || (kind == AudioDeviceKind.Wasapi && deviceKind == (uint)AudioDeviceKind.CoreAudio);

    private static void SelectTag(ComboBox box, string tag)
    {
        foreach (ComboBoxItem item in box.Items)
        {
            if (Equals(item.Tag, tag))
            {
                box.SelectedItem = item;
                return;
            }
        }
    }
}
