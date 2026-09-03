namespace Eiviz.Host;

internal static class TagCatalog
{
    public static string? Normalize(string? name)
    {
        var trimmed = name?.Trim();
        return string.IsNullOrEmpty(trimmed) ? null : trimmed;
    }

    public static List<string> NormalizeList(IEnumerable<string>? tags)
    {
        var result = new List<string>();
        if (tags is null)
            return result;
        foreach (var tag in tags)
        {
            if (Normalize(tag) is not { } name)
                continue;
            if (result.Exists(item => string.Equals(item, name, StringComparison.Ordinal)))
                continue;
            result.Add(name);
        }
        return result;
    }

    public static void MergeInto(List<string> catalog, IEnumerable<string>? tags)
    {
        foreach (var tag in NormalizeList(tags))
        {
            if (!catalog.Exists(item => string.Equals(item, tag, StringComparison.Ordinal)))
                catalog.Add(tag);
        }
    }

    public static bool TryAdd(List<string> catalog, string? name, out string normalized)
    {
        var value = Normalize(name) ?? "";
        normalized = value;
        if (value.Length == 0)
            return false;
        if (catalog.Exists(item => string.Equals(item, value, StringComparison.Ordinal)))
            return false;
        catalog.Add(value);
        return true;
    }

    public static bool Rename(List<string> catalog, IEnumerable<List<string>> owners, string current, string? next)
    {
        if (Normalize(next) is not { } name)
            return false;
        var index = catalog.FindIndex(item => string.Equals(item, current, StringComparison.Ordinal));
        if (index < 0)
            return false;
        if (!string.Equals(name, current, StringComparison.Ordinal)
            && catalog.Exists(item => string.Equals(item, name, StringComparison.Ordinal)))
            return false;
        catalog[index] = name;
        foreach (var owner in owners)
        {
            for (var i = 0; i < owner.Count; i++)
            {
                if (string.Equals(owner[i], current, StringComparison.Ordinal))
                    owner[i] = name;
            }
        }
        return true;
    }

    public static void Remove(List<string> catalog, IEnumerable<List<string>> owners, string name)
    {
        catalog.RemoveAll(item => string.Equals(item, name, StringComparison.Ordinal));
        foreach (var owner in owners)
            owner.RemoveAll(item => string.Equals(item, name, StringComparison.Ordinal));
    }

    public static bool Contains(IEnumerable<string> tags, string name) =>
        tags.Any(item => string.Equals(item, name, StringComparison.Ordinal));

    public static void Replace(List<string> target, IEnumerable<string>? tags)
    {
        target.Clear();
        target.AddRange(NormalizeList(tags));
    }
}

internal enum ListFilterMode
{
    All,
    Tag,
    Kind
}

internal sealed class ListFilter
{
    public ListFilterMode Mode { get; init; } = ListFilterMode.All;
    public string? Tag { get; init; }
    public InputKind? Kind { get; init; }

    public static ListFilter All { get; } = new();

    public static ListFilter ForTag(string tag) => new() { Mode = ListFilterMode.Tag, Tag = tag };

    public static ListFilter ForKind(InputKind kind) => new() { Mode = ListFilterMode.Kind, Kind = kind };

    public bool MatchesInput(InputEntry input) => Mode switch
    {
        ListFilterMode.Tag => Tag is { } tag && TagCatalog.Contains(input.Tags, tag),
        ListFilterMode.Kind => Kind is { } kind && InputKindNames.SameCategory(input.Kind, kind),
        _ => true
    };

    public bool MatchesScene(SceneEntry scene) => Mode switch
    {
        ListFilterMode.Tag => Tag is { } tag && TagCatalog.Contains(scene.Tags, tag),
        _ => true
    };

    public bool SameAs(ListFilter other) =>
        Mode == other.Mode
        && string.Equals(Tag, other.Tag, StringComparison.Ordinal)
        && Kind == other.Kind;
}
