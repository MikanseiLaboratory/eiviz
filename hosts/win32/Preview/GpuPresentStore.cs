using System.IO;
using System.Text.Json;

namespace Eiviz.Host.Preview;

internal static class GpuPresentStore
{
    private static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        WriteIndented = true
    };

    public static int? ObservedCeiling { get; private set; }

    public static void Load()
    {
        try
        {
            var path = StorePath();
            if (!File.Exists(path))
                return;
            var dto = JsonSerializer.Deserialize<Dto>(File.ReadAllText(path), Json);
            if (dto is { ObservedCeiling: >= 2 and <= 16 })
                ObservedCeiling = dto.ObservedCeiling;
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
        }
    }

    public static void Save(int ceiling)
    {
        if (ceiling < 2)
            return;
        ObservedCeiling = ceiling;
        try
        {
            var path = StorePath();
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, JsonSerializer.Serialize(new Dto { ObservedCeiling = ceiling }, Json));
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
        }
    }

    private static string StorePath() =>
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "eiviz", "gpu-present.json");

    private sealed class Dto
    {
        public int ObservedCeiling { get; set; }
    }
}
