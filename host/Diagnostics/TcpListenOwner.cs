using System.Diagnostics;
using System.Runtime.InteropServices;

namespace Eiviz.Host;

internal static class TcpListenOwner
{
    private const int AfInet = 2;
    private const int AfInet6 = 23;
    private const int TcpTableOwnerPidListener = 3;
    private const uint ErrorInsufficientBuffer = 122;

    public static string? TryGetName(uint port)
    {
        if (port is 0 or > 65535)
            return null;
        try
        {
            foreach (var pid in EnumeratePids((ushort)port))
            {
                if (pid == 0 || pid == Environment.ProcessId)
                    continue;
                try
                {
                    using var process = Process.GetProcessById(pid);
                    var name = FriendlyName(process);
                    if (!string.IsNullOrWhiteSpace(name))
                        return name;
                }
                catch (ArgumentException)
                {
                }
                catch (InvalidOperationException)
                {
                }
            }
        }
        catch
        {
            // Port ownership is best-effort for the warning dialog.
        }

        return null;
    }

    private static IEnumerable<int> EnumeratePids(ushort port)
    {
        foreach (var pid in QueryTable(AfInet, port, ipv6: false))
            yield return pid;
        foreach (var pid in QueryTable(AfInet6, port, ipv6: true))
            yield return pid;
    }

    private static List<int> QueryTable(int family, ushort port, bool ipv6)
    {
        var result = new List<int>();
        var size = 0;
        var err = GetExtendedTcpTable(nint.Zero, ref size, true, family, TcpTableOwnerPidListener, 0);
        if (err != ErrorInsufficientBuffer || size <= 0)
            return result;
        var buffer = Marshal.AllocHGlobal(size);
        try
        {
            err = GetExtendedTcpTable(buffer, ref size, true, family, TcpTableOwnerPidListener, 0);
            if (err != 0)
                return result;
            var count = Marshal.ReadInt32(buffer);
            var rowSize = ipv6 ? 56 : 24;
            var first = nint.Add(buffer, 4);
            for (var i = 0; i < count; i++)
            {
                var row = nint.Add(first, i * rowSize);
                if (ReadHostPort(row, ipv6 ? 20 : 8) != port)
                    continue;
                result.Add(Marshal.ReadInt32(row, ipv6 ? 52 : 20));
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }

        return result;
    }

    private static int ReadHostPort(nint row, int offset)
    {
        var raw = (ushort)(Marshal.ReadInt32(row, offset) & 0xFFFF);
        return (raw >> 8) | ((raw & 0xFF) << 8);
    }

    private static string FriendlyName(Process process)
    {
        var raw = process.ProcessName;
        if (string.IsNullOrWhiteSpace(raw))
            return process.Id.ToString();
        return raw.ToLowerInvariant() switch
        {
            "vmix" or "vmix64" or "vmix64bit" => "vMix",
            "httpd" or "apache" or "apache2" => "Apache HTTP Server",
            "nginx" => "nginx",
            "w3wp" or "iisexpress" => "IIS",
            _ => raw
        };
    }

    [DllImport("iphlpapi.dll", SetLastError = true)]
    private static extern uint GetExtendedTcpTable(
        nint table,
        ref int size,
        bool order,
        int ipVersion,
        int tableClass,
        uint reserved);
}
