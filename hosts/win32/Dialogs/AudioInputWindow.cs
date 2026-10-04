using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using Eiviz.Host.I18n;
using Eiviz.Host.Preview;

namespace Eiviz.Host.Dialogs;

internal sealed class AudioInputWindow : Window
{
    private readonly InputEntry _input;
    private readonly IReadOnlyList<MixingUnitEntry> _units;
    private readonly MeterStrip _pre;
    private readonly MeterStrip _post;

    public ulong InputId => _input.Id;
    public event Action<InputEntry, IReadOnlyList<ulong>, float, bool>? Changed;
    public event Action<MeterStrip>? ListenRequested;
    public event Action<ulong, ulong>? FollowCleared;

    public AudioInputWindow(InputEntry input, IReadOnlyList<MixingUnitEntry> units)
    {
        _input = input;
        _units = units;
        Title = Loc.Format("audio.inputTitle", input.ListLabel);
        Width = 420;
        SizeToContent = SizeToContent.Height;
        ResizeMode = ResizeMode.NoResize;
        Background = new SolidColorBrush(Color.FromRgb(0x11, 0x11, 0x11));
        Foreground = new SolidColorBrush(Color.FromRgb(0xEE, 0xEE, 0xEE));
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ShowInTaskbar = false;

        _pre = new MeterStrip(
            MeterKind.Input, input.Id, Loc.T("audio.pre"), input.Gain, input.Mute,
            showFader: false, showOpen: false, showRoutes: false, showListen: false);
        _post = new MeterStrip(
            MeterKind.Input, input.Id, Loc.T("audio.post"), input.Gain, input.Mute,
            showFader: true, showOpen: false, showRoutes: input.Kind != InputKind.Mix);
        if (input.Kind != InputKind.Mix)
            _post.SetRoutes(units, input.AudioUnits);
        _post.FaderChanged += (_, gain, mute) =>
            Changed?.Invoke(_input, _post.Routes, gain, mute);
        _post.RoutesChanged += (_, routes) =>
            Changed?.Invoke(_input, routes, _post.Gain, _post.Mute);
        _post.FollowCleared += (inputId, unitId) => FollowCleared?.Invoke(inputId, unitId);
        _post.ListenRequested += strip => ListenRequested?.Invoke(strip);

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

    public void Sync(float gain, bool mute, IReadOnlyList<ulong> routes, IReadOnlyList<ulong>? followed = null)
    {
        _pre.SyncFrom(gain, mute);
        _post.SyncFrom(gain, mute);
        if (!SameRoutes(_post.Routes, routes) || followed is not null)
            _post.SetRoutes(_units, routes, followed);
    }

    public void SetRoutes(IReadOnlyList<ulong> routes, IReadOnlyList<ulong>? followed = null) =>
        _post.SetRoutes(_units, routes, followed);

    public void SetListening(bool on) => _post.SetListening(on);

    private static bool SameRoutes(IReadOnlyList<ulong> left, IReadOnlyList<ulong> right) =>
        left.Count == right.Count && left.Order().SequenceEqual(right.Order());
}
