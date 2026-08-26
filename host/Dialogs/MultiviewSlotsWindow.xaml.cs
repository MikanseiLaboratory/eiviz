using System.Windows;
using System.Windows.Controls;

namespace Eiviz.Host.Dialogs;

public partial class MultiviewSlotsWindow : Window
{
    private readonly IList<MvSlot> _target;
    private readonly Session _session;
    private readonly List<MvSlot> _tiles;
    private readonly int _maxTiles;
    private bool _suppress;

    public MultiviewSlotsWindow(MixingUnitEntry unit, Session session)
        : this(session, unit.MultiviewTiles, 16)
    {
    }

    public MultiviewSlotsWindow(Session session, IList<MvSlot> target, int maxTiles)
    {
        InitializeComponent();
        _session = session;
        _target = target;
        _maxTiles = maxTiles;
        _tiles = target.Select(Clone).ToList();
        Rebuild();
    }

    private void Add_Click(object sender, RoutedEventArgs e)
    {
        if (_tiles.Count >= _maxTiles)
        {
            MessageBox.Show(this, $"This mosaic can have at most {_maxTiles} tiles.");
            return;
        }
        _tiles.Add(new MvSlot());
        Rebuild();
    }

    private void Rebuild()
    {
        SlotRows.Children.Clear();
        _suppress = true;
        for (var i = 0; i < _tiles.Count; i++)
        {
            var index = i;
            var tile = _tiles[i];
            var box = new Border
            {
                BorderBrush = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
                BorderThickness = new Thickness(1),
                Padding = new Thickness(8),
                Margin = new Thickness(0, 0, 0, 8)
            };
            var stack = new StackPanel();
            var header = new DockPanel { Margin = new Thickness(0, 0, 0, 6) };
            header.Children.Add(new TextBlock { Text = $"Tile {i + 1}", FontWeight = FontWeights.Bold, VerticalAlignment = VerticalAlignment.Center });
            var remove = new Button { Content = "−", Width = 28, HorizontalAlignment = HorizontalAlignment.Right };
            remove.Click += (_, _) =>
            {
                _tiles.RemoveAt(index);
                Rebuild();
            };
            DockPanel.SetDock(remove, Dock.Right);
            header.Children.Add(remove);
            stack.Children.Add(header);

            var kinds = new WrapPanel();
            AddRadio(kinds, tile, MvSlotKind.None, "None", index);
            AddRadio(kinds, tile, MvSlotKind.Scene, "Scene", index);
            AddRadio(kinds, tile, MvSlotKind.Input, "Input", index);
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
            box.Child = stack;
            SlotRows.Children.Add(box);
        }
        _suppress = false;
    }

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
        switch (tile.Kind)
        {
            case MvSlotKind.Scene:
                choices.AddRange(_session.Scenes.Select(item => new SlotChoice(item.Name, item.GpuId)));
                break;
            case MvSlotKind.Input:
                choices.AddRange(_session.Inputs.Select(item => new SlotChoice(item.Name, item.Id)));
                break;
            case MvSlotKind.MuPreview:
            case MvSlotKind.MuProgram:
                choices.AddRange(_session.Units.Select(item => new SlotChoice(item.Name, item.Id)));
                break;
        }
        box.ItemsSource = choices;
        box.DisplayMemberPath = "Label";
        box.IsEnabled = choices.Count > 0;
        box.SelectedItem = choices.FirstOrDefault(item => item.Id == tile.SourceId) ?? choices.FirstOrDefault();
        if (box.SelectedItem is SlotChoice selected)
            tile.SourceId = selected.Id;
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        _target.Clear();
        foreach (var tile in _tiles)
            _target.Add(tile);
        DialogResult = true;
    }

    private static MvSlot Clone(MvSlot slot) => new() { Kind = slot.Kind, SourceId = slot.SourceId };

    private sealed record SlotChoice(string Label, ulong Id);
}
