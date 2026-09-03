using System.Windows;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Preview;

internal static class FlipBudget
{
    public const int AutoDefault = 8;
    private const long LearnWindowMs = 3000;

    private static uint _limitSetting;
    private static int _ceiling = AutoDefault;
    private static int _attached;
    private static SwapchainHost? _lastAttach;
    private static long _attachTick;
    private static ulong _seenLost;
    private static bool _refusedOnce;

    public static void Configure(uint limit)
    {
        _limitSetting = IsAllowed(limit) ? limit : 0u;
        _ceiling = _limitSetting == 0
            ? GpuPresentStore.ObservedCeiling ?? AutoDefault
            : (int)_limitSetting;
    }

    public static bool TryBegin(SwapchainHost host)
    {
        if (!OperatingSystem.IsWindows())
            return true;
        if (_attached >= EffectiveMax())
        {
            ShowRefuse();
            return false;
        }
        _attached++;
        _lastAttach = host;
        _attachTick = Environment.TickCount64;
        return true;
    }

    public static void Cancel(SwapchainHost host)
    {
        if (!OperatingSystem.IsWindows())
            return;
        if (_attached > 0)
            _attached--;
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
        if (_attached <= 2)
            return;
        if (Environment.TickCount64 - _attachTick > LearnWindowMs)
            return;
        var victim = _lastAttach;
        victim.ReleaseNative();
        if (_limitSetting == 0 && _attached >= 2)
            GpuPresentStore.Save(_attached);
        _ceiling = Math.Max(2, _attached);
    }

    private static int EffectiveMax() =>
        _limitSetting == 0 ? _ceiling : (int)_limitSetting;

    private static bool IsAllowed(uint limit) =>
        limit is 0 or 4 or 6 or 8 or 10 or 12 or 16;

    private static void ShowRefuse()
    {
        if (_refusedOnce)
            return;
        _refusedOnce = true;
        var app = Application.Current;
        if (app is null)
            return;
        app.Dispatcher.BeginInvoke(() =>
        {
            var owner = app.MainWindow;
            if (owner is null)
                MessageBox.Show(Loc.T("msg.flipBudget"));
            else
                MessageBox.Show(owner, Loc.T("msg.flipBudget"));
        });
    }
}
