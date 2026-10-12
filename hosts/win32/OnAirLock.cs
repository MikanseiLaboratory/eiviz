using System.Windows;
using Eiviz.Host.I18n;

namespace Eiviz.Host;

internal static class OnAirLock
{
    private static bool _active;
    private static int _closeBypass;

    public static bool Active
    {
        get => _active;
        set
        {
            if (_active == value)
                return;
            _active = value;
            Changed?.Invoke();
        }
    }

    public static event Action? Changed;

    public static bool ShouldCancelClose() => _active && _closeBypass == 0;

    public static IDisposable AllowClose()
    {
        _closeBypass++;
        return new CloseBypass();
    }

    public static bool BlockClosing(Window window, System.ComponentModel.CancelEventArgs e)
    {
        if (!ShouldCancelClose())
            return false;
        e.Cancel = true;
        MessageBox.Show(window, Loc.T("msg.onAirLock"), Loc.T("chrome.onAirLock"));
        return true;
    }

    private sealed class CloseBypass : IDisposable
    {
        public void Dispose()
        {
            if (_closeBypass > 0)
                _closeBypass--;
        }
    }
}
