namespace Eiviz.Host;

internal static class OnAirLock
{
    private static bool _active;

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
}
