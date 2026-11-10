using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;

namespace Eiviz.Host;

internal static class PlanGate
{
    public static bool EncodeSettingsOpen => RemoteOr(caps => caps.RtmpLinked != 0 && caps.RtmpMaxWidth != 0);

    public static bool DeckLinkOutput => RemoteOr(caps => caps.DecklinkLinked != 0 && caps.DecklinkOutputLimit != 0);

    public static bool ReplayRecord => RemoteOr(caps => caps.ReplayLinked != 0 && caps.ReplayAngleLimit != 0);

    /// <summary>
    /// Replay plan limits. Remote mode cannot read them yet, so it reports
    /// "unknown" instead of inventing a number.
    /// </summary>
    public static bool TryReplayLimits(out uint angles, out uint channels)
    {
        angles = 0;
        channels = 0;
        if (HostRole.IsRemote || !MixerNative.TryQueryCapabilities(out var caps))
            return false;
        angles = caps.ReplayAngleLimit;
        channels = caps.ReplayChannelLimit;
        return true;
    }

    public static bool CanAddMixingUnit(int existing) =>
        RemoteOr(caps => caps.MixingUnitLimit == uint.MaxValue || (uint)existing < caps.MixingUnitLimit);

    public static bool AllowsEncodeSize(uint width, uint height)
    {
        if (width == 0 || height == 0)
            return true;
        return RemoteOr(caps =>
            caps.RtmpMaxWidth != 0
            && caps.RtmpMaxHeight != 0
            && width <= caps.RtmpMaxWidth
            && height <= caps.RtmpMaxHeight);
    }

    public static bool AllowsEncodeFps(uint fpsNum, uint fpsDen)
    {
        if (fpsNum == 0 || fpsDen == 0)
            return true;
        return RemoteOr(caps =>
            caps.RtmpMaxFpsNum != 0
            && caps.RtmpMaxFpsDen != 0
            && (ulong)fpsNum * caps.RtmpMaxFpsDen <= (ulong)caps.RtmpMaxFpsNum * fpsDen);
    }

    public static void Tip(Window? owner, string key) =>
        StatusToast.ShowTip(owner, Loc.T(key));

    public static Border HoverDisabled(FrameworkElement choice, string key, bool fade = false)
    {
        choice.IsEnabled = false;
        if (fade)
            choice.Opacity = 0.45;
        return new Border
        {
            Background = Brushes.Transparent,
            Child = choice,
            ToolTip = Loc.T(key),
            HorizontalAlignment = HorizontalAlignment.Stretch
        };
    }

    public static void BlockComboItem(ComboBoxItem item, string key)
    {
        item.ToolTip = Loc.T(key);
        ToolTipService.SetShowOnDisabled(item, true);
        item.IsEnabled = false;
    }

    private static bool RemoteOr(Func<MixerCapabilities, bool> allow)
    {
        if (HostRole.IsRemote)
            return true;
        return MixerNative.TryQueryCapabilities(out var caps) && allow(caps);
    }
}
