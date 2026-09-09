using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using Eiviz.Host.I18n;
using Eiviz.Host.Preview;

namespace Eiviz.Host.Dialogs;

internal sealed class AudioInputWindow : Window
{
    private readonly InputEntry _input;
    private readonly MeterStrip _pre;
    private readonly MeterStrip _post;

    public ulong InputId => _input.Id;
    public event Action<InputEntry, uint, float, bool>? Changed;

    public AudioInputWindow(InputEntry input, IReadOnlyList<AudioBusEntry> buses)
    {
        _input = input;
        Title = Loc.Format("audio.inputTitle", input.ListLabel);
        Width = 280;
        SizeToContent = SizeToContent.Height;
        ResizeMode = ResizeMode.NoResize;
        Background = new SolidColorBrush(Color.FromRgb(0x11, 0x11, 0x11));
        Foreground = new SolidColorBrush(Color.FromRgb(0xEE, 0xEE, 0xEE));
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ShowInTaskbar = false;

        _pre = new MeterStrip(
            MeterKind.Input, input.Id, Loc.T("audio.pre"), input.Gain, input.Mute,
            showFader: false, showOpen: false, showRoutes: false);
        _post = new MeterStrip(
            MeterKind.Input, input.Id, Loc.T("audio.post"), input.Gain, input.Mute,
            showFader: true, showOpen: false, showRoutes: input.Kind != InputKind.Mix);
        if (input.Kind != InputKind.Mix)
            _post.SetBuses(buses, input.BusMask == 0 ? 1u : input.BusMask);
        _post.FaderChanged += (_, gain, mute) =>
            Changed?.Invoke(_input, _post.BusMask == 0 ? 1u : _post.BusMask, gain, mute);
        _post.BusMaskChanged += (_, mask) =>
            Changed?.Invoke(_input, mask == 0 ? 1u : mask, _post.Gain, _post.Mute);

        var row = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Center
        };
        row.Children.Add(_pre);
        row.Children.Add(_post);
        Content = new Border
        {
            Padding = new Thickness(12, 10, 2, 10),
            Child = row
        };
    }

    public void SetPeaks(float left, float right)
    {
        _pre.SetLevels(left, right);
        var post = MeterStrip.PostPeak(left, right, _post.Gain, _post.Mute);
        _post.SetLevels(post.L, post.R);
    }

    public void Sync(float gain, bool mute, uint mask)
    {
        _pre.SyncFrom(gain, mute);
        _post.SyncFrom(gain, mute);
        if (_post.BusMask != mask)
            _post.SetBuses(((App)Application.Current).Session.Buses, mask);
    }

    public void SetBuses(IReadOnlyList<AudioBusEntry> buses, uint mask) =>
        _post.SetBuses(buses, mask);
}
