using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

public partial class OverlayWindow : Window
{
    private readonly Session _session;
    private MixingUnitEntry _unit;
    private readonly ulong _monitorId;
    private OverlaySlot? _selected;
    private bool _dragging;
    private bool _resizing;
    private Point _last;
    private bool _suppress;

    public OverlayWindow(Session session, MixingUnitEntry unit)
    {
        InitializeComponent();
        _session = session;
        _unit = unit;
        _monitorId = session.NextMonitorId++;
        SceneBox.ItemsSource = session.Scenes;
        Title = $"Overlays — {unit.Name}";
        Loaded += (_, _) => RefreshList();
    }

    public void Reload(MixingUnitEntry unit)
    {
        _unit = unit;
        Title = $"Overlays — {unit.Name}";
        _selected = unit.Overlays.FirstOrDefault();
        RefreshList();
    }

    private void Push()
    {
        ((App)Application.Current).Commands.TryEnqueue(new PatchAuxCommand(_unit.Id, _unit));
        if (Owner is MainWindow main)
            main.RebuildOverlayToggles();
    }

    private void RefreshList()
    {
        _suppress = true;
        SlotList.ItemsSource = null;
        SlotList.ItemsSource = _unit.Overlays.Select((slot, index) =>
            $"{(slot.Enabled ? "ON" : "off")}  {index + 1}: {SceneName(slot)}").ToArray();
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

    private string SceneName(OverlaySlot slot) =>
        _session.Scenes.FirstOrDefault(item => item.GpuId == slot.SceneGpuId)?.Name ?? "(none)";

    private void DrawWireframe()
    {
        WireCanvas.Children.Clear();
        var hues = new[]
        {
            Color.FromRgb(0xE8, 0x77, 0x22),
            Color.FromRgb(0x42, 0xA5, 0xF5),
            Color.FromRgb(0x66, 0xBB, 0x6A),
            Color.FromRgb(0xAB, 0x47, 0xBC)
        };
        for (var i = 0; i < _unit.Overlays.Count; i++)
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
                Fill = new SolidColorBrush(Color.FromArgb(40, color.R, color.G, color.B)),
                Tag = slot
            };
            Canvas.SetLeft(rect, slot.X * WireCanvas.Width);
            Canvas.SetTop(rect, slot.Y * WireCanvas.Height);
            WireCanvas.Children.Add(rect);
            if (ReferenceEquals(slot, _selected))
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

    private void FillFields()
    {
        if (_selected is null)
            return;
        _suppress = true;
        OnBox.IsChecked = _selected.Enabled;
        SceneBox.SelectedItem = _session.Scenes.FirstOrDefault(item => item.GpuId == _selected.SceneGpuId);
        XBox.Text = _selected.X.ToString("0.####");
        YBox.Text = _selected.Y.ToString("0.####");
        WBox.Text = _selected.Width.ToString("0.####");
        HBox.Text = _selected.Height.ToString("0.####");
        OpBox.Text = _selected.Opacity.ToString("0.###");
        KindBox.SelectedIndex = _selected.TransitionKind == MixerNative.TransitionCut ? 0 : 1;
        DurationBox.Text = _selected.DurationValue.ToString();
        DurationUnitBox.SelectedIndex = _selected.DurationUnit == MixerNative.DurationMs ? 1 : 0;
        _suppress = false;
    }

    private void UpdatePreview()
    {
        if (_selected is null)
            return;
        if (PreviewHost.MonitorId == 0)
            PreviewHost.RetargetMonitor(_monitorId, _selected.SceneGpuId);
        else
            PreviewHost.UpdateMonitorSource(_selected.SceneGpuId);
    }

    private void Commit()
    {
        DrawWireframe();
        Push();
        RefreshList();
    }

    private void Add_Click(object sender, RoutedEventArgs e)
    {
        if (_unit.Overlays.Count >= 8)
            return;
        var sceneId = _session.Scenes.Count > 0 ? _session.Scenes[0].GpuId : 0UL;
        var slot = new OverlaySlot { SceneGpuId = sceneId, Z = _unit.Overlays.Count, Enabled = true };
        _unit.Overlays.Add(slot);
        _selected = slot;
        Commit();
    }

    private void Delete_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        _unit.Overlays.Remove(_selected);
        _selected = _unit.Overlays.LastOrDefault();
        Commit();
    }

    private void ZUp_Click(object sender, RoutedEventArgs e) => ShiftZ(1);

    private void ZDown_Click(object sender, RoutedEventArgs e) => ShiftZ(-1);

    private void ShiftZ(int delta)
    {
        if (_selected is null)
            return;
        var index = _unit.Overlays.IndexOf(_selected);
        var target = index + delta;
        if (target < 0 || target >= _unit.Overlays.Count)
            return;
        (_unit.Overlays[index], _unit.Overlays[target]) = (_unit.Overlays[target], _unit.Overlays[index]);
        Commit();
    }

    private void SlotList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || SlotList.SelectedIndex < 0 || SlotList.SelectedIndex >= _unit.Overlays.Count)
            return;
        _selected = _unit.Overlays[SlotList.SelectedIndex];
        DrawWireframe();
        FillFields();
        UpdatePreview();
    }

    private void OnBox_Changed(object sender, RoutedEventArgs e)
    {
        if (_suppress || _selected is null)
            return;
        _selected.Enabled = OnBox.IsChecked == true;
        Commit();
    }

    private void SceneBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || SceneBox.SelectedItem is not SceneEntry scene)
            return;
        _selected.SceneGpuId = scene.GpuId;
        Commit();
    }

    private void KindBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || KindBox.SelectedItem is not ComboBoxItem item)
            return;
        _selected.TransitionKind = item.Tag is string text && uint.TryParse(text, out var value)
            ? value
            : MixerNative.TransitionFade;
        Push();
    }

    private void DurationUnit_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || DurationUnitBox.SelectedItem is not ComboBoxItem item)
            return;
        _selected.DurationUnit = item.Tag is string text && uint.TryParse(text, out var value)
            ? value
            : MixerNative.DurationFrames;
        Push();
    }

    private void Duration_LostFocus(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        if (uint.TryParse(DurationBox.Text, out var value) && value > 0)
            _selected.DurationValue = value;
        Push();
    }

    private void Numeric_LostFocus(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        if (float.TryParse(XBox.Text, out var x)) _selected.X = x;
        if (float.TryParse(YBox.Text, out var y)) _selected.Y = y;
        if (float.TryParse(WBox.Text, out var w)) _selected.Width = Math.Max(0.01f, w);
        if (float.TryParse(HBox.Text, out var h)) _selected.Height = Math.Max(0.01f, h);
        if (float.TryParse(OpBox.Text, out var op)) _selected.Opacity = Math.Clamp(op, 0, 1);
        Commit();
    }

    private void WireCanvas_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        var pos = e.GetPosition(WireCanvas);
        if (e.OriginalSource is Rectangle { Tag: "handle" })
        {
            _resizing = true;
            _last = pos;
            WireCanvas.CaptureMouse();
            return;
        }
        OverlaySlot? hit = null;
        foreach (var slot in _unit.Overlays.AsEnumerable().Reverse())
        {
            if (pos.X >= slot.X * WireCanvas.Width
                && pos.X <= (slot.X + slot.Width) * WireCanvas.Width
                && pos.Y >= slot.Y * WireCanvas.Height
                && pos.Y <= (slot.Y + slot.Height) * WireCanvas.Height)
            {
                hit = slot;
                break;
            }
        }
        _selected = hit;
        _dragging = hit is not null;
        _last = pos;
        if (_dragging)
            WireCanvas.CaptureMouse();
        DrawWireframe();
        FillFields();
        UpdatePreview();
    }

    private void WireCanvas_MouseMove(object sender, MouseEventArgs e)
    {
        if (_selected is null || (!_dragging && !_resizing) || e.LeftButton != MouseButtonState.Pressed)
            return;
        var pos = e.GetPosition(WireCanvas);
        var dx = (float)((pos.X - _last.X) / WireCanvas.Width);
        var dy = (float)((pos.Y - _last.Y) / WireCanvas.Height);
        _last = pos;
        if (_resizing)
        {
            _selected.Width = Math.Max(0.02f, _selected.Width + dx);
            _selected.Height = Math.Max(0.02f, _selected.Height + dy);
        }
        else
        {
            _selected.X += dx;
            _selected.Y += dy;
        }
        DrawWireframe();
        FillFields();
    }

    private void WireCanvas_MouseLeftButtonUp(object sender, MouseButtonEventArgs e)
    {
        if (_dragging || _resizing)
            Push();
        _dragging = false;
        _resizing = false;
        WireCanvas.ReleaseMouseCapture();
    }
}
