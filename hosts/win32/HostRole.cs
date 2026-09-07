namespace Eiviz.Host;

internal static class HostRole
{
#if EIVIZ_REMOTE
    public static bool IsRemote { get; } = true;
#else
    public static bool IsRemote { get; } = false;
#endif
}
