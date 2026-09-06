using System.IO;

namespace Eiviz.Host;

internal static class HostProcess
{
    public static bool IsRemote =>
        string.Equals(ProcessName, "Eiviz.Remote", StringComparison.OrdinalIgnoreCase);

    public static string ProcessName =>
        Path.GetFileNameWithoutExtension(Environment.ProcessPath ?? AppDomain.CurrentDomain.FriendlyName);
}
