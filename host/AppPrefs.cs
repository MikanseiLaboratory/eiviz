using System.Globalization;
using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Eiviz.Host;

internal enum AppLanguage
{
    En,
    Ja
}

internal enum AppThemeMode
{
    Dark,
    Light,
    System
}

internal sealed class AppPrefs
{
    private const int RecentCap = 12;
    private const int InputCap = 24;
    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) }
    };

    public AppLanguage Language { get; set; } = DefaultLanguage();
    public AppThemeMode Theme { get; set; } = AppThemeMode.Dark;
    public List<string> RecentSessions { get; set; } = [];
    public List<string> RecentStills { get; set; } = [];
    public List<string> RecentVideos { get; set; } = [];

    public static AppPrefs Current { get; private set; } = Load();

    public static string StorePath =>
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "eiviz", "prefs.json");

    public static AppPrefs Load()
    {
        try
        {
            if (File.Exists(StorePath))
            {
                var loaded = JsonSerializer.Deserialize<AppPrefs>(File.ReadAllText(StorePath), Json);
                if (loaded is not null)
                {
                    loaded.RecentSessions = Clean(loaded.RecentSessions, RecentCap);
                    loaded.RecentStills = Clean(loaded.RecentStills, InputCap);
                    loaded.RecentVideos = Clean(loaded.RecentVideos, InputCap);
                    return loaded;
                }
            }
        }
        catch
        {
            // Keep defaults when the prefs file is missing or unreadable.
        }
        return new AppPrefs();
    }

    public void Save()
    {
        var dir = Path.GetDirectoryName(StorePath);
        if (!string.IsNullOrEmpty(dir))
            Directory.CreateDirectory(dir);
        File.WriteAllText(StorePath, JsonSerializer.Serialize(this, Json));
    }

    public void RememberSession(string path)
    {
        Remember(RecentSessions, path, RecentCap);
        Save();
    }

    public void RememberStill(string path)
    {
        Remember(RecentStills, path, InputCap);
        Save();
    }

    public void RememberVideo(string path)
    {
        Remember(RecentVideos, path, InputCap);
        Save();
    }

    public IEnumerable<string> ExistingSessions()
    {
        var keep = RecentSessions.Where(File.Exists).ToList();
        if (keep.Count != RecentSessions.Count)
        {
            RecentSessions = keep;
            Save();
        }
        return keep;
    }

    private static void Remember(List<string> list, string path, int cap)
    {
        if (string.IsNullOrWhiteSpace(path))
            return;
        list.Remove(path);
        list.Insert(0, path);
        if (list.Count > cap)
            list.RemoveRange(cap, list.Count - cap);
    }

    private static List<string> Clean(List<string>? list, int cap) =>
        (list ?? []).Where(item => !string.IsNullOrWhiteSpace(item)).Distinct().Take(cap).ToList();

    private static AppLanguage DefaultLanguage()
    {
        var name = CultureInfo.CurrentUICulture.TwoLetterISOLanguageName;
        return name == "ja" ? AppLanguage.Ja : AppLanguage.En;
    }
}
