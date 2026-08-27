using System.Windows;
using System.Windows.Controls;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

public partial class SettingsWindow : Window
{
    private readonly Session _session;
    private ulong _nextOutputId;

    public SettingsWindow(Session session)
    {
        InitializeComponent();
        _session = session;
        _nextOutputId = session.NextOutputId;
        Settings = new SessionSettings
        {
            MasterFpsNum = session.Settings.MasterFpsNum,
            MasterFpsDen = session.Settings.MasterFpsDen,
            DefaultWidth = session.Settings.DefaultWidth,
            DefaultHeight = session.Settings.DefaultHeight,
            Theme = session.Settings.Theme,
            DefaultMultiviewUnitId = session.Settings.DefaultMultiviewUnitId,
            FrameBufferFrames = session.Settings.FrameBufferFrames,
            InternalColorFormat = session.Settings.InternalColorFormat
        };
        foreach (var output in session.Outputs)
        {
            Outputs.Add(Clone(output));
            _nextOutputId = Math.Max(_nextOutputId, output.Id + 1);
        }
        SelectTag(FpsBox, $"{Settings.MasterFpsNum}/{Settings.MasterFpsDen}");
        SelectTag(SizeBox, $"{Settings.DefaultWidth}x{Settings.DefaultHeight}");
        SelectTag(BufferBox, Settings.FrameBufferFrames.ToString());
        SelectTag(ColorFormatBox, Settings.InternalColorFormat == InternalColorFormat.Bgra ? "bgra" : "uyvy");
        MvUnitBox.ItemsSource = session.Units;
        MvUnitBox.SelectedItem = session.Units.FirstOrDefault(item => item.Id == Settings.DefaultMultiviewUnitId)
            ?? session.Units.FirstOrDefault();
        RebuildOutputs();
        RebuildLayouts();
    }

    public SessionSettings Settings { get; }
    public List<OutputEntry> Outputs { get; } = [];
    private bool _suppressOutputs;

    private void CategoryList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (DisplayPanel is null || MultiviewPanel is null)
            return;
        var index = CategoryList.SelectedIndex;
        DisplayPanel.Visibility = index == 0 ? Visibility.Visible : Visibility.Collapsed;
        OutputPanel.Visibility = index == 1 ? Visibility.Visible : Visibility.Collapsed;
        MultiviewPanel.Visibility = index == 2 ? Visibility.Visible : Visibility.Collapsed;
        AboutPanel.Visibility = index == 3 ? Visibility.Visible : Visibility.Collapsed;
    }

    private void Default_Click(object sender, RoutedEventArgs e)
    {
        SelectTag(FpsBox, "60000/1001");
        SelectTag(SizeBox, "1920x1080");
        SelectTag(BufferBox, "3");
        SelectTag(ColorFormatBox, "uyvy");
    }

    private void RebuildLayouts()
    {
        MvList.ItemsSource = null;
        MvList.ItemsSource = _session.Multiviews;
        if (_session.Multiviews.Count > 0 && MvList.SelectedIndex < 0)
            MvList.SelectedIndex = 0;
    }

    private void AddMv_Click(object sender, RoutedEventArgs e)
    {
        if (Owner is MainWindow main)
            main.OpenNewMultiview(Settings.DefaultMultiviewUnitId);
        RebuildLayouts();
    }

    private void OpenMv_Click(object sender, RoutedEventArgs e)
    {
        if (MvList.SelectedItem is not MultiviewLayout layout)
        {
            if (Owner is MainWindow mainNew)
                mainNew.OpenNewMultiview(Settings.DefaultMultiviewUnitId);
            RebuildLayouts();
            return;
        }
        if (Owner is MainWindow main)
            main.OpenMultiviewWindow(layout);
    }

    private void EditLayoutTiles_Click(object sender, RoutedEventArgs e)
    {
        if (MvList.SelectedItem is not MultiviewLayout layout)
            return;
        var dialog = new MultiviewSlotsWindow(_session, layout) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        var unit = MvUnitBox.SelectedItem as MixingUnitEntry ?? _session.Units[0];
        ((App)Application.Current).Commands.PushMultiviewNow(layout, unit.Width, unit.Height);
    }

    private void DeleteMv_Click(object sender, RoutedEventArgs e)
    {
        if (MvList.SelectedItem is not MultiviewLayout layout)
            return;
        if (Owner is MainWindow main)
            main.CloseMultiview(layout.Id);
        MixerNative.DestroyScene(layout.GpuId);
        _session.Multiviews.Remove(layout);
        RebuildLayouts();
    }

    private void EditTiles_Click(object sender, RoutedEventArgs e)
    {
    }

    private void AddOutput_Click(object sender, RoutedEventArgs e)
    {
        Outputs.Add(new OutputEntry
        {
            Id = _nextOutputId++,
            Name = NextOutputName(),
            Transport = OutputTransport.Omt,
            SourceKind = OutputSourceKind.MuProgram,
            UnitId = _session.Units.Count > 0 ? _session.Units[0].Id : 1
        });
        RebuildOutputs();
    }

    private void RebuildOutputs()
    {
        OutputRows.Children.Clear();
        _suppressOutputs = true;
        for (var i = 0; i < Outputs.Count; i++)
        {
            var index = i;
            var output = Outputs[i];
            var box = new Border
            {
                BorderBrush = new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x44, 0x44, 0x44)),
                BorderThickness = new Thickness(1),
                Padding = new Thickness(8),
                Margin = new Thickness(0, 0, 0, 8)
            };
            var grid = new Grid();
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());
            grid.RowDefinitions.Add(new RowDefinition());

            var name = new TextBox { Text = output.Name, Margin = new Thickness(0, 0, 8, 6) };
            name.TextChanged += (_, _) => output.Name = name.Text.Trim();
            var transport = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
            transport.Items.Add(new ComboBoxItem { Content = "OMT", Tag = OutputTransport.Omt });
            transport.Items.Add(new ComboBoxItem { Content = "NDI", Tag = OutputTransport.Ndi });
            transport.Items.Add(new ComboBoxItem { Content = "DeckLink", Tag = OutputTransport.DeckLink });
            transport.SelectedIndex = (int)output.Transport;
            transport.SelectionChanged += (_, _) =>
            {
                if (transport.SelectedItem is ComboBoxItem item && item.Tag is OutputTransport value)
                    output.Transport = value;
            };

            var kinds = new WrapPanel { Margin = new Thickness(0, 0, 0, 6) };
            AddKind(kinds, output, OutputSourceKind.Input, "Input", index);
            AddKind(kinds, output, OutputSourceKind.Scene, "Scene", index);
            AddKind(kinds, output, OutputSourceKind.MuPreview, "MU PRV", index);
            AddKind(kinds, output, OutputSourceKind.MuProgram, "MU PGM", index);
            AddKind(kinds, output, OutputSourceKind.Multiview, "Multiview", index);

            var pick = new ComboBox { Margin = new Thickness(0, 0, 8, 6) };
            FillOutputPick(pick, output);
            pick.SelectionChanged += (_, _) => ApplyOutputPick(pick, output);

            var remove = new Button { Content = "−", Width = 28 };
            remove.Click += (_, _) =>
            {
                Outputs.RemoveAt(index);
                RebuildOutputs();
            };

            Grid.SetRow(name, 0);
            Grid.SetColumnSpan(name, 3);
            Grid.SetRow(remove, 0);
            Grid.SetColumn(remove, 3);
            Grid.SetRow(transport, 1);
            Grid.SetRow(kinds, 2);
            Grid.SetColumnSpan(kinds, 4);
            Grid.SetRow(pick, 3);
            Grid.SetColumnSpan(pick, 3);
            grid.Children.Add(name);
            grid.Children.Add(remove);
            grid.Children.Add(transport);
            grid.Children.Add(kinds);
            grid.Children.Add(pick);
            box.Child = grid;
            OutputRows.Children.Add(box);
        }
        _suppressOutputs = false;
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        if (FpsBox.SelectedItem is ComboBoxItem fps && fps.Tag is string fpsTag)
        {
            var parts = fpsTag.Split('/');
            Settings.MasterFpsNum = uint.Parse(parts[0]);
            Settings.MasterFpsDen = uint.Parse(parts[1]);
        }
        if (SizeBox.SelectedItem is ComboBoxItem size && size.Tag is string sizeTag)
        {
            var parts = sizeTag.Split('x');
            Settings.DefaultWidth = uint.Parse(parts[0]);
            Settings.DefaultHeight = uint.Parse(parts[1]);
        }
        _session.NextOutputId = _nextOutputId;
        if (MvUnitBox.SelectedItem is MixingUnitEntry unit)
            Settings.DefaultMultiviewUnitId = unit.Id;
        if (BufferBox.SelectedItem is ComboBoxItem buffer && buffer.Tag is string bufferTag
            && uint.TryParse(bufferTag, out var frames))
            Settings.FrameBufferFrames = Math.Clamp(frames, 1u, 8u);
        if (ColorFormatBox.SelectedItem is ComboBoxItem color && color.Tag is string colorTag)
            Settings.InternalColorFormat = colorTag == "bgra" ? InternalColorFormat.Bgra : InternalColorFormat.Uyvy;
        DialogResult = true;
    }

    private void AddKind(WrapPanel panel, OutputEntry output, OutputSourceKind kind, string label, int index)
    {
        var radio = new RadioButton
        {
            Content = label,
            GroupName = $"out-{index}",
            IsChecked = output.SourceKind == kind,
            Foreground = System.Windows.Media.Brushes.White,
            Margin = new Thickness(0, 0, 12, 0)
        };
        radio.Checked += (_, _) =>
        {
            if (_suppressOutputs)
                return;
            output.SourceKind = kind;
            RebuildOutputs();
        };
        panel.Children.Add(radio);
    }

    private void FillOutputPick(ComboBox box, OutputEntry output)
    {
        switch (output.SourceKind)
        {
            case OutputSourceKind.Input:
                box.ItemsSource = _session.Inputs;
                box.DisplayMemberPath = "Name";
                box.SelectedValuePath = "Id";
                box.SelectedValue = output.SourceId;
                if (box.SelectedItem is InputEntry input)
                    output.SourceId = input.Id;
                else if (_session.Inputs.Count > 0)
                {
                    box.SelectedIndex = 0;
                    output.SourceId = _session.Inputs[0].Id;
                }
                break;
            case OutputSourceKind.Scene:
                box.ItemsSource = _session.Scenes;
                box.DisplayMemberPath = "Name";
                box.SelectedValuePath = "GpuId";
                box.SelectedValue = output.SourceId;
                if (box.SelectedItem is SceneEntry scene)
                    output.SourceId = scene.GpuId;
                else if (_session.Scenes.Count > 0)
                {
                    box.SelectedIndex = 0;
                    output.SourceId = _session.Scenes[0].GpuId;
                }
                break;
            case OutputSourceKind.Multiview:
                box.ItemsSource = _session.Multiviews;
                box.DisplayMemberPath = "Name";
                box.SelectedValuePath = "GpuId";
                box.SelectedValue = output.SourceId;
                if (box.SelectedItem is MultiviewLayout layout)
                    output.SourceId = layout.GpuId;
                else if (_session.Multiviews.Count > 0)
                {
                    box.SelectedIndex = 0;
                    output.SourceId = _session.Multiviews[0].GpuId;
                }
                break;
            default:
                box.ItemsSource = _session.Units;
                box.DisplayMemberPath = "Name";
                box.SelectedValuePath = "Id";
                box.SelectedValue = output.UnitId;
                if (box.SelectedItem is MixingUnitEntry unit)
                    output.UnitId = unit.Id;
                break;
        }
    }

    private static void ApplyOutputPick(ComboBox box, OutputEntry output)
    {
        switch (output.SourceKind)
        {
            case OutputSourceKind.Input:
                if (box.SelectedItem is InputEntry input)
                    output.SourceId = input.Id;
                break;
            case OutputSourceKind.Scene:
                if (box.SelectedItem is SceneEntry scene)
                    output.SourceId = scene.GpuId;
                break;
            case OutputSourceKind.Multiview:
                if (box.SelectedItem is MultiviewLayout layout)
                    output.SourceId = layout.GpuId;
                break;
            default:
                if (box.SelectedItem is MixingUnitEntry unit)
                    output.UnitId = unit.Id;
                break;
        }
    }

    private string NextOutputName()
    {
        const string prefix = "eiviz-out";
        if (Outputs.TrueForAll(item => item.Name != prefix)
            && _session.Outputs.TrueForAll(item => item.Name != prefix))
            return prefix;
        for (var i = 2; ; i++)
        {
            var name = $"{prefix}-{i}";
            if (Outputs.TrueForAll(item => item.Name != name)
                && _session.Outputs.TrueForAll(item => item.Name != name))
                return name;
        }
    }

    private static OutputEntry Clone(OutputEntry output) => new()
    {
        Id = output.Id,
        Name = output.Name,
        Transport = output.Transport,
        SourceKind = output.SourceKind,
        SourceId = output.SourceId,
        UnitId = output.UnitId
    };

    private static void SelectTag(ComboBox box, string tag)
    {
        foreach (ComboBoxItem item in box.Items)
        {
            if (Equals(item.Tag, tag))
            {
                box.SelectedItem = item;
                return;
            }
        }
    }
}
