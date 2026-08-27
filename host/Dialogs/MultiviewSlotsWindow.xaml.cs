using System.Windows;
using System.Windows.Controls;

namespace Eiviz.Host.Dialogs;

public partial class MultiviewSlotsWindow : Window
{
    private readonly MultiviewLayout _layout;
    private readonly Session _session;
    private readonly List<MvSlot> _tiles;
    private ulong _previewUnit;
    private ulong _programUnit;
    private bool _suppress;

    public MultiviewSlotsWindow(Session session, MultiviewLayout layout)
    {
        InitializeComponent();
        _session = session;
        _layout = layout;
        layout.EnsureTiles();
        _previewUnit = layout.PreviewUnitId;
        _programUnit = layout.ProgramUnitId;
        _tiles = layout.Tiles.Select(Clone).ToList();
        Rebuild();
    }

    private void Rebuild()
    {
        SlotRows.Children.Clear();
        _suppress = true;
        SlotRows.Children.Add(UnitRow("PRV (top left)", true));
        SlotRows.Children.Add(UnitRow("PGM (top right)", false));
        for (var i = 0; i < 8; i++)
            SlotRows.Children.Add(TileRow(i, _tiles[i]));
        _suppress = false;
    }

    private Border UnitRow(string title, bool preview)
    {
        var box = Frame();
        var stack = new StackPanel();
        stack.Children.Add(new TextBlock { Text = title, FontWeight = FontWeights.Bold, Margin = new Thickness(0, 0, 0, 6) });
        var pick = new ComboBox();
        pick.ItemsSource = _session.Units;
        pick.DisplayMemberPath = "Name";
        pick.SelectedValuePath = "Id";
        pick.SelectedValue = preview ? _previewUnit : _programUnit;
        if (pick.SelectedItem is MixingUnitEntry selected)
        {
            if (preview)
                _previewUnit = selected.Id;
            else
                _programUnit = selected.Id;
        }
        pick.SelectionChanged += (_, _) =>
        {
            if (pick.SelectedItem is MixingUnitEntry unit)
            {
                if (preview)
                    _previewUnit = unit.Id;
                else
                    _programUnit = unit.Id;
            }
        };
        stack.Children.Add(pick);
        box.Child = stack;
        return box;
    }

    private Border TileRow(int index, MvSlot tile)
    {
        var box = Frame();
        var stack = new StackPanel();
        stack.Children.Add(new TextBlock { Text = $"Window {index + 1}", FontWeight = FontWeights.Bold, Margin = new Thickness(0, 0, 0, 6) });
        var kinds = new WrapPanel();
        AddRadio(kinds, tile, MvSlotKind.None, "None", index);
        AddRadio(kinds, tile, MvSlotKind.Input, "Input", index);
        AddRadio(kinds, tile, MvSlotKind.Scene, "Scene", index);
        stack.Children.Add(kinds);
        var pick = new ComboBox { Margin = new Thickness(0, 8, 0, 0) };
        FillPick(pick, tile);
        pick.SelectionChanged += (_, _) =>
        {
            if (pick.SelectedItem is SlotChoice choice)
                tile.SourceId = choice.Id;
        };
        stack.Children.Add(pick);
        box.Child = stack;
        return box;
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
        box.ItemsSource = choices;
        box.DisplayMemberPath = "Label";
        box.IsEnabled = choices.Count > 0;
        box.SelectedItem = choices.FirstOrDefault(item => item.Id == tile.SourceId) ?? choices.FirstOrDefault();
        if (box.SelectedItem is SlotChoice selected)
            tile.SourceId = selected.Id;
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        _layout.PreviewUnitId = _previewUnit;
        _layout.ProgramUnitId = _programUnit;
        _layout.Tiles.Clear();
        foreach (var tile in _tiles)
            _layout.Tiles.Add(tile);
        _layout.EnsureTiles();
        DialogResult = true;
    }

    private static MvSlot Clone(MvSlot slot) => new() { Kind = slot.Kind, SourceId = slot.SourceId };

    private sealed record SlotChoice(string Label, ulong Id);
}
