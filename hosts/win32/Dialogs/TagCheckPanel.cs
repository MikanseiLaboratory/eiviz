using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Dialogs;

internal sealed class TagCheckPanel
{
    private readonly WrapPanel _panel;
    private readonly List<string> _catalog;
    private readonly HashSet<string> _selected;
    private readonly Window _owner;

    public TagCheckPanel(WrapPanel panel, List<string> catalog, IEnumerable<string>? selected, Window owner)
    {
        _panel = panel;
        _catalog = catalog;
        _selected = new HashSet<string>(TagCatalog.NormalizeList(selected), StringComparer.Ordinal);
        _owner = owner;
        Rebuild();
    }

    public IReadOnlyList<string> Selected =>
        _catalog.Where(_selected.Contains).ToList();

    public void PromptAdd()
    {
        if (!TextPromptWindow.TryPrompt(_owner, Loc.T("tag.add"), Loc.T("tag.name"), "", out var name))
            return;
        if (!TagCatalog.TryAdd(_catalog, name, out var normalized))
        {
            if (normalized.Length == 0)
                return;
        }
        _selected.Add(normalized);
        Rebuild();
    }

    private void Rebuild()
    {
        _panel.Children.Clear();
        foreach (var tag in _catalog)
        {
            var box = new CheckBox
            {
                Content = tag,
                IsChecked = _selected.Contains(tag),
                Foreground = Brushes.WhiteSmoke,
                Margin = new Thickness(0, 0, 8, 4),
                VerticalAlignment = VerticalAlignment.Center
            };
            box.Checked += (_, _) => _selected.Add(tag);
            box.Unchecked += (_, _) => _selected.Remove(tag);
            _panel.Children.Add(box);
        }
    }
}
