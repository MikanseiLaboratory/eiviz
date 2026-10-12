using System.Windows;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;

namespace Eiviz.Host;

internal static class PlanGate
{
    public static bool EncodeSettingsOpen => RemoteOr(caps => caps.RtmpLinked != 0 && caps.RtmpMaxWidth != 0);

    public static bool DeckLinkOutput => RemoteOr(caps => caps.DecklinkLinked != 0 && caps.DecklinkOutputLimit != 0);

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

    private static bool RemoteOr(Func<MixerCapabilities, bool> allow)
    {
        if (HostRole.IsRemote)
            return true;
        return MixerNative.TryQueryCapabilities(out var caps) && allow(caps);
    }
}
