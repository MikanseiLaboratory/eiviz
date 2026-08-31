using System.Reflection;

namespace Eiviz.Host;

internal static class HostVersion
{
    public static string Display
    {
        get
        {
            var info = Assembly.GetExecutingAssembly()
                .GetCustomAttribute<AssemblyInformationalVersionAttribute>()
                ?.InformationalVersion;
            if (!string.IsNullOrWhiteSpace(info))
            {
                var plus = info.IndexOf('+');
                return plus < 0 ? info.Trim() : info[..plus].Trim();
            }
            return Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "0.2.0";
        }
    }
}
