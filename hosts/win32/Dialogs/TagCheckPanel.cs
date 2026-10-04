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
    private readonly Action? _changed;

    public TagCheckPanel(WrapPanel panel, List<string> catalog, IEnumerable<string>? selected, Window owner, Action? changed = null)
    {
        _panel = panel;
        _catalog = catalog;
        _selected = new HashSet<string>(TagCatalog.NormalizeList(selected), StringComparer.Ordinal);
        _owner = owner;
        _changed = changed;
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
        _changed?.Invoke();
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
            box.Checked += (_, _) =>
            {
                _selected.Add(tag);
                _changed?.Invoke();
            };
            box.Unchecked += (_, _) =>
            {
                _selected.Remove(tag);
                _changed?.Invoke();
            };
            _panel.Children.Add(box);
        }
    }
}
