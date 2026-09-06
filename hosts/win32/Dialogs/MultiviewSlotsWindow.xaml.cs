using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Dialogs;

public partial class MultiviewSlotsWindow : Window
{
    private readonly MultiviewLayout _layout;
    private readonly Session _session;
    private readonly List<MvSlot> _tiles;
    private MultiviewTemplate _template;
    private int _selectedTile;
    private bool _suppress;

    public MultiviewSlotsWindow(Session session, MultiviewLayout layout)
    {
        InitializeComponent();
        _session = session;
        _layout = layout;
        _template = layout.Template;
        layout.Template = _template;
        layout.EnsureTiles();
        _tiles = layout.Tiles.Select(Clone).ToList();
        Rebuild();
    }

    private void Rebuild()
    {
        SlotRows.Children.Clear();
        _suppress = true;
        EnsureTileCount();
        if (_selectedTile >= _tiles.Count)
            _selectedTile = Math.Max(0, _tiles.Count - 1);
        SlotRows.Children.Add(LayoutChrome());
        SlotRows.Children.Add(Workspace());
        SlotRows.Children.Add(TemplateRow());
        _suppress = false;
    }

    private void EnsureTileCount()
    {
        var want = MultiviewGeometry.TileCount(_template);
        while (_tiles.Count < want)
            _tiles.Add(new MvSlot());
        while (_tiles.Count > want)
            _tiles.RemoveAt(_tiles.Count - 1);
    }

    private Border LayoutChrome()
    {
        var box = Frame();
        box.Padding = new Thickness(8, 6, 8, 6);
        box.Margin = new Thickness(0, 0, 0, 8);
        var row = new WrapPanel { VerticalAlignment = VerticalAlignment.Center };
        row.Children.Add(new TextBlock
        {
            Text = Loc.T("mv.labelPosition"),
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 0, 8, 0)
        });
        var anchor = new ComboBox { Width = 88, Height = 24, Margin = new Thickness(0, 0, 12, 0) };
        anchor.Items.Add(new ComboBoxItem { Content = Loc.T("mv.bottom"), Tag = "Bottom" });
        anchor.Items.Add(new ComboBoxItem { Content = Loc.T("mv.top"), Tag = "Top" });
        var currentAnchor = _layout.ResolvedLabelAnchor(_session.Settings) == MvLabelAnchor.Top ? "Top" : "Bottom";
        foreach (ComboBoxItem item in anchor.Items)
        {
            if (Equals(item.Tag, currentAnchor))
                anchor.SelectedItem = item;
        }
        anchor.SelectionChanged += (_, _) =>
        {
            if (_suppress || anchor.SelectedItem is not ComboBoxItem item || item.Tag is not string tag)
                return;
            _layout.LabelAnchor = tag == "Top" ? MvLabelAnchor.Top : MvLabelAnchor.Bottom;
            _layout.PushLabelStyle(_session.Settings);
        };
        row.Children.Add(anchor);
        row.Children.Add(new TextBlock
        {
            Text = Loc.T("settings.mvLabelSize"),
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 0, 8, 0)
        });
        var size = new TextBox
        {
            Width = 48,
            Height = 24,
            Text = _layout.ResolvedLabelSize(_session.Settings).ToString("0.##"),
            Margin = new Thickness(0, 0, 4, 0),
            VerticalContentAlignment = VerticalAlignment.Center
        };
        size.LostFocus += (_, _) =>
        {
            if (float.TryParse(size.Text, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.CurrentCulture, out var value)
                || float.TryParse(size.Text, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out value))
            {
                _layout.LabelSize = Math.Clamp(value, 1f, 200f);
                size.Text = _layout.LabelSize.Value.ToString("0.##");
                _layout.PushLabelStyle(_session.Settings);
            }
            else
            {
                size.Text = _layout.ResolvedLabelSize(_session.Settings).ToString("0.##");
            }
        };
        row.Children.Add(size);
        var unit = new ComboBox { Width = 56, Height = 24 };
        unit.Items.Add(new ComboBoxItem { Content = "px", Tag = "Px" });
        unit.Items.Add(new ComboBoxItem { Content = "%", Tag = "Percent" });
        var currentUnit = _layout.ResolvedLabelUnit(_session.Settings) == MvLabelUnit.Percent ? "Percent" : "Px";
        foreach (ComboBoxItem item in unit.Items)
        {
            if (Equals(item.Tag, currentUnit))
                unit.SelectedItem = item;
        }
        unit.SelectionChanged += (_, _) =>
        {
            if (_suppress || unit.SelectedItem is not ComboBoxItem item || item.Tag is not string tag)
                return;
            _layout.LabelUnit = tag == "Percent" ? MvLabelUnit.Percent : MvLabelUnit.Px;
            _layout.PushLabelStyle(_session.Settings);
        };
        row.Children.Add(unit);
        box.Child = row;
        return box;
    }

    private Border Workspace()
    {
        var box = Frame();
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(16) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var preview = new StackPanel();
        preview.Children.Add(new TextBlock { Text = "Template", FontWeight = FontWeights.Bold, Margin = new Thickness(0, 0, 0, 6) });
        var mosaic = NumberedMosaic(_template, 320, 180, _selectedTile, index =>
        {
            if (_suppress)
                return;
            _selectedTile = index;
            Rebuild();
        });
        preview.Children.Add(new Border
        {
            HorizontalAlignment = HorizontalAlignment.Left,
            BorderBrush = new SolidColorBrush(Color.FromRgb(0x44, 0x44, 0x44)),
            BorderThickness = new Thickness(1),
            Child = mosaic
        });
        Grid.SetColumn(preview, 0);
        var editor = TileEditor(_selectedTile, _tiles[_selectedTile]);
        Grid.SetColumn(editor, 2);
        grid.Children.Add(preview);
        grid.Children.Add(editor);
        box.Child = grid;
        return box;
    }

    private Border TemplateRow()
    {
        var box = Frame();
        var stack = new StackPanel();
        foreach (var (title, items) in MultiviewGeometry.Groups)
        {
            stack.Children.Add(new TextBlock
            {
                Text = title,
                Foreground = new SolidColorBrush(Color.FromRgb(0xAA, 0xAA, 0xAA)),
                Margin = new Thickness(0, 0, 0, 4)
            });
            var row = new WrapPanel();
            foreach (var item in items)
                row.Children.Add(TemplateCard(item));
            stack.Children.Add(row);
        }
        box.Child = stack;
        return box;
    }

    private UIElement TemplateCard(MultiviewTemplate template)
    {
        var selected = template == _template;
        const double width = 112;
        const double height = 63;
        var canvas = NumberedMosaic(template, width, height, selected ? _selectedTile : -1, null);
        var card = new StackPanel { Width = width, Margin = new Thickness(0, 0, 8, 6), Cursor = Cursors.Hand };
        card.Children.Add(new Border
        {
            BorderBrush = selected
                ? Brushes.White
                : new SolidColorBrush(Color.FromRgb(0x44, 0x44, 0x44)),
            BorderThickness = new Thickness(selected ? 2 : 1),
            Child = canvas
        });
        card.Children.Add(new TextBlock
        {
            Text = MultiviewGeometry.Title(template),
            FontSize = 10,
            Margin = new Thickness(0, 2, 0, 0),
            TextAlignment = TextAlignment.Center
        });
        card.MouseLeftButtonUp += (_, _) =>
        {
            if (_suppress || template == _template)
                return;
            _template = template;
            Rebuild();
        };
        return card;
    }

    private static Canvas NumberedMosaic(MultiviewTemplate template, double width, double height, int selected, Action<int>? onPick)
    {
        var canvas = new Canvas
        {
            Width = width,
            Height = height,
            Background = Brushes.Black
        };
        var panes = MultiviewGeometry.Panes(template);
        for (var i = 0; i < panes.Count; i++)
        {
            var pane = panes[i];
            var index = i;
            var w = Math.Max(1, pane.Width * width - 1);
            var h = Math.Max(1, pane.Height * height - 1);
            var cell = new Grid
            {
                Width = w,
                Height = h,
                Cursor = onPick is null ? Cursors.Arrow : Cursors.Hand
            };
            cell.Children.Add(new Rectangle
            {
                Fill = new SolidColorBrush(index == selected
                    ? Color.FromRgb(0x6E, 0x6E, 0x6E)
                    : Color.FromRgb(0x4A, 0x4A, 0x4A)),
                Stroke = Brushes.Black,
                StrokeThickness = 1
            });
            cell.Children.Add(new TextBlock
            {
                Text = (index + 1).ToString(),
                Foreground = Brushes.White,
                FontWeight = FontWeights.Bold,
                FontSize = Math.Clamp(Math.Min(w, h) * 0.42, 7, 18),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center
            });
            if (onPick is not null)
                cell.MouseLeftButtonUp += (_, _) => onPick(index);
            Canvas.SetLeft(cell, pane.X * width);
            Canvas.SetTop(cell, pane.Y * height);
            canvas.Children.Add(cell);
        }
        return canvas;
    }

    private Border TileEditor(int index, MvSlot tile)
    {
        var box = new Border();
        var stack = new StackPanel();
        stack.Children.Add(new TextBlock { Text = $"Window {index + 1}", FontWeight = FontWeights.Bold, Margin = new Thickness(0, 0, 0, 6) });
        var kinds = new WrapPanel();
        AddRadio(kinds, tile, MvSlotKind.None, "None", index);
        AddRadio(kinds, tile, MvSlotKind.Input, "Input", index);
        AddRadio(kinds, tile, MvSlotKind.Scene, "Scene", index);
        AddRadio(kinds, tile, MvSlotKind.MuPreview, "MU PRV", index);
        AddRadio(kinds, tile, MvSlotKind.MuProgram, "MU PGM", index);
        stack.Children.Add(kinds);
        var pick = new ComboBox { Margin = new Thickness(0, 8, 0, 0) };
        FillPick(pick, tile);
        pick.SelectionChanged += (_, _) =>
        {
            if (pick.SelectedItem is SlotChoice choice)
                tile.SourceId = choice.Id;
        };
        stack.Children.Add(pick);
        stack.Children.Add(LabelEditor(
            $"label-tile-{index}",
            tile.LabelFollow,
            tile.Label,
            follow =>
            {
                tile.LabelFollow = follow;
                Rebuild();
            },
            text => tile.Label = text));
        box.Child = stack;
        return box;
    }

    private UIElement LabelEditor(string group, bool follow, string custom, Action<bool> setFollow, Action<string> setCustom)
    {
        var stack = new StackPanel { Margin = new Thickness(0, 8, 0, 0) };
        var modes = new WrapPanel();
        var followRadio = new RadioButton
        {
            Content = "Follow",
            GroupName = group,
            IsChecked = follow,
            Foreground = System.Windows.Media.Brushes.White,
            Margin = new Thickness(0, 0, 12, 4)
        };
        var customRadio = new RadioButton
        {
            Content = "Custom",
            GroupName = group,
            IsChecked = !follow,
            Foreground = System.Windows.Media.Brushes.White,
            Margin = new Thickness(0, 0, 12, 4)
        };
        followRadio.Checked += (_, _) =>
        {
            if (_suppress)
                return;
            setFollow(true);
        };
        customRadio.Checked += (_, _) =>
        {
            if (_suppress)
                return;
            setFollow(false);
        };
        modes.Children.Add(followRadio);
        modes.Children.Add(customRadio);
        stack.Children.Add(modes);
        var box = new TextBox
        {
            Text = custom,
            IsEnabled = !follow,
            Margin = new Thickness(0, 4, 0, 0)
        };
        box.TextChanged += (_, _) => setCustom(box.Text);
        stack.Children.Add(box);
        return stack;
    }

    private static Border Frame() => new()
    {
        BorderBrush = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
        BorderThickness = new Thickness(1),
        Padding = new Thickness(8),
        Margin = new Thickness(0, 0, 0, 8)
    };

    private void AddRadio(WrapPanel panel, MvSlot tile, MvSlotKind kind, string label, int index)
    {
        var radio = new RadioButton
        {
            Content = label,
            GroupName = $"mv-{index}",
            IsChecked = tile.Kind == kind,
            Foreground = System.Windows.Media.Brushes.White,
            Margin = new Thickness(0, 0, 12, 4)
        };
        radio.Checked += (_, _) =>
        {
            if (_suppress)
                return;
            tile.Kind = kind;
            Rebuild();
        };
        panel.Children.Add(radio);
    }

    private void FillPick(ComboBox box, MvSlot tile)
    {
        var choices = new List<SlotChoice>();
        if (tile.Kind == MvSlotKind.Scene)
            choices.AddRange(_session.Scenes.Select(item => new SlotChoice(item.Name, item.GpuId)));
        else if (tile.Kind == MvSlotKind.Input)
            choices.AddRange(_session.Inputs.Select(item => new SlotChoice(item.Name, item.Id)));
        else if (tile.Kind is MvSlotKind.MuPreview or MvSlotKind.MuProgram)
            choices.AddRange(_session.Units.Select(item => new SlotChoice(item.Name, item.Id)));
        box.ItemsSource = choices;
        box.DisplayMemberPath = "Label";
        box.IsEnabled = choices.Count > 0;
        box.SelectedItem = choices.FirstOrDefault(item => item.Id == tile.SourceId) ?? choices.FirstOrDefault();
        if (box.SelectedItem is SlotChoice selected)
            tile.SourceId = selected.Id;
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        _layout.Template = _template;
        _layout.Tiles.Clear();
        foreach (var tile in _tiles)
            _layout.Tiles.Add(tile);
        _layout.EnsureTiles();
        _layout.PreviewUnitId = _layout.Tiles.FirstOrDefault(tile => tile.Kind == MvSlotKind.MuPreview)?.SourceId
            ?? _layout.PreviewUnitId;
        _layout.ProgramUnitId = _layout.Tiles.FirstOrDefault(tile => tile.Kind == MvSlotKind.MuProgram)?.SourceId
            ?? _layout.ProgramUnitId;
        DialogResult = true;
    }

    private static MvSlot Clone(MvSlot slot) => new()
    {
        Kind = slot.Kind,
        SourceId = slot.SourceId,
        LabelFollow = slot.LabelFollow,
        Label = slot.Label ?? ""
    };

    private sealed record SlotChoice(string Label, ulong Id);
}
