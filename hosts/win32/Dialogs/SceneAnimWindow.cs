using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using System.Windows.Threading;
using Eiviz.Host;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Preview;
using MixerRect = Eiviz.Host.Interop.Rect;

namespace Eiviz.Host.Dialogs;

internal sealed class SceneAnimWindow : Window
{
    private static readonly (string Label, uint Kind)[] Easings =
    [
        ("Linear", MixerNative.EasingLinear),
        ("EaseIn", MixerNative.EasingIn),
        ("EaseOut", MixerNative.EasingOut),
        ("EaseInOut", MixerNative.EasingInOut),
        ("Smoothstep", MixerNative.EasingSmoothstep),
        ("Bezier", MixerNative.EasingBezier),
        ("Hold", MixerNative.EasingHold)
    ];

    private static readonly Brush Panel = new SolidColorBrush(Color.FromRgb(0x11, 0x11, 0x11));
    private static readonly Brush Card = new SolidColorBrush(Color.FromRgb(0x24, 0x24, 0x24));
    private static readonly Brush LiveFill = new SolidColorBrush(Color.FromRgb(0x1E, 0x6B, 0x3A));
    private static readonly Brush MoveFill = new SolidColorBrush(Color.FromRgb(0x2E, 0x5E, 0x8E));
    private static readonly Brush WaitFill = new SolidColorBrush(Color.FromRgb(0x3A, 0x3A, 0x3A));

    private readonly SceneEntry _scene;
    private readonly bool _persist;
    private readonly ListBox _states = new();
    private readonly ListBox _sequences = new();
    private readonly StackPanel _stateForm = new();
    private readonly StackPanel _sequenceForm = new();
    private readonly Grid _timeline = new() { Height = 28, Margin = new Thickness(0, 4, 0, 2) };
    private readonly TextBlock _total = new() { FontSize = 11, Foreground = Brushes.Silver };
    private readonly TextBlock _status = new() { Foreground = Brushes.Goldenrod, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 8, 0, 0) };
    private readonly Canvas _wire = new() { Background = Brushes.Black, ClipToBounds = true };
    private readonly SwapchainHost? _preview = App.IsRemote ? null : new();
    private readonly DispatcherTimer _tally;
    private SceneAnimLive _live = SceneAnimLive.Idle;
    private bool _filling;
    private SceneLayer? _picked;
    private bool _moving;
    private bool _sizing;
    private bool _panning;
    private bool _zooming;
    private float _grabX;
    private float _grabY;
    private float? _snapX;
    private float? _snapY;
    private float _viewZoom = 0.8f;
    private DateTime _lastShow;

    public SceneAnimWindow(SceneEntry scene, bool persist)
    {
        _scene = scene;
        _persist = persist;
        Title = Loc.Format("anim.title", scene.Name);
        Width = 1040;
        Height = 900;
        Background = new SolidColorBrush(Color.FromRgb(0x1A, 0x1A, 0x1A));
        Foreground = Brushes.White;
        Content = Build();
        _tally = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(200) };
        _tally.Tick += (_, _) => PaintTally();
        Loaded += (_, _) =>
        {
            _scene.AssignLayerIds();
            var (width, height) = ProjectSize();
            _wire.Width = width;
            _wire.Height = height;
            Reload();
            _tally.Start();
            if (_preview is not null && Application.Current is App app)
            {
                var monitorId = app.Session.NextMonitorId++;
                Dispatcher.BeginInvoke(
                    () => _preview.RetargetMonitor(monitorId, _scene.GpuId),
                    DispatcherPriority.Loaded);
            }
        };
        Closed += (_, _) =>
        {
            _tally.Stop();
            _preview?.ReleaseNative();
            if (_persist && Remote)
                Push(publish: true);
        };
    }

    private UIElement Build()
    {
        var root = new DockPanel { Margin = new Thickness(12) };
        root.Children.Add(_status);
        DockPanel.SetDock(_status, Dock.Bottom);
        var stage = BuildStage();
        DockPanel.SetDock(stage, Dock.Top);
        root.Children.Add(stage);
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(2, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(16) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(3, GridUnitType.Star) });
        grid.Children.Add(Column(Loc.T("anim.states"), Loc.T("anim.statesHelp"), _states, StateButtons(), _stateForm));
        var divider = new Border
        {
            Width = 1,
            Background = new SolidColorBrush(Color.FromRgb(0x55, 0x55, 0x55)),
            HorizontalAlignment = HorizontalAlignment.Center
        };
        Grid.SetColumn(divider, 1);
        grid.Children.Add(divider);
        var right = Column(Loc.T("anim.sequences"), Loc.T("anim.sequencesHelp"), _sequences, SequenceButtons(), _sequenceForm);
        Grid.SetColumn(right, 2);
        grid.Children.Add(right);
        root.Children.Add(grid);
        _states.SelectionChanged += (_, _) => { if (!_filling) ShowState(); };
        _sequences.SelectionChanged += (_, _) => { if (!_filling) ShowSequence(); };
        return root;
    }

    private static DockPanel Column(string title, string help, ListBox list, Panel buttons, Panel form)
    {
        list.Background = Panel;
        list.Foreground = Brushes.White;
        list.Height = 150;
        list.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        var dock = new DockPanel();
        var heading = new TextBlock { Text = title, FontWeight = FontWeights.Bold, FontSize = 14 };
        var note = Note(help);
        var scroll = new ScrollViewer
        {
            Content = form,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Margin = new Thickness(0, 6, 0, 0)
        };
        foreach (var top in new UIElement[] { heading, note, list, buttons })
        {
            DockPanel.SetDock(top, Dock.Top);
            dock.Children.Add(top);
        }
        dock.Children.Add(scroll);
        return dock;
    }

    private Panel StateButtons()
    {
        var bar = new WrapPanel { Margin = new Thickness(0, 6, 0, 0) };
        bar.Children.Add(Button(Loc.T("anim.newState"), (_, _) => AddState()));
        bar.Children.Add(Button(Loc.T("anim.updateState"), (_, _) => Capture()));
        bar.Children.Add(Button(Loc.T("anim.delete"), (_, _) => DeleteState()));
        return bar;
    }

    private Panel SequenceButtons()
    {
        var bar = new WrapPanel { Margin = new Thickness(0, 6, 0, 0) };
        bar.Children.Add(Button(Loc.T("anim.newSequence"), (_, _) => AddSequence()));
        bar.Children.Add(Button(Loc.T("anim.delete"), (_, _) => DeleteSequence()));
        bar.Children.Add(new Border { Width = 12 });
        bar.Children.Add(Button(Loc.T("anim.play"), (_, _) => Run(SelectedSequence(), MixerNative.SceneSeqPlay)));
        bar.Children.Add(Button(Loc.T("anim.reverse"), (_, _) => Run(SelectedSequence(), MixerNative.SceneSeqReverse)));
        bar.Children.Add(Button(Loc.T("anim.stop"), (_, _) => Run(SelectedSequence(), MixerNative.SceneSeqStop)));
        return bar;
    }

    private void Reload()
    {
        var stateId = SelectedState()?.Id;
        var sequenceId = SelectedSequence()?.Id;
        _filling = true;
        FillLists();
        Select(_states, stateId);
        Select(_sequences, sequenceId);
        _filling = false;
        ShowState();
        ShowSequence();
        PaintTally();
    }

    private void RefreshRows()
    {
        var stateId = SelectedState()?.Id;
        var sequenceId = SelectedSequence()?.Id;
        _filling = true;
        FillLists();
        Select(_states, stateId);
        Select(_sequences, sequenceId);
        _filling = false;
        PaintTally();
    }

    private void FillLists()
    {
        _states.Items.Clear();
        for (var i = 0; i < _scene.States.Count; i++)
            _states.Items.Add(StateRow(_scene.States[i], i));
        _sequences.Items.Clear();
        for (var i = 0; i < _scene.Sequences.Count; i++)
            _sequences.Items.Add(SequenceRow(_scene.Sequences[i], i));
    }

    private ListBoxItem StateRow(SceneState state, int index)
    {
        var go = SmallButton(Loc.T("anim.goTo"), (_, _) => Go(state.Id));
        var row = Row(go, DisplayName(state.Name, state.Id), MotionSummary(state.Enter), index, _scene.States.Count, MoveState);
        row.Tag = state.Id;
        return row;
    }

    private ListBoxItem SequenceRow(SceneSequence sequence, int index)
    {
        var play = SmallButton("▶", (_, _) => Run(sequence, MixerNative.SceneSeqPlay));
        var summary = Loc.Format("anim.stepCount", sequence.Steps.Count, Seconds(TotalFrames(sequence)));
        var row = Row(play, DisplayName(sequence.Name, sequence.Id), summary, index, _scene.Sequences.Count, MoveSequence);
        row.Tag = sequence.Id;
        return row;
    }

    private ListBoxItem Row(Button action, string title, string detail, int index, int count, Action<int, int> move)
    {
        var row = new DockPanel();
        action.Margin = new Thickness(2, 0, 0, 0);
        var tools = new StackPanel { Orientation = Orientation.Horizontal };
        tools.Children.Add(IconButton("↑", Loc.T("anim.moveUp"), index > 0, () => move(index, -1)));
        tools.Children.Add(IconButton("↓", Loc.T("anim.moveDown"), index < count - 1, () => move(index, 1)));
        tools.Children.Add(action);
        DockPanel.SetDock(tools, Dock.Right);
        row.Children.Add(tools);
        row.Children.Add(TwoLines(title, detail));
        return new ListBoxItem { Content = row, Foreground = Brushes.White, Padding = new Thickness(6, 3, 6, 3) };
    }

    private void MoveState(int index, int delta) => Move(_scene.States, index, delta);

    private void MoveSequence(int index, int delta) => Move(_scene.Sequences, index, delta);

    private void Move<T>(IList<T> items, int index, int delta)
    {
        var next = index + delta;
        if (next < 0 || next >= items.Count)
            return;
        (items[index], items[next]) = (items[next], items[index]);
        Persist();
        Reload();
    }

    private static StackPanel TwoLines(string title, string detail)
    {
        var lines = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        lines.Children.Add(new TextBlock { Text = title, FontWeight = FontWeights.SemiBold, TextTrimming = TextTrimming.CharacterEllipsis });
        lines.Children.Add(new TextBlock { Text = detail, FontSize = 11, Foreground = Brushes.Silver });
        return lines;
    }

    private static void Select(ListBox list, ulong? id)
    {
        foreach (ListBoxItem item in list.Items)
        {
            if (item.Tag is ulong value && value == id)
            {
                list.SelectedItem = item;
                return;
            }
        }
        if (list.Items.Count > 0)
            list.SelectedIndex = 0;
    }

    private SceneState? SelectedState()
    {
        if (_states.SelectedItem is not ListBoxItem item || item.Tag is not ulong id)
            return null;
        return _scene.States.FirstOrDefault(state => state.Id == id);
    }

    private SceneSequence? SelectedSequence()
    {
        if (_sequences.SelectedItem is not ListBoxItem item || item.Tag is not ulong id)
            return null;
        return _scene.Sequences.FirstOrDefault(sequence => sequence.Id == id);
    }

    private void ShowState()
    {
        _stateForm.Children.Clear();
        DrawWire();
        var state = SelectedState();
        if (state is null)
            return;
        _stateForm.Children.Add(Label(Loc.T("anim.name")));
        var name = new TextBox { Text = state.Name };
        name.LostFocus += (_, _) =>
        {
            state.Name = name.Text.Trim();
            RefreshRows();
            Persist();
        };
        _stateForm.Children.Add(name);
        _stateForm.Children.Add(MotionEditor(state.Enter, () => { RefreshRows(); ShowSequence(); Persist(); }));
    }

    private void ShowSequence()
    {
        _sequenceForm.Children.Clear();
        var sequence = SelectedSequence();
        if (sequence is null)
            return;
        _sequenceForm.Children.Add(Label(Loc.T("anim.name")));
        var name = new TextBox { Text = sequence.Name };
        name.LostFocus += (_, _) =>
        {
            sequence.Name = name.Text.Trim();
            RefreshRows();
            Persist();
        };
        _sequenceForm.Children.Add(name);

        _sequenceForm.Children.Add(Label(Loc.T("anim.steps")));
        _sequenceForm.Children.Add(_timeline);
        _sequenceForm.Children.Add(_total);
        BuildTimeline(sequence);

        for (var i = 0; i < sequence.Steps.Count; i++)
            _sequenceForm.Children.Add(StepCard(sequence, i));

        var add = Button(Loc.T("anim.addStep"), (_, _) => AddStep(sequence));
        add.HorizontalAlignment = HorizontalAlignment.Left;
        add.Margin = new Thickness(0, 4, 0, 0);
        _sequenceForm.Children.Add(add);
        if (sequence.Steps.Count < 2)
            _sequenceForm.Children.Add(Note(Loc.T("anim.needTwoSteps")));
    }

    private Border StepCard(SceneSequence sequence, int index)
    {
        var step = sequence.Steps[index];
        var body = new StackPanel();

        var head = new DockPanel();
        var tools = new StackPanel { Orientation = Orientation.Horizontal };
        tools.Children.Add(IconButton("↑", Loc.T("anim.moveUp"), index > 0, () => MoveStep(sequence, index, -1)));
        tools.Children.Add(IconButton("↓", Loc.T("anim.moveDown"), index < sequence.Steps.Count - 1, () => MoveStep(sequence, index, 1)));
        tools.Children.Add(IconButton("×", Loc.T("anim.removeStep"), true, () =>
        {
            sequence.Steps.RemoveAt(index);
            StepsChanged();
        }));
        DockPanel.SetDock(tools, Dock.Right);
        head.Children.Add(tools);
        var badge = new Border
        {
            Background = MoveFill,
            CornerRadius = new CornerRadius(10),
            Width = 20,
            Height = 20,
            Margin = new Thickness(0, 0, 8, 0),
            Child = new TextBlock
            {
                Text = (index + 1).ToString(),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                FontWeight = FontWeights.Bold
            }
        };
        DockPanel.SetDock(badge, Dock.Left);
        head.Children.Add(badge);
        var moveTo = new TextBlock { Text = Loc.T("anim.moveTo"), VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 6, 0), Foreground = Brushes.Silver };
        DockPanel.SetDock(moveTo, Dock.Left);
        head.Children.Add(moveTo);
        var pick = new ComboBox { MinWidth = 160, Margin = new Thickness(0, 0, 8, 0) };
        foreach (var state in _scene.States)
            pick.Items.Add(new ComboBoxItem { Content = DisplayName(state.Name, state.Id), Tag = state.Id, IsSelected = state.Id == step.StateId });
        pick.SelectionChanged += (_, _) =>
        {
            if (pick.SelectedItem is ComboBoxItem item && item.Tag is ulong stateId && stateId != step.StateId)
            {
                step.StateId = stateId;
                StepsChanged();
            }
        };
        head.Children.Add(pick);
        body.Children.Add(head);

        var target = _scene.States.FirstOrDefault(state => state.Id == step.StateId);
        var inherit = new CheckBox
        {
            Content = Loc.Format("anim.useStateTiming", target is null ? "-" : MotionSummary(target.Enter)),
            IsChecked = step.Motion is null,
            Foreground = Brushes.White,
            Margin = new Thickness(28, 6, 0, 0)
        };
        inherit.Checked += (_, _) => { step.Motion = null; StepsChanged(); };
        inherit.Unchecked += (_, _) =>
        {
            step.Motion = target is null ? new Motion() : CopyMotion(target.Enter);
            StepsChanged();
        };
        body.Children.Add(inherit);
        if (step.Motion is { } motion)
        {
            var own = MotionEditor(motion, StepsChanged);
            if (own is FrameworkElement element)
                element.Margin = new Thickness(28, 0, 0, 0);
            body.Children.Add(own);
        }

        var wait = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(28, 6, 0, 0) };
        wait.Children.Add(new TextBlock { Text = Loc.T("anim.thenWait"), VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 6, 0) });
        var waitSeconds = new TextBlock { Text = Seconds(step.HoldFrames), Foreground = Brushes.Silver, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0) };
        wait.Children.Add(NumberBox(step.HoldFrames, 0, value =>
        {
            step.HoldFrames = value;
            waitSeconds.Text = Seconds(value);
            StepsChanged(rebuild: false);
        }));
        wait.Children.Add(new TextBlock { Text = Loc.T("anim.frames"), VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0) });
        wait.Children.Add(waitSeconds);
        body.Children.Add(wait);

        return new Border
        {
            Background = Card,
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(8),
            Margin = new Thickness(0, 6, 0, 0),
            Child = body,
            Tag = index
        };
    }

    private void BuildTimeline(SceneSequence sequence)
    {
        _timeline.Children.Clear();
        _timeline.ColumnDefinitions.Clear();
        var column = 0;
        for (var i = 0; i < sequence.Steps.Count; i++)
        {
            var step = sequence.Steps[i];
            var state = _scene.States.FirstOrDefault(item => item.Id == step.StateId);
            var move = MoveFrames(step);
            AddSegment(ref column, move, MoveFill, $"{i + 1}. {(state is null ? "?" : DisplayName(state.Name, state.Id))}", i, false);
            if (step.HoldFrames > 0)
                AddSegment(ref column, step.HoldFrames, WaitFill, Loc.T("anim.wait"), i, true);
        }
        var total = TotalFrames(sequence);
        _total.Text = Loc.Format("anim.total", total, Seconds(total));
        PaintTimeline();
    }

    private void AddSegment(ref int column, uint frames, Brush fill, string label, int step, bool wait)
    {
        _timeline.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(Math.Max(1, frames), GridUnitType.Star) });
        var segment = new Border
        {
            Background = fill,
            BorderBrush = Brushes.Black,
            BorderThickness = new Thickness(0, 0, 1, 0),
            Tag = (step, wait),
            ToolTip = $"{label} — {frames} {Loc.T("anim.frames")} ({Seconds(frames)})",
            Child = new TextBlock
            {
                Text = label,
                FontSize = 11,
                Margin = new Thickness(4, 0, 4, 0),
                VerticalAlignment = VerticalAlignment.Center,
                TextTrimming = TextTrimming.CharacterEllipsis
            }
        };
        Grid.SetColumn(segment, column);
        _timeline.Children.Add(segment);
        column++;
    }

    private void PaintTimeline()
    {
        var sequence = SelectedSequence();
        var playing = sequence is not null && _live.SequenceId == sequence.Id;
        foreach (var child in _timeline.Children)
        {
            if (child is not Border { Tag: (int step, bool wait) } segment)
                continue;
            var on = playing && _live.StepIndex == step && _live.Holding == wait;
            segment.Background = on ? LiveFill : wait ? WaitFill : MoveFill;
        }
    }

    private void StepsChanged() => StepsChanged(rebuild: true);

    private void StepsChanged(bool rebuild)
    {
        if (SelectedSequence() is not { } sequence)
            return;
        if (rebuild)
            ShowSequence();
        else
            BuildTimeline(sequence);
        RefreshRows();
        Persist();
    }

    private void MoveStep(SceneSequence sequence, int index, int delta)
    {
        var next = index + delta;
        if (next < 0 || next >= sequence.Steps.Count)
            return;
        (sequence.Steps[index], sequence.Steps[next]) = (sequence.Steps[next], sequence.Steps[index]);
        StepsChanged();
    }

    private void AddStep(SceneSequence sequence)
    {
        if (_scene.States.Count == 0)
        {
            _status.Text = Loc.T("anim.needTwoStates");
            return;
        }
        var last = sequence.Steps.Count > 0 ? sequence.Steps[^1].StateId : 0;
        var pick = _scene.States.FirstOrDefault(state => state.Id != last) ?? _scene.States[0];
        sequence.Steps.Add(new SequenceStep { StateId = pick.Id });
        StepsChanged();
    }

    private UIElement MotionEditor(Motion motion, Action changed)
    {
        var panel = new StackPanel();
        panel.Children.Add(Label(Loc.T("anim.moveTime")));
        var row = new StackPanel { Orientation = Orientation.Horizontal };
        var seconds = new TextBlock { Text = Seconds(motion.DurationFrames), Foreground = Brushes.Silver, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0) };
        row.Children.Add(NumberBox(Math.Max(1, motion.DurationFrames), 1, value =>
        {
            motion.DurationFrames = value;
            seconds.Text = Seconds(value);
            changed();
        }));
        row.Children.Add(new TextBlock { Text = Loc.T("anim.frames"), VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0) });
        row.Children.Add(seconds);
        panel.Children.Add(row);

        panel.Children.Add(Label(Loc.T("anim.curve")));
        var easing = new ComboBox { Width = 160, HorizontalAlignment = HorizontalAlignment.Left };
        foreach (var (label, kind) in Easings)
            easing.Items.Add(new ComboBoxItem { Content = label, Tag = kind, IsSelected = motion.Easing == kind });
        var bezier = BezierEditor(motion, changed);
        easing.SelectionChanged += (_, _) =>
        {
            if (easing.SelectedItem is not ComboBoxItem item || item.Tag is not uint kind)
                return;
            motion.Easing = kind;
            if (kind == MixerNative.EasingBezier)
                EnsureBezier(motion);
            else
                motion.Bezier = null;
            bezier.Visibility = kind == MixerNative.EasingBezier ? Visibility.Visible : Visibility.Collapsed;
            changed();
        };
        panel.Children.Add(easing);
        panel.Children.Add(bezier);
        return panel;
    }

    private static StackPanel BezierEditor(Motion motion, Action changed)
    {
        if (motion.Easing == MixerNative.EasingBezier)
            EnsureBezier(motion);
        var panel = new StackPanel
        {
            Visibility = motion.Easing == MixerNative.EasingBezier ? Visibility.Visible : Visibility.Collapsed,
            Margin = new Thickness(0, 4, 0, 0)
        };
        var x1 = HandleBox("X1", () => motion.Bezier?.X1 ?? 0.42f, value => { EnsureBezier(motion); motion.Bezier!.X1 = Math.Clamp(value, 0, 1); }, changed);
        var y1 = HandleBox("Y1", () => motion.Bezier?.Y1 ?? 0, value => { EnsureBezier(motion); motion.Bezier!.Y1 = value; }, changed);
        var x2 = HandleBox("X2", () => motion.Bezier?.X2 ?? 0.58f, value => { EnsureBezier(motion); motion.Bezier!.X2 = Math.Clamp(value, 0, 1); }, changed);
        var y2 = HandleBox("Y2", () => motion.Bezier?.Y2 ?? 1, value => { EnsureBezier(motion); motion.Bezier!.Y2 = value; }, changed);
        var presets = new WrapPanel();
        void Add(string label, float hx1, float hy1, float hx2, float hy2)
        {
            presets.Children.Add(Button(label, (_, _) =>
            {
                motion.Bezier = new BezierHandles { X1 = hx1, Y1 = hy1, X2 = hx2, Y2 = hy2 };
                motion.Easing = MixerNative.EasingBezier;
                x1.Text = hx1.ToString("0.###");
                y1.Text = hy1.ToString("0.###");
                x2.Text = hx2.ToString("0.###");
                y2.Text = hy2.ToString("0.###");
                changed();
            }));
        }
        Add("Ease", 0.25f, 0.1f, 0.25f, 1);
        Add("In", 0.42f, 0, 1, 1);
        Add("Out", 0, 0, 0.58f, 1);
        Add("In Out", 0.42f, 0, 0.58f, 1);
        panel.Children.Add(presets);
        var handles = new WrapPanel();
        foreach (var box in new[] { x1, y1, x2, y2 })
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 2, 10, 0) };
            row.Children.Add(new TextBlock { Text = (string)box.Tag, Width = 22, VerticalAlignment = VerticalAlignment.Center });
            row.Children.Add(box);
            handles.Children.Add(row);
        }
        panel.Children.Add(handles);
        return panel;
    }

    private static TextBox HandleBox(string title, Func<float> get, Action<float> set, Action changed)
    {
        var box = new TextBox { Text = get().ToString("0.###"), Width = 56, Tag = title };
        void Commit()
        {
            if (float.TryParse(box.Text, out var value))
            {
                set(value);
                box.Text = get().ToString("0.###");
                changed();
            }
        }
        box.LostFocus += (_, _) => Commit();
        box.KeyDown += (_, e) => { if (e.Key == Key.Enter) Commit(); };
        return box;
    }

    private static TextBox NumberBox(uint value, uint min, Action<uint> set)
    {
        var box = new TextBox { Text = value.ToString(), Width = 64 };
        var last = value;
        void Commit()
        {
            if (uint.TryParse(box.Text, out var parsed) && parsed >= min)
            {
                if (parsed != last)
                {
                    last = parsed;
                    set(parsed);
                }
            }
            else
            {
                box.Text = last.ToString();
            }
        }
        box.LostFocus += (_, _) => Commit();
        box.KeyDown += (_, e) => { if (e.Key == Key.Enter) Commit(); };
        return box;
    }

    private static void EnsureBezier(Motion motion) =>
        motion.Bezier ??= new BezierHandles { X1 = 0.42f, Y1 = 0, X2 = 0.58f, Y2 = 1 };

    private static Motion CopyMotion(Motion motion) => new()
    {
        DurationFrames = motion.DurationFrames,
        Easing = motion.Easing,
        Bezier = motion.Bezier is null ? null : new BezierHandles
        {
            X1 = motion.Bezier.X1,
            Y1 = motion.Bezier.Y1,
            X2 = motion.Bezier.X2,
            Y2 = motion.Bezier.Y2
        }
    };

    private List<LayerKey> CurrentLayout()
    {
        _scene.AssignLayerIds();
        return _scene.Layers.Where(layer => layer.LayerId != 0).Select(layer => new LayerKey
        {
            LayerId = layer.LayerId,
            Geom = SceneLayerGeom.From(layer)
        }).ToList();
    }

    private void AddState()
    {
        var id = NextId(_scene.States.Select(state => state.Id));
        _scene.States.Add(new SceneState
        {
            Id = id,
            Name = $"State {_scene.States.Count + 1}",
            Enter = new Motion(),
            Layers = CurrentLayout(),
            Camera = _scene.Camera.Clone()
        });
        Persist();
        Reload();
        Select(_states, id);
    }

    private void DeleteState()
    {
        var state = SelectedState();
        if (state is null)
            return;
        _scene.States.Remove(state);
        foreach (var sequence in _scene.Sequences)
            sequence.Steps.RemoveAll(step => step.StateId == state.Id);
        Persist();
        Reload();
    }

    private void Capture()
    {
        var state = SelectedState();
        if (state is null)
            return;
        state.Layers = CurrentLayout();
        state.Camera = _scene.Camera.Clone();
        Persist();
        DrawWire();
        ShowPose(true);
        _status.Text = Loc.T("anim.captured");
    }

    private void AddSequence()
    {
        if (_scene.States.Count < 2)
        {
            _status.Text = Loc.T("anim.needTwoStates");
            return;
        }
        var id = NextId(_scene.Sequences.Select(sequence => sequence.Id));
        _scene.Sequences.Add(new SceneSequence
        {
            Id = id,
            Name = $"Sequence {_scene.Sequences.Count + 1}",
            Steps =
            [
                new SequenceStep { StateId = _scene.States[0].Id },
                new SequenceStep { StateId = _scene.States[1].Id }
            ]
        });
        Persist();
        Reload();
        Select(_sequences, id);
    }

    private void DeleteSequence()
    {
        var sequence = SelectedSequence();
        if (sequence is null)
            return;
        _scene.Sequences.Remove(sequence);
        Persist();
        Reload();
    }

    private void Go(ulong stateId)
    {
        if (!Push(publish: _persist || Remote))
            return;
        if (!SceneAnimPlayback.GoTo(_scene, stateId))
            _status.Text = Loc.T("anim.goFailed");
    }

    private void Run(SceneSequence? sequence, uint op)
    {
        if (sequence is null)
            return;
        if (sequence.Steps.Count < 2)
        {
            _status.Text = Loc.T("anim.needTwoSteps");
            return;
        }
        if (!Push(publish: _persist || Remote))
            return;
        if (!SceneAnimPlayback.Sequence(_scene, sequence.Id, op))
            _status.Text = Loc.T("anim.sequenceFailed");
    }

    private void Persist()
    {
        if (Remote)
            return;
        try
        {
            MixerApply.DefineSceneAnim(_scene);
            if (_persist && Application.Current is App app)
                SessionStore.Publish(app.Session);
            _status.Text = "";
        }
        catch (Exception ex)
        {
            _status.Text = ex.Message;
        }
    }

    private bool Push(bool publish)
    {
        _scene.AssignLayerIds();
        if (Application.Current is not App app)
            return false;
        if (app.Backend.IsRemote)
        {
            if (!publish)
                return true;
            var ok = app.Backend.Mutate(MutationJson.UpsertScene(_scene), app.Backend.Revision, out var error);
            if (!ok)
                _status.Text = error;
            return ok;
        }
        try
        {
            MixerApply.DefineSceneAnim(_scene);
            if (publish)
                SessionStore.Publish(app.Session);
            _status.Text = "";
            return true;
        }
        catch (Exception ex)
        {
            _status.Text = ex.Message;
            return false;
        }
    }

    private static bool Remote => Application.Current is App { Backend.IsRemote: true };

    private void PaintTally()
    {
        _live = SceneAnimPlayback.Read(_scene);
        var lit = _live.MovingState ?? _live.ShownState;
        foreach (ListBoxItem item in _states.Items)
            item.Background = item.Tag is ulong id && lit == id ? LiveFill : Brushes.Transparent;
        foreach (ListBoxItem item in _sequences.Items)
            item.Background = item.Tag is ulong id && _live.SequenceId == id ? LiveFill : Brushes.Transparent;
        PaintTimeline();
    }

    private uint MoveFrames(SequenceStep step)
    {
        if (step.Motion is { } own)
            return Math.Max(1, own.DurationFrames);
        var state = _scene.States.FirstOrDefault(item => item.Id == step.StateId);
        return Math.Max(1, state?.Enter.DurationFrames ?? 1);
    }

    private uint TotalFrames(SceneSequence sequence)
    {
        uint total = 0;
        foreach (var step in sequence.Steps)
            total += MoveFrames(step) + step.HoldFrames;
        return total;
    }

    private static string Seconds(uint frames)
    {
        var settings = (Application.Current as App)?.Session.Settings;
        var fps = settings is null ? 60.0 : settings.MasterFpsNum / (double)Math.Max(1, settings.MasterFpsDen);
        return Loc.Format("anim.seconds", frames / Math.Max(1.0, fps));
    }

    private UIElement BuildStage()
    {
        _wire.MouseLeftButtonDown += WireDown;
        _wire.MouseMove += WireMove;
        _wire.MouseLeftButtonUp += WireUp;
        _wire.PreviewMouseWheel += WireWheel;
        var (width, height) = ProjectSize();
        _wire.Width = width;
        _wire.Height = height;
        var view = new Viewbox
        {
            Stretch = Stretch.Uniform,
            Child = _wire,
            Margin = new Thickness(0, 0, 8, 0)
        };
        var zoomOut = new Button { Content = "−", Width = 26, Height = 26, Tag = "-", ToolTip = Loc.T("editor.viewZoomHelp") };
        var zoomIn = new Button
        {
            Content = "+",
            Width = 26,
            Height = 26,
            Margin = new Thickness(4, 0, 0, 0),
            Tag = "+",
            ToolTip = Loc.T("editor.viewZoomHelp")
        };
        zoomOut.Click += ViewZoom_Click;
        zoomIn.Click += ViewZoom_Click;
        var zoomBar = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Top,
            Margin = new Thickness(0, 4, 12, 0)
        };
        zoomBar.Children.Add(zoomOut);
        zoomBar.Children.Add(zoomIn);
        var stage = new Grid();
        stage.Children.Add(view);
        stage.Children.Add(zoomBar);
        var grid = new Grid { Height = 250, Margin = new Thickness(0, 0, 0, 10) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        if (_preview is not null)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(360) });
            _preview.IsMonitor = true;
            var aspect = new AspectBox
            {
                RatioWidth = width,
                RatioHeight = height,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                Child = _preview
            };
            Grid.SetColumn(aspect, 1);
            grid.Children.Add(aspect);
        }
        grid.Children.Add(stage);
        return grid;
    }

    private double MapX(float scene) => (scene * _viewZoom + (1 - _viewZoom) / 2) * _wire.Width;

    private double MapY(float scene) => (scene * _viewZoom + (1 - _viewZoom) / 2) * _wire.Height;

    private double MapW(float scene) => scene * _viewZoom * _wire.Width;

    private double MapH(float scene) => scene * _viewZoom * _wire.Height;

    private float UnmapX(double canvas) =>
        (float)((canvas / Math.Max(_wire.Width, 1) - (1 - _viewZoom) / 2) / _viewZoom);

    private float UnmapY(double canvas) =>
        (float)((canvas / Math.Max(_wire.Height, 1) - (1 - _viewZoom) / 2) / _viewZoom);

    private void ViewZoom_Click(object sender, RoutedEventArgs e)
    {
        var step = sender is Button { Tag: "+" } ? 0.1f : -0.1f;
        _viewZoom = Math.Clamp(_viewZoom + step, 0.5f, 1f);
        DrawWire();
    }

    private void WireWheel(object sender, MouseWheelEventArgs e)
    {
        _viewZoom = Math.Clamp(_viewZoom + (e.Delta > 0 ? 0.1f : -0.1f), 0.5f, 1f);
        DrawWire();
        e.Handled = true;
    }

    private static (uint Width, uint Height) ProjectSize()
    {
        if (Application.Current is App app)
        {
            var unit = app.Session.Units.FirstOrDefault(item => item.Id == app.Session.SelectedUnitId)
                ?? app.Session.Units.FirstOrDefault();
            if (unit is { Width: > 0, Height: > 0 })
                return (unit.Width, unit.Height);
        }
        return (1920, 1080);
    }

    private SceneLayerGeom GeomOf(SceneState state, SceneLayer layer) =>
        state.Layers.FirstOrDefault(key => key.LayerId == layer.LayerId)?.Geom ?? SceneLayerGeom.From(layer);

    private SceneLayerGeom EnsureGeom(SceneState state, SceneLayer layer)
    {
        var key = state.Layers.FirstOrDefault(item => item.LayerId == layer.LayerId);
        if (key is not null)
            return key.Geom;
        var geom = SceneLayerGeom.From(layer);
        state.Layers.Add(new LayerKey { LayerId = layer.LayerId, Geom = geom });
        return geom;
    }

    private SceneCamera CameraOf(SceneState state) => state.Camera ?? _scene.Camera;

    private static void CameraSceneRect(SceneCamera camera, out float left, out float top, out float width, out float height)
    {
        var zoom = Math.Clamp(camera.Zoom, 1f, 8f);
        width = 1f / zoom;
        height = 1f / zoom;
        left = camera.X - width / 2f;
        top = camera.Y - height / 2f;
    }

    private void DrawWire()
    {
        _wire.Children.Clear();
        var output = new Rectangle
        {
            Width = Math.Max(8, MapW(1)),
            Height = Math.Max(8, MapH(1)),
            Fill = new SolidColorBrush(Color.FromRgb(0x14, 0x14, 0x14)),
            Stroke = new SolidColorBrush(Color.FromRgb(0x66, 0x66, 0x66)),
            StrokeThickness = 1,
            IsHitTestVisible = false
        };
        Canvas.SetLeft(output, MapX(0));
        Canvas.SetTop(output, MapY(0));
        _wire.Children.Add(output);
        var state = SelectedState();
        var ordered = _scene.Layers.OrderBy(layer => state is null ? layer.Z : GeomOf(state, layer).Z).ToList();
        for (var i = 0; i < ordered.Count; i++)
        {
            var layer = ordered[i];
            var geom = state is null ? SceneLayerGeom.From(layer) : GeomOf(state, layer);
            var color = WireColor(i);
            var box = new Rectangle
            {
                Width = Math.Max(8, MapW(geom.Width)),
                Height = Math.Max(8, MapH(geom.Height)),
                Stroke = new SolidColorBrush(color),
                StrokeThickness = layer == _picked ? 4 : 2,
                Fill = new SolidColorBrush(Color.FromArgb(0x28, color.R, color.G, color.B)),
                IsHitTestVisible = false
            };
            Canvas.SetLeft(box, MapX(geom.X));
            Canvas.SetTop(box, MapY(geom.Y));
            _wire.Children.Add(box);
            if (layer == _picked && !layer.Locked)
            {
                var handle = new Rectangle
                {
                    Width = 16,
                    Height = 16,
                    Fill = new SolidColorBrush(color),
                    IsHitTestVisible = false
                };
                Canvas.SetLeft(handle, MapX(geom.X + geom.Width) - 16);
                Canvas.SetTop(handle, MapY(geom.Y + geom.Height) - 16);
                _wire.Children.Add(handle);
            }
        }
        if (state is null || _wire.Width <= 0)
            return;
        var camera = CameraOf(state);
        CameraSceneRect(camera, out var sceneLeft, out var sceneTop, out var sceneWidth, out var sceneHeight);
        var width = MapW(sceneWidth);
        var height = MapH(sceneHeight);
        var left = MapX(sceneLeft);
        var top = MapY(sceneTop);
        var frame = new Rectangle
        {
            Width = Math.Max(8, width),
            Height = Math.Max(8, height),
            Stroke = new SolidColorBrush(Color.FromRgb(0xFF, 0xE0, 0x82)),
            StrokeThickness = 3,
            StrokeDashArray = new DoubleCollection { 8, 4 },
            Fill = Brushes.Transparent,
            IsHitTestVisible = false
        };
        Canvas.SetLeft(frame, left);
        Canvas.SetTop(frame, top);
        _wire.Children.Add(frame);
        var zoomHandle = new Rectangle
        {
            Width = 16,
            Height = 16,
            Fill = new SolidColorBrush(Color.FromRgb(0xFF, 0xE0, 0x82)),
            IsHitTestVisible = false
        };
        Canvas.SetLeft(zoomHandle, left + width - 16);
        Canvas.SetTop(zoomHandle, top + height - 16);
        _wire.Children.Add(zoomHandle);
    }

    private static Color WireColor(int index)
    {
        Color[] colors =
        [
            Color.FromRgb(0xE8, 0x77, 0x22),
            Color.FromRgb(0x42, 0xA5, 0xF5),
            Color.FromRgb(0x66, 0xBB, 0x6A),
            Color.FromRgb(0xAB, 0x47, 0xBC),
            Color.FromRgb(0xEF, 0x53, 0x50)
        ];
        return colors[index % colors.Length];
    }

    private void WireDown(object sender, MouseButtonEventArgs e)
    {
        var state = SelectedState();
        if (state is null)
            return;
        var pos = e.GetPosition(_wire);
        _snapX = null;
        _snapY = null;
        if (OnLayerHandle(state, pos))
        {
            _sizing = true;
            _wire.CaptureMouse();
            return;
        }
        if (BeginCamera(state, pos))
        {
            _wire.CaptureMouse();
            return;
        }
        _picked = HitPose(state, pos);
        _moving = _picked is { Locked: false };
        if (_moving && _picked is not null)
        {
            var geom = EnsureGeom(state, _picked);
            _grabX = UnmapX(pos.X) - geom.X;
            _grabY = UnmapY(pos.Y) - geom.Y;
            _wire.CaptureMouse();
        }
        DrawWire();
    }

    private void WireMove(object sender, MouseEventArgs e)
    {
        if (e.LeftButton != MouseButtonState.Pressed)
            return;
        var state = SelectedState();
        if (state is null)
            return;
        var pos = e.GetPosition(_wire);
        if (_zooming || _panning)
        {
            MovePoseCamera(state, pos);
            return;
        }
        if (_sizing && _picked is { Locked: false } sized)
        {
            var geom = EnsureGeom(state, sized);
            var width = Math.Max(0.02f, UnmapX(pos.X) - geom.X);
            var height = sized.SizeLinked && geom.Width > 0
                ? width * (geom.Height / geom.Width)
                : Math.Max(0.02f, UnmapY(pos.Y) - geom.Y);
            var x = geom.X;
            var y = geom.Y;
            var (px, py) = WirePixels();
            SceneSnap.SnapResize(
                ref x,
                ref y,
                ref width,
                ref height,
                moveLeft: false,
                moveTop: false,
                sized.SizeLinked,
                PoseBoxes(state, sized),
                (float)(SceneSnap.EngagePixels / px),
                (float)(SceneSnap.EngagePixels / py));
            geom.X = x;
            geom.Y = y;
            geom.Width = width;
            geom.Height = height;
            DrawWire();
            ShowPose(false);
            return;
        }
        if (_moving && _picked is { Locked: false } moving)
        {
            var geom = EnsureGeom(state, moving);
            var x = UnmapX(pos.X) - _grabX;
            var y = UnmapY(pos.Y) - _grabY;
            var (px, py) = WirePixels();
            var boxes = PoseBoxes(state, moving);
            geom.X = SceneSnap.LatchMoveAxis(x, geom.Width, boxes, true, px, ref _snapX);
            geom.Y = SceneSnap.LatchMoveAxis(y, geom.Height, boxes, false, py, ref _snapY);
            DrawWire();
            ShowPose(false);
        }
    }

    private (double X, double Y) WirePixels()
    {
        var rendered = SceneSnap.RenderedSize(this, _wire);
        return (Math.Max(rendered.Width, 1) * _viewZoom, Math.Max(rendered.Height, 1) * _viewZoom);
    }

    private List<SceneSnap.Box> PoseBoxes(SceneState state, SceneLayer self) =>
        _scene.Layers.Select(layer =>
        {
            var geom = GeomOf(state, layer);
            return new SceneSnap.Box(geom.X, geom.Y, geom.Width, geom.Height, layer.Hidden, layer == self);
        }).ToList();

    private void WireUp(object sender, MouseButtonEventArgs e)
    {
        var edited = _moving || _sizing || _panning || _zooming;
        _moving = false;
        _sizing = false;
        _panning = false;
        _zooming = false;
        _snapX = null;
        _snapY = null;
        _wire.ReleaseMouseCapture();
        if (!edited)
            return;
        if (Remote)
        {
            var id = SelectedState()?.Id ?? 0;
            if (id != 0 && Push(publish: true))
                SceneAnimPlayback.GoTo(_scene, id);
            return;
        }
        Persist();
        ShowPose(true);
    }

    private bool OnLayerHandle(SceneState state, Point pos)
    {
        if (_picked is not { Locked: false } layer)
            return false;
        var geom = GeomOf(state, layer);
        var x = MapX(geom.X + geom.Width);
        var y = MapY(geom.Y + geom.Height);
        return pos.X >= x - 20 && pos.X <= x + 4 && pos.Y >= y - 20 && pos.Y <= y + 4;
    }

    private bool BeginCamera(SceneState state, Point pos)
    {
        var camera = CameraOf(state);
        CameraSceneRect(camera, out var sceneLeft, out var sceneTop, out var sceneWidth, out var sceneHeight);
        var width = MapW(sceneWidth);
        var height = MapH(sceneHeight);
        var left = MapX(sceneLeft);
        var top = MapY(sceneTop);
        var onZoom = pos.X >= left + width - 20 && pos.X <= left + width + 4
            && pos.Y >= top + height - 20 && pos.Y <= top + height + 4;
        if (onZoom)
        {
            _zooming = true;
            _panning = false;
            return true;
        }
        var inside = pos.X >= left - 4 && pos.X <= left + width + 4
            && pos.Y >= top - 4 && pos.Y <= top + height + 4;
        if (!inside || HitPose(state, pos) is not null)
            return false;
        _panning = true;
        _zooming = false;
        _grabX = UnmapX(pos.X) - camera.X;
        _grabY = UnmapY(pos.Y) - camera.Y;
        return true;
    }

    private void MovePoseCamera(SceneState state, Point pos)
    {
        var camera = state.Camera ??= _scene.Camera.Clone();
        if (_zooming)
        {
            var dx = Math.Abs(UnmapX(pos.X) - camera.X);
            var dy = Math.Abs(UnmapY(pos.Y) - camera.Y);
            var half = Math.Max(dx, dy);
            camera.Zoom = half < 1e-3f ? 8f : 0.5f / half;
        }
        else
        {
            camera.X = UnmapX(pos.X) - _grabX;
            camera.Y = UnmapY(pos.Y) - _grabY;
        }
        camera.Zoom = Math.Clamp(camera.Zoom, 1f, 8f);
        var margin = 0.5f / camera.Zoom;
        camera.X = Math.Clamp(camera.X, margin, 1f - margin);
        camera.Y = Math.Clamp(camera.Y, margin, 1f - margin);
        DrawWire();
        ShowPose(false);
    }

    private SceneLayer? HitPose(SceneState state, Point pos)
    {
        var hits = _scene.Layers.Where(layer =>
        {
            var geom = GeomOf(state, layer);
            var x = UnmapX(pos.X);
            var y = UnmapY(pos.Y);
            return x >= geom.X && x <= geom.X + geom.Width
                && y >= geom.Y && y <= geom.Y + geom.Height;
        }).ToList();
        if (hits.Count == 0)
            return null;
        if (_picked is not null && hits.Contains(_picked))
            return _picked;
        return hits.OrderByDescending(layer => GeomOf(state, layer).Z).First();
    }

    private void ShowPose(bool force)
    {
        var state = SelectedState();
        if (state is null || Remote)
            return;
        if (!force && DateTime.UtcNow - _lastShow < TimeSpan.FromMilliseconds(50))
            return;
        _lastShow = DateTime.UtcNow;
        _scene.AssignLayerIds();
        var layers = _scene.Layers.Select(layer =>
        {
            var geom = GeomOf(state, layer);
            return new OverlayDesc
            {
                SourceId = layer.InputId,
                Rect = new MixerRect { X = geom.X, Y = geom.Y, Width = geom.Width, Height = geom.Height },
                Opacity = geom.Opacity,
                Z = geom.Z,
                AudioFollow = layer.AudioFollow ? 1u : 0u,
                Hidden = layer.Hidden ? 1u : 0u,
                Crop = new MixerRect { X = geom.CropX, Y = geom.CropY, Width = geom.CropWidth, Height = geom.CropHeight },
                LayerId = layer.LayerId
            };
        }).ToArray();
        try
        {
            MixerApply.ShowPose(_scene.GpuId, layers, CameraOf(state), state.Id);
        }
        catch (Exception ex)
        {
            _status.Text = ex.Message;
        }
    }

    private static string MotionSummary(Motion motion)
    {
        var curve = Easings.FirstOrDefault(item => item.Kind == motion.Easing).Label ?? "?";
        return $"{Math.Max(1, motion.DurationFrames)} f · {curve} · {Seconds(Math.Max(1, motion.DurationFrames))}";
    }

    private static string DisplayName(string name, ulong id) =>
        string.IsNullOrWhiteSpace(name) ? id.ToString() : name;

    private static ulong NextId(IEnumerable<ulong> ids)
    {
        ulong max = 0;
        foreach (var id in ids)
        {
            if (id > max)
                max = id;
        }
        return max + 1;
    }

    private static Button Button(string label, RoutedEventHandler click)
    {
        var button = new Button { Content = label, Height = 26, Margin = new Thickness(0, 0, 6, 6), Padding = new Thickness(8, 0, 8, 0) };
        button.Click += click;
        return button;
    }

    private static Button SmallButton(string label, RoutedEventHandler click)
    {
        var button = new Button { Content = label, Height = 22, MinWidth = 32, Padding = new Thickness(6, 0, 6, 0), VerticalAlignment = VerticalAlignment.Center };
        button.Click += click;
        return button;
    }

    private static Button IconButton(string label, string tip, bool enabled, Action click)
    {
        var button = new Button { Content = label, Width = 24, Height = 22, Margin = new Thickness(2, 0, 0, 0), ToolTip = tip, IsEnabled = enabled };
        button.Click += (_, _) => click();
        return button;
    }

    private static TextBlock Label(string text) => new()
    {
        Text = text,
        FontSize = 11,
        Foreground = Brushes.Silver,
        Margin = new Thickness(0, 8, 0, 2)
    };

    private static TextBlock Note(string text) => new()
    {
        Text = text,
        FontSize = 11,
        Foreground = Brushes.Silver,
        TextWrapping = TextWrapping.Wrap,
        Margin = new Thickness(0, 2, 0, 6)
    };
}
