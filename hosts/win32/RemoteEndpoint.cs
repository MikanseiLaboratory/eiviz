using System.Globalization;

namespace Eiviz.Host;

internal static class RemoteEndpoint
{
    public const string DefaultHost = "127.0.0.1";
    public const uint DefaultPort = 9400;

    public static bool TryParse(string? text, out string host, out uint port)
    {
        host = DefaultHost;
        port = DefaultPort;
        if (string.IsNullOrWhiteSpace(text))
            return false;
        var raw = text.Trim();
        if (!raw.Contains("://", StringComparison.Ordinal))
            raw = "ws://" + raw;
        if (!Uri.TryCreate(raw, UriKind.Absolute, out var uri) || string.IsNullOrEmpty(uri.Host))
            return false;
        host = uri.Host;
        port = uri.Port > 0 ? (uint)uri.Port : DefaultPort;
        return true;
    }

    public static (string Host, uint Port) Split(string? text)
    {
        TryParse(text, out var host, out var port);
        return (host, port);
    }

    public static string Format(string host, uint port)
    {
        host = host.Trim();
        if (host.Contains(':', StringComparison.Ordinal) && !host.StartsWith('['))
            return $"ws://[{host}]:{port}";
        return $"ws://{host}:{port}";
    }

    public static string Display(string? text)
    {
        var (host, port) = Split(text);
        if (host.Contains(':', StringComparison.Ordinal) && !host.StartsWith('['))
            return $"[{host}]:{port}";
        return $"{host}:{port}";
    }

    public static bool TryCompose(string host, string portText, out string url)
    {
        url = "";
        host = host.Trim();
        if (string.IsNullOrEmpty(host))
            return false;
        if (!uint.TryParse(portText.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var port)
            || port is 0 or > 65535)
            return false;
        url = Format(host, port);
        return true;
    }
}
