using System.Windows;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Preview;

internal static class FlipBudget
{
    public const int AutoDefault = 6;
    private const long LearnWindowMs = 3000;

    private static uint _limitSetting;
    private static int _ceiling = AutoDefault;
    private static readonly HashSet<SwapchainHost> Live = [];
    private static SwapchainHost? _lastAttach;
    private static long _attachTick;
    private static ulong _seenLost;

    public static void Configure(uint limit)
    {
        _limitSetting = IsAllowed(limit) ? limit : 0u;
        _ceiling = _limitSetting == 0
            ? Math.Max(AutoDefault, GpuPresentStore.ObservedCeiling ?? AutoDefault)
            : (int)_limitSetting;
    }

    public static bool TryOpen(int surfaces, Window? owner = null)
    {
        if (!OperatingSystem.IsWindows() || surfaces <= 0)
            return true;
        if (Live.Count + surfaces <= EffectiveMax())
            return true;
        ShowRefuse(owner);
        return false;
    }

    public static bool TryBegin(SwapchainHost host)
    {
        if (!OperatingSystem.IsWindows())
            return true;
        if (Live.Contains(host))
            return true;
        if (Live.Count >= EffectiveMax())
            return false;
        Live.Add(host);
        _lastAttach = host;
        _attachTick = Environment.TickCount64;
        return true;
    }

    public static void Cancel(SwapchainHost host)
    {
        if (!OperatingSystem.IsWindows())
            return;
        Live.Remove(host);
        if (ReferenceEquals(_lastAttach, host))
            _lastAttach = null;
    }

    public static void End(SwapchainHost host)
    {
        Cancel(host);
    }

    public static void ObserveLost(ulong total)
    {
        if (!OperatingSystem.IsWindows() || total <= _seenLost)
            return;
        _seenLost = total;
        if (_lastAttach is null)
            return;
        if (Live.Count <= 2)
            return;
        if (Environment.TickCount64 - _attachTick > LearnWindowMs)
            return;
        var victim = _lastAttach;
        victim.ReleaseNative();
        // Do not ratchet the ceiling down to the remaining count. That leftover
        // cap survives after the extra window is closed and blocks Scene Editor.
        if (_limitSetting == 0 && Live.Count >= AutoDefault)
            GpuPresentStore.Save(Live.Count);
    }

    private static int EffectiveMax() =>
        _limitSetting == 0 ? _ceiling : (int)_limitSetting;

    private static bool IsAllowed(uint limit) =>
        limit is 0 or 4 or 6 or 8 or 10 or 12 or 16;

    private static void ShowRefuse(Window? owner)
    {
        owner ??= Application.Current?.MainWindow;
        if (owner is null)
            MessageBox.Show(Loc.T("msg.flipBudget"));
        else
            MessageBox.Show(owner, Loc.T("msg.flipBudget"));
    }
}
