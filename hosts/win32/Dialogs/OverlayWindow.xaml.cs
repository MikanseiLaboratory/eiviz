using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using Eiviz.Host;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

public partial class OverlayWindow : Window
{
    private readonly Session _session;
    private MixingUnitEntry _unit;
    private OverlaySlot? _selected;
    private bool _dragging;
    private bool _resizing;
    private bool _cropping;
    private bool _cropLeft;
    private bool _cropRight;
    private bool _cropUp;
    private bool _cropDown;
    private Point _last;
    private float _grabX;
    private float _grabY;
    private float? _snapX;
    private float? _snapY;
    private bool _suppress;
    private DateTime _lastGpuPush;

    public OverlayWindow(Session session, MixingUnitEntry unit)
    {
        InitializeComponent();
        LayoutSnapButton.Content = "🧲";
        LayoutSnapButton.ToolTip = $"{Loc.T("editor.layoutSnap")}\n{Loc.T("editor.layoutSnapHelp")}";
        _session = session;
        _unit = unit;
        AddKindBox.SelectedIndex = 0;
        FillAddSources();
        Title = $"Overlays — {unit.Name}";
        PreviewAspect.RatioWidth = unit.Width;
        PreviewAspect.RatioHeight = unit.Height;
        WireCanvas.Width = unit.Width;
        WireCanvas.Height = unit.Height;
        WireLabel.Text = $"Wireframe ({unit.Width}x{unit.Height})";
        if (!App.IsRemote)
            PreviewHost.RetargetUnit(unit.Id, MixerNative.OutputProgram);
        Loaded += (_, _) =>
        {
            _unit.Overlays.Sort((a, b) => b.Z.CompareTo(a.Z));
            NormalizeOrder();
            RefreshList();
            AttachDrags();
            ListReorder.Attach(SlotList, MoveSlot);
        };
        Closed += (_, _) => PreviewHost.ReleaseNative();
    }

    internal void RefreshEnabled() => RefreshList();

    public void Reload(MixingUnitEntry unit)
    {
        _unit = unit;
        Title = $"Overlays — {unit.Name}";
        PreviewAspect.RatioWidth = unit.Width;
        PreviewAspect.RatioHeight = unit.Height;
        WireCanvas.Width = unit.Width;
        WireCanvas.Height = unit.Height;
        WireLabel.Text = $"Wireframe ({unit.Width}x{unit.Height})";
        if (!App.IsRemote)
            PreviewHost.RetargetUnit(unit.Id, MixerNative.OutputProgram);
        _selected = unit.Overlays.FirstOrDefault();
        RefreshList();
    }

    private float WidthPx => _unit.Width;
    private float HeightPx => _unit.Height;

    private void AttachDrags()
    {
        void Bind(FrameworkElement handle, TextBox box, float scale, string format = "0.#")
        {
            void Preview() => ApplyNumeric(false, box);
            void Commit() => ApplyNumeric(true, box);
            NumericDrag.Attach(handle, box, scale, Preview, Commit, format);
            NumericDrag.AttachBox(box, scale, Preview, Commit, format);
        }
        Bind(PosXLabel, XBox, 2);
        Bind(PosYLabel, YBox, 2);
        Bind(SizeXLabel, WBox, 2);
        Bind(SizeYLabel, HBox, 2);
        Bind(CropXLabel, CropXBox, 2);
        Bind(CropYLabel, CropYBox, 2);
        Bind(CropWLabel, CropWBox, 2);
        Bind(CropHLabel, CropHBox, 2);
        Bind(OpLabel, OpBox, 400, "0.###");
    }

    private void Push()
    {
        if (Application.Current is App { Backend.IsRemote: true } app && _selected is not null)
        {
            var index = (uint)Math.Max(0, _unit.Overlays.IndexOf(_selected));
            if (Owner is MainWindow remote)
                remote.RemoteMutate(MutationJson.SetOverlaySlot(_unit.Id, index, _selected), Loc.T("chrome.overlay"), reloadDocument: false);
        }
        else
            MixerApply.PatchAux(_unit.Id, _unit);
        if (Owner is MainWindow main)
            main.RebuildOverlayToggles();
    }

    private void NormalizeOrder()
    {
        for (var i = 0; i < _unit.Overlays.Count; i++)
            _unit.Overlays[i].Z = _unit.Overlays.Count - 1 - i;
    }

    private bool MoveSlot(int from, int to)
    {
        if (from < 0 || from >= _unit.Overlays.Count)
            return false;
        to = Math.Clamp(to, 0, _unit.Overlays.Count);
        if (to == from || to == from + 1)
            return false;
        var slot = _unit.Overlays[from];
        _unit.Overlays.RemoveAt(from);
        if (to > from)
            to--;
        _unit.Overlays.Insert(to, slot);
        _selected = slot;
        NormalizeOrder();
        RefreshList();
        Push();
        return true;
    }

    private void RefreshList()
    {
        _suppress = true;
        SlotList.Items.Clear();
        foreach (var slot in _unit.Overlays)
            SlotList.Items.Add(BuildSlotRow(slot));
        if (_selected is not null)
        {
            var index = _unit.Overlays.IndexOf(_selected);
            if (index >= 0)
                SlotList.SelectedIndex = index;
        }
        else if (_unit.Overlays.Count > 0)
        {
            _selected = _unit.Overlays[0];
            SlotList.SelectedIndex = 0;
        }
        _suppress = false;
        DrawWireframe();
        FillFields();
        UpdatePreview();
    }

    private DockPanel BuildSlotRow(OverlaySlot slot)
    {
        var audio = new Button
        {
            Content = slot.AudioFollow ? "🔊" : "🔇",
            Width = 26,
            Height = 22,
            Padding = new Thickness(0),
            Tag = slot,
            ToolTip = "Audio Follow"
        };
        var lockBtn = new Button
        {
            Content = slot.Locked ? "🔒" : "🔓",
            Width = 26,
            Height = 22,
            Padding = new Thickness(0),
            Margin = new Thickness(4, 0, 0, 0),
            Tag = slot,
            ToolTip = "Lock"
        };
        audio.Click += SlotAudio_Click;
        lockBtn.Click += SlotLock_Click;
        DockPanel.SetDock(audio, Dock.Left);
        DockPanel.SetDock(lockBtn, Dock.Left);
        var name = new TextBlock
        {
            Text = $"{_unit.Overlays.IndexOf(slot) + 1}. {slot.DisplayName(_session)}",
            Margin = new Thickness(8, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = slot.Enabled ? Brushes.White : new SolidColorBrush(Color.FromRgb(0x88, 0x88, 0x88))
        };
        var row = new DockPanel { Tag = slot, LastChildFill = true };
        row.Children.Add(audio);
        row.Children.Add(lockBtn);
        row.Children.Add(name);
        return row;
    }

    private void DrawWireframe()
    {
        WireCanvas.Children.Clear();
        var hues = new[]
        {
            Color.FromRgb(0xE8, 0x77, 0x22),
            Color.FromRgb(0x42, 0xA5, 0xF5),
            Color.FromRgb(0x66, 0xBB, 0x6A),
            Color.FromRgb(0xAB, 0x47, 0xBC),
            Color.FromRgb(0xEF, 0x53, 0x50)
        };
        for (var i = _unit.Overlays.Count - 1; i >= 0; i--)
        {
            var slot = _unit.Overlays[i];
            var color = hues[i % hues.Length];
            if (!slot.Enabled)
                color = Color.FromRgb(0x55, 0x55, 0x55);
            var rect = new Rectangle
            {
                Width = Math.Max(8, slot.Width * WireCanvas.Width),
                Height = Math.Max(8, slot.Height * WireCanvas.Height),
                Stroke = new SolidColorBrush(color),
                StrokeThickness = ReferenceEquals(slot, _selected) ? 4 : 2,
                StrokeDashArray = slot.Enabled ? null : new DoubleCollection { 4, 3 },
                Fill = new SolidColorBrush(Color.FromArgb(slot.Enabled ? (byte)40 : (byte)20, color.R, color.G, color.B)),
                Tag = slot
            };
            Canvas.SetLeft(rect, slot.X * WireCanvas.Width);
            Canvas.SetTop(rect, slot.Y * WireCanvas.Height);
            WireCanvas.Children.Add(rect);
            var cropW = Math.Clamp(slot.CropWidth, 0.01f, 1f);
            var cropH = Math.Clamp(slot.CropHeight, 0.01f, 1f);
            var cropX = Math.Clamp(slot.CropX, 0f, 1f - cropW);
            var cropY = Math.Clamp(slot.CropY, 0f, 1f - cropH);
            if (cropX > 0.001f || cropY > 0.001f || cropW < 0.999f || cropH < 0.999f)
            {
                var crop = new Rectangle
                {
                    Width = Math.Max(4, rect.Width * cropW),
                    Height = Math.Max(4, rect.Height * cropH),
                    Stroke = new SolidColorBrush(color),
                    StrokeThickness = 1,
                    StrokeDashArray = new DoubleCollection { 3, 2 },
                    Fill = Brushes.Transparent,
                    IsHitTestVisible = false
                };
                Canvas.SetLeft(crop, Canvas.GetLeft(rect) + rect.Width * cropX);
                Canvas.SetTop(crop, Canvas.GetTop(rect) + rect.Height * cropY);
                WireCanvas.Children.Add(crop);
            }
            var label = new TextBlock
            {
                Text = (i + 1).ToString(),
                Foreground = Brushes.White,
                FontWeight = FontWeights.Bold,
                FontSize = 28,
                IsHitTestVisible = false
            };
            Canvas.SetLeft(label, Canvas.GetLeft(rect) + 8);
            Canvas.SetTop(label, Canvas.GetTop(rect) + 4);
            WireCanvas.Children.Add(label);
            if (ReferenceEquals(slot, _selected) && !slot.Locked)
            {
                var handle = new Rectangle
                {
                    Width = 16,
                    Height = 16,
                    Fill = new SolidColorBrush(color),
                    Tag = "handle"
                };
                Canvas.SetLeft(handle, Canvas.GetLeft(rect) + rect.Width - 16);
                Canvas.SetTop(handle, Canvas.GetTop(rect) + rect.Height - 16);
                WireCanvas.Children.Add(handle);
            }
        }
    }

    private void WriteCropBoxes()
    {
        if (_selected is null)
            return;
        WriteInsetBoxes(_selected);
    }

    private void WriteInsetBoxes(OverlaySlot slot)
    {
        CropInsets.FromRect(slot.CropX, slot.CropY, slot.CropWidth, slot.CropHeight, out var left, out var up, out var right, out var down);
        CropXBox.Text = (left * WidthPx).ToString("0.#");
        CropYBox.Text = (up * HeightPx).ToString("0.#");
        CropWBox.Text = (right * WidthPx).ToString("0.#");
        CropHBox.Text = (down * HeightPx).ToString("0.#");
    }

    private void FillFields()
    {
        if (_selected is null)
            return;
        _suppress = true;
        SourceKindBox.SelectedIndex = _selected.SourceKind == OverlaySourceKind.Input ? 1 : 0;
        FillSourceBox();
        XBox.Text = (_selected.X * WidthPx).ToString("0.#");
        YBox.Text = (_selected.Y * HeightPx).ToString("0.#");
        WBox.Text = (_selected.Width * WidthPx).ToString("0.#");
        HBox.Text = (_selected.Height * HeightPx).ToString("0.#");
        WriteInsetBoxes(_selected);
        OpBox.Text = _selected.Opacity.ToString("0.###");
        LinkBox.IsChecked = _selected.SizeLinked;
        KindBox.SelectedIndex = _selected.TransitionKind == MixerNative.TransitionCut ? 0 : 1;
        DurationBox.Text = _selected.DurationValue.ToString();
        DurationUnitBox.SelectedIndex = _selected.DurationUnit == MixerNative.DurationMs ? 1 : 0;
        var edit = !_selected.Locked;
        XBox.IsEnabled = edit;
        YBox.IsEnabled = edit;
        WBox.IsEnabled = edit;
        HBox.IsEnabled = edit;
        CropXBox.IsEnabled = edit;
        CropYBox.IsEnabled = edit;
        CropWBox.IsEnabled = edit;
        CropHBox.IsEnabled = edit;
        LinkBox.IsEnabled = edit;
        OpBox.IsEnabled = edit;
        SourceKindBox.IsEnabled = edit;
        SourceBox.IsEnabled = edit;
        _suppress = false;
    }

    private void FillAddSources()
    {
        var kind = AddKindBox.SelectedIndex == 1 ? OverlaySourceKind.Input : OverlaySourceKind.Scene;
        AddSourceBox.ItemsSource = kind == OverlaySourceKind.Input
            ? _session.Inputs
            : (System.Collections.IEnumerable)_session.Scenes;
        if (AddSourceBox.Items.Count > 0)
            AddSourceBox.SelectedIndex = 0;
    }

    private void FillSourceBox()
    {
        if (_selected is null)
            return;
        SourceBox.ItemsSource = _selected.SourceKind == OverlaySourceKind.Input
            ? _session.Inputs
            : (System.Collections.IEnumerable)_session.Scenes;
        SourceBox.SelectedItem = _selected.SourceKind == OverlaySourceKind.Input
            ? _session.Inputs.FirstOrDefault(item => item.Id == _selected.SceneGpuId)
            : _session.Scenes.FirstOrDefault(item => item.GpuId == _selected.SceneGpuId);
    }

    private void UpdatePreview() => PreviewHost.RefreshSize();

    private void Add_Click(object sender, RoutedEventArgs e)
    {
        if (_unit.Overlays.Count >= 8)
            return;
        var kind = AddKindBox.SelectedIndex == 1 ? OverlaySourceKind.Input : OverlaySourceKind.Scene;
        var id = kind == OverlaySourceKind.Input
            ? (AddSourceBox.SelectedItem as InputEntry)?.Id ?? _session.Inputs.FirstOrDefault()?.Id ?? 0UL
            : (AddSourceBox.SelectedItem as SceneEntry)?.GpuId ?? _session.Scenes.FirstOrDefault()?.GpuId ?? 0UL;
        var slot = new OverlaySlot { SourceKind = kind, SceneGpuId = id };
        _unit.Overlays.Insert(0, slot);
        _selected = slot;
        NormalizeOrder();
        RefreshList();
        Push();
    }

    private void Delete_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        _unit.Overlays.Remove(_selected);
        _selected = _unit.Overlays.FirstOrDefault();
        NormalizeOrder();
        RefreshList();
        Push();
    }

    private void ZUp_Click(object sender, RoutedEventArgs e) => ShiftDisplay(-1);

    private void ZDown_Click(object sender, RoutedEventArgs e) => ShiftDisplay(1);

    private void ShiftDisplay(int delta)
    {
        if (_selected is null)
            return;
        var index = _unit.Overlays.IndexOf(_selected);
        var target = index + delta;
        if (target < 0 || target >= _unit.Overlays.Count)
            return;
        (_unit.Overlays[index], _unit.Overlays[target]) = (_unit.Overlays[target], _unit.Overlays[index]);
        NormalizeOrder();
        RefreshList();
        Push();
    }

    private void SlotList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress)
            return;
        if (SlotList.SelectedItem is FrameworkElement { Tag: OverlaySlot slot })
            _selected = slot;
        else if (SlotList.SelectedIndex >= 0 && SlotList.SelectedIndex < _unit.Overlays.Count)
            _selected = _unit.Overlays[SlotList.SelectedIndex];
        else
            return;
        DrawWireframe();
        FillFields();
    }

    private void SlotAudio_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: OverlaySlot slot })
            return;
        slot.AudioFollow = !slot.AudioFollow;
        _selected = slot;
        RefreshList();
        Push();
        e.Handled = true;
    }

    private void Link_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        _selected.SizeLinked = LinkBox.IsChecked == true;
    }

    private void AddKind_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!IsLoaded)
            return;
        FillAddSources();
    }

    private void SourceKind_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || _selected.Locked)
            return;
        var kind = SourceKindBox.SelectedIndex == 1 ? OverlaySourceKind.Input : OverlaySourceKind.Scene;
        if (_selected.SourceKind == kind)
            return;
        _selected.SourceKind = kind;
        _selected.SceneGpuId = kind == OverlaySourceKind.Input
            ? _session.Inputs.FirstOrDefault()?.Id ?? 0UL
            : _session.Scenes.FirstOrDefault()?.GpuId ?? 0UL;
        RefreshList();
        Push();
        UpdatePreview();
    }

    private void SourceBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || _selected.Locked)
            return;
        if (_selected.SourceKind == OverlaySourceKind.Input && SourceBox.SelectedItem is InputEntry input)
            _selected.SceneGpuId = input.Id;
        else if (_selected.SourceKind == OverlaySourceKind.Scene && SourceBox.SelectedItem is SceneEntry scene)
            _selected.SceneGpuId = scene.GpuId;
        else
            return;
        RefreshList();
        Push();
        UpdatePreview();
    }

    private void SlotLock_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: OverlaySlot slot })
            return;
        slot.Locked = !slot.Locked;
        _selected = slot;
        RefreshList();
        e.Handled = true;
    }

    private void Reset_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null || _selected.Locked)
            return;
        _selected.ResetLayout();
        RefreshList();
        Push();
    }

    private void KindBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || KindBox.SelectedItem is not ComboBoxItem item)
            return;
        _selected.TransitionKind = ParseComboTag(item.Tag, MixerNative.TransitionFade);
        Push();
    }

    private void DurationUnit_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || DurationUnitBox.SelectedItem is not ComboBoxItem item)
            return;
        _selected.DurationUnit = ParseComboTag(item.Tag, MixerNative.DurationFrames);
        Push();
    }

    private static uint ParseComboTag(object? tag, uint fallback)
    {
        switch (tag)
        {
            case string text when uint.TryParse(text, out var parsed):
                return parsed;
            case uint value:
                return value;
            case int value when value >= 0:
                return (uint)value;
            default:
                try
                {
                    return Convert.ToUInt32(tag);
                }
                catch
                {
                    return fallback;
                }
        }
    }

    private void Duration_LostFocus(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        if (uint.TryParse(DurationBox.Text, out var value) && value > 0)
            _selected.DurationValue = value;
        Push();
    }

    private void Numeric_LostFocus(object sender, RoutedEventArgs e) => ApplyNumeric(true, sender);

    private void ApplyNumeric(bool push, object? sender)
    {
        if (_selected is null || _selected.Locked)
            return;
        if (float.TryParse(XBox.Text, out var x)) _selected.X = x / WidthPx;
        if (float.TryParse(YBox.Text, out var y)) _selected.Y = y / HeightPx;
        var fromW = ReferenceEquals(sender, WBox);
        var fromH = ReferenceEquals(sender, HBox);
        if (fromW && float.TryParse(WBox.Text, out var w))
        {
            var width = Math.Max(1f, w) / WidthPx;
            if (_selected.SizeLinked && _selected.Width > 0)
                _selected.Height = width * (_selected.Height / _selected.Width);
            _selected.Width = width;
        }
        else if (fromH && float.TryParse(HBox.Text, out var h))
        {
            var height = Math.Max(1f, h) / HeightPx;
            if (_selected.SizeLinked && _selected.Height > 0)
                _selected.Width = height * (_selected.Width / _selected.Height);
            _selected.Height = height;
        }
        else
        {
            if (float.TryParse(WBox.Text, out var bothW))
                _selected.Width = Math.Max(1f, bothW) / WidthPx;
            if (float.TryParse(HBox.Text, out var bothH))
                _selected.Height = Math.Max(1f, bothH) / HeightPx;
        }
        if (ReferenceEquals(sender, CropXBox) && float.TryParse(CropXBox.Text, out var cx))
            _selected.SetCropInset(CropEdit.Left, cx / WidthPx);
        else if (ReferenceEquals(sender, CropYBox) && float.TryParse(CropYBox.Text, out var cy))
            _selected.SetCropInset(CropEdit.Up, cy / HeightPx);
        else if (ReferenceEquals(sender, CropWBox) && float.TryParse(CropWBox.Text, out var cw))
            _selected.SetCropInset(CropEdit.Right, cw / WidthPx);
        else if (ReferenceEquals(sender, CropHBox) && float.TryParse(CropHBox.Text, out var ch))
            _selected.SetCropInset(CropEdit.Down, ch / HeightPx);
        else
        {
            CropInsets.FromRect(_selected.CropX, _selected.CropY, _selected.CropWidth, _selected.CropHeight, out var left, out var up, out var right, out var down);
            if (float.TryParse(CropXBox.Text, out var allCx)) left = allCx / WidthPx;
            if (float.TryParse(CropYBox.Text, out var allCy)) up = allCy / HeightPx;
            if (float.TryParse(CropWBox.Text, out var allCw)) right = allCw / WidthPx;
            if (float.TryParse(CropHBox.Text, out var allCh)) down = allCh / HeightPx;
            _selected.SetCropInset(CropEdit.Left, left);
            _selected.SetCropInset(CropEdit.Up, up);
            _selected.SetCropInset(CropEdit.Right, right);
            _selected.SetCropInset(CropEdit.Down, down);
        }
        if (float.TryParse(OpBox.Text, out var op)) _selected.Opacity = Math.Clamp(op, 0, 1);
        DrawWireframe();
        WriteCropBoxes();
        if (push)
        {
            FillFields();
            Push();
        }
        else
        {
            if (fromW)
                HBox.Text = (_selected.Height * HeightPx).ToString("0.#");
            else if (fromH)
                WBox.Text = (_selected.Width * WidthPx).ToString("0.#");
            if (DateTime.UtcNow - _lastGpuPush >= TimeSpan.FromMilliseconds(50))
            {
                _lastGpuPush = DateTime.UtcNow;
                Push();
            }
        }
    }

    private void WireCanvas_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        var pos = e.GetPosition(WireCanvas);
        _last = pos;
        _snapX = null;
        _snapY = null;
        if (e.OriginalSource is Rectangle { Tag: "handle" } && _selected is { Locked: false }
            && !Keyboard.Modifiers.HasFlag(ModifierKeys.Alt))
        {
            _resizing = true;
            _dragging = false;
            _cropping = false;
            WireCanvas.CaptureMouse();
            return;
        }
        var hit = HitSlot(pos);
        _selected = hit;
        _cropping = false;
        _resizing = false;
        _dragging = false;
        if (hit is { Locked: false } && Keyboard.Modifiers.HasFlag(ModifierKeys.Alt)
            && TryBeginCrop(hit, pos))
        {
            _cropping = true;
            WireCanvas.CaptureMouse();
        }
        else if (hit is { Locked: false })
        {
            _dragging = true;
            _grabX = (float)(pos.X / WireCanvas.Width) - hit.X;
            _grabY = (float)(pos.Y / WireCanvas.Height) - hit.Y;
            WireCanvas.CaptureMouse();
        }
        DrawWireframe();
        FillFields();
        UpdatePreview();
    }

    private bool TryBeginCrop(OverlaySlot slot, Point pos)
    {
        var left = slot.X * WireCanvas.Width;
        var top = slot.Y * WireCanvas.Height;
        var right = (slot.X + slot.Width) * WireCanvas.Width;
        var bottom = (slot.Y + slot.Height) * WireCanvas.Height;
        const double edge = 8;
        _cropLeft = Math.Abs(pos.X - left) <= edge;
        _cropRight = Math.Abs(pos.X - right) <= edge;
        _cropUp = Math.Abs(pos.Y - top) <= edge;
        _cropDown = Math.Abs(pos.Y - bottom) <= edge;
        return _cropLeft || _cropRight || _cropUp || _cropDown;
    }

    private OverlaySlot? HitSlot(Point pos)
    {
        var hits = _unit.Overlays.Where(slot =>
            pos.X >= slot.X * WireCanvas.Width
            && pos.X <= (slot.X + slot.Width) * WireCanvas.Width
            && pos.Y >= slot.Y * WireCanvas.Height
            && pos.Y <= (slot.Y + slot.Height) * WireCanvas.Height).ToList();
        if (hits.Count == 0)
            return null;
        if (_selected is not null && hits.Contains(_selected))
            return _selected;
        return hits.OrderByDescending(item => item.Z).First();
    }

    private void WireCanvas_MouseMove(object sender, MouseEventArgs e)
    {
        if (_selected is null || (!_dragging && !_resizing && !_cropping) || e.LeftButton != MouseButtonState.Pressed)
            return;
        var pos = e.GetPosition(WireCanvas);
        var dx = (float)((pos.X - _last.X) / WireCanvas.Width);
        var dy = (float)((pos.Y - _last.Y) / WireCanvas.Height);
        _last = pos;
        if (_selected.Locked)
            return;
        if (_cropping)
            ApplyCropDrag(_selected, dx, dy);
        else if (_resizing)
        {
            var width = Math.Max(0.02f, _selected.Width + dx);
            if (_selected.SizeLinked && _selected.Width > 0)
                _selected.Height = Math.Max(0.02f, width * (_selected.Height / _selected.Width));
            else
                _selected.Height = Math.Max(0.02f, _selected.Height + dy);
            _selected.Width = width;
            ApplyResizeSnap();
        }
        else
        {
            var x = (float)(pos.X / WireCanvas.Width) - _grabX;
            var y = (float)(pos.Y / WireCanvas.Height) - _grabY;
            ApplyMoveSnap(ref x, ref y);
            _selected.X = x;
            _selected.Y = y;
        }
        DrawWireframe();
        FillFields();
        if (DateTime.UtcNow - _lastGpuPush >= TimeSpan.FromMilliseconds(50))
        {
            _lastGpuPush = DateTime.UtcNow;
            Push();
        }
    }

    private void ApplyCropDrag(OverlaySlot slot, float dx, float dy)
    {
        if (slot.Width <= 0 || slot.Height <= 0)
            return;
        if (_cropLeft)
            slot.SetCropInset(CropEdit.Left, slot.CropX + dx / slot.Width);
        if (_cropRight)
            slot.SetCropInset(CropEdit.Right, 1f - slot.CropX - slot.CropWidth - dx / slot.Width);
        if (_cropUp)
            slot.SetCropInset(CropEdit.Up, slot.CropY + dy / slot.Height);
        if (_cropDown)
            slot.SetCropInset(CropEdit.Down, 1f - slot.CropY - slot.CropHeight - dy / slot.Height);
    }

    private void LayoutSnap_Click(object sender, RoutedEventArgs e)
    {
        _snapX = null;
        _snapY = null;
    }

    private bool LayoutSnapOn => LayoutSnapButton.IsChecked == true;

    private void ApplyMoveSnap(ref float x, ref float y)
    {
        if (_selected is null || !LayoutSnapOn)
            return;
        var rendered = SceneSnap.RenderedSize(this, WireCanvas);
        var boxes = _unit.Overlays.Select(slot => new SceneSnap.Box(
            slot.X, slot.Y, slot.Width, slot.Height, slot.Hidden || !slot.Enabled, ReferenceEquals(slot, _selected))).ToList();
        x = SceneSnap.LatchMoveAxis(x, _selected.Width, boxes, true, rendered.Width, ref _snapX);
        y = SceneSnap.LatchMoveAxis(y, _selected.Height, boxes, false, rendered.Height, ref _snapY);
    }

    private void ApplyResizeSnap()
    {
        if (_selected is null || !LayoutSnapOn)
            return;
        var rendered = SceneSnap.RenderedSize(this, WireCanvas);
        var boxes = _unit.Overlays.Select(slot => new SceneSnap.Box(
            slot.X, slot.Y, slot.Width, slot.Height, slot.Hidden || !slot.Enabled, ReferenceEquals(slot, _selected))).ToList();
        var width = _selected.Width;
        var height = _selected.Height;
        SceneSnap.SnapResize(
            ref width,
            ref height,
            _selected.X,
            _selected.Y,
            _selected.SizeLinked,
            boxes,
            (float)(SceneSnap.EngagePixels / Math.Max(rendered.Width, 1)),
            (float)(SceneSnap.EngagePixels / Math.Max(rendered.Height, 1)));
        _selected.Width = width;
        _selected.Height = height;
    }

    private void WireCanvas_MouseLeftButtonUp(object sender, MouseButtonEventArgs e)
    {
        if (_dragging || _resizing || _cropping)
            Push();
        _dragging = false;
        _resizing = false;
        _cropping = false;
        _snapX = null;
        _snapY = null;
        WireCanvas.ReleaseMouseCapture();
    }

    private void WireCanvas_MouseRightButtonUp(object sender, MouseButtonEventArgs e)
    {
        var pos = e.GetPosition(WireCanvas);
        var hit = HitSlot(pos);
        if (hit is null)
            return;
        _selected = hit;
        RefreshList();
        var menu = new ContextMenu();
        var fit = new MenuItem
        {
            Header = Loc.T("scene.fitToScreen"),
            IsEnabled = !hit.Locked
        };
        fit.Click += (_, _) => FitSlotToScreen(hit);
        menu.Items.Add(fit);
        menu.PlacementTarget = WireCanvas;
        menu.IsOpen = true;
        e.Handled = true;
    }

    private void FitSlotToScreen(OverlaySlot slot)
    {
        if (slot.Locked)
            return;
        slot.X = 0;
        slot.Y = 0;
        slot.Width = 1;
        slot.Height = 1;
        slot.SizeLinked = true;
        slot.CropX = 0;
        slot.CropY = 0;
        slot.CropWidth = 1;
        slot.CropHeight = 1;
        _selected = slot;
        RefreshList();
        Push();
    }
}
