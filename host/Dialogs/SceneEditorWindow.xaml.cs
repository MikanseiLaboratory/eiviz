using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using Eiviz.Host;

namespace Eiviz.Host.Dialogs;

public partial class SceneEditorWindow : Window
{
    private readonly SceneEntry _scene;
    private readonly Session _session;
    private readonly CommandQueue _commands;
    private readonly uint _width;
    private readonly uint _height;
    private readonly List<SceneLayer> _original;
    private SceneLayer? _selected;
    private bool _dragging;
    private bool _resizing;
    private Point _last;

    public SceneEditorWindow(SceneEntry scene, Session session, uint width, uint height, ulong monitorId)
    {
        InitializeComponent();
        _scene = scene;
        _session = session;
        _commands = ((App)Application.Current).Commands;
        _width = width;
        _height = height;
        _original = scene.Layers.Select(Clone).ToList();
        NameBox.Text = scene.Name;
        WireCanvas.Width = width;
        WireCanvas.Height = height;
        WireLabel.Text = $"Wireframe ({width}x{height})";
        InputPick.ItemsSource = session.Inputs;
        if (session.Inputs.Count > 0)
            InputPick.SelectedIndex = 0;
        PreviewHost.RetargetMonitor(monitorId, scene.GpuId);
        Loaded += (_, _) =>
        {
            FillPresets();
            RefreshLayers();
            PushGpu();
        };
    }

    private static SceneLayer Clone(SceneLayer layer) => new()
    {
        InputId = layer.InputId,
        X = layer.X,
        Y = layer.Y,
        Width = layer.Width,
        Height = layer.Height,
        Opacity = layer.Opacity,
        Z = layer.Z,
        AudioFollow = layer.AudioFollow,
        Locked = layer.Locked,
        SizeLinked = layer.SizeLinked
    };

    private void RefreshLayers()
    {
        _scene.Layers.Sort((a, b) => a.Z.CompareTo(b.Z));
        LayerList.ItemsSource = null;
        LayerList.ItemsSource = _scene.Layers.Select(LayerLabel).ToArray();
        if (_selected is not null)
        {
            var index = _scene.Layers.IndexOf(_selected);
            if (index >= 0)
                LayerList.SelectedIndex = index;
        }
        DrawWireframe();
        FillNumeric();
    }

    private string LayerLabel(SceneLayer layer)
    {
        var input = _session.Inputs.FirstOrDefault(item => item.Id == layer.InputId);
        return $"{layer.Z}: {input?.Name ?? layer.InputId.ToString()}{(layer.Locked ? " 🔒" : "")}";
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
        for (var i = 0; i < _scene.Layers.Count; i++)
        {
            var layer = _scene.Layers[i];
            var color = hues[i % hues.Length];
            var rect = new Rectangle
            {
                Width = Math.Max(8, layer.Width * WireCanvas.Width),
                Height = Math.Max(8, layer.Height * WireCanvas.Height),
                Stroke = new SolidColorBrush(color),
                StrokeThickness = ReferenceEquals(layer, _selected) ? 4 : 2,
                Fill = new SolidColorBrush(Color.FromArgb(40, color.R, color.G, color.B)),
                Tag = layer
            };
            Canvas.SetLeft(rect, layer.X * WireCanvas.Width);
            Canvas.SetTop(rect, layer.Y * WireCanvas.Height);
            WireCanvas.Children.Add(rect);
            if (ReferenceEquals(layer, _selected))
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

    private void FillNumeric()
    {
        if (_selected is null)
            return;
        XBox.Text = (_selected.X * _width).ToString("0.#");
        YBox.Text = (_selected.Y * _height).ToString("0.#");
        WBox.Text = (_selected.Width * _width).ToString("0.#");
        HBox.Text = (_selected.Height * _height).ToString("0.#");
        ZBox.Text = _selected.Z.ToString();
        OpBox.Text = _selected.Opacity.ToString("0.###");
        AudioFollowBox.IsChecked = _selected.AudioFollow;
        LinkBox.IsChecked = _selected.SizeLinked;
        LockBox.IsChecked = _selected.Locked;
        var edit = !_selected.Locked;
        XBox.IsEnabled = edit;
        YBox.IsEnabled = edit;
        WBox.IsEnabled = edit;
        HBox.IsEnabled = edit;
        LinkBox.IsEnabled = edit;
        OpBox.IsEnabled = edit;
    }

    private void PushGpu() => _commands.DefineSceneNow(_scene, _width, _height);

    private void AddLayer_Click(object sender, RoutedEventArgs e)
    {
        if (InputPick.SelectedItem is not InputEntry input)
            return;
        var layer = new SceneLayer
        {
            InputId = input.Id,
            Width = 1,
            Height = 1,
            Opacity = 1,
            AudioFollow = true,
            SizeLinked = true,
            Z = _scene.Layers.Count == 0 ? 0 : _scene.Layers.Max(item => item.Z) + 1
        };
        _scene.Layers.Add(layer);
        _selected = layer;
        RefreshLayers();
        PushGpu();
    }

    private void DeleteLayer_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        _scene.Layers.Remove(_selected);
        _selected = _scene.Layers.LastOrDefault();
        RefreshLayers();
        PushGpu();
    }

    private void ZUp_Click(object sender, RoutedEventArgs e) => ShiftZ(1);

    private void ZDown_Click(object sender, RoutedEventArgs e) => ShiftZ(-1);

    private void ShiftZ(int delta)
    {
        if (_selected is null)
            return;
        var index = _scene.Layers.IndexOf(_selected);
        var target = index + delta;
        if (target < 0 || target >= _scene.Layers.Count)
            return;
        (_scene.Layers[index].Z, _scene.Layers[target].Z) = (_scene.Layers[target].Z, _scene.Layers[index].Z);
        RefreshLayers();
        PushGpu();
    }

    private void LayerList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (LayerList.SelectedIndex >= 0 && LayerList.SelectedIndex < _scene.Layers.Count)
        {
            _selected = _scene.Layers[LayerList.SelectedIndex];
            DrawWireframe();
            FillNumeric();
        }
    }

    private void Numeric_LostFocus(object sender, RoutedEventArgs e)
    {
        if (_selected is null || _selected.Locked)
            return;
        if (float.TryParse(XBox.Text, out var x)) _selected.X = x / _width;
        if (float.TryParse(YBox.Text, out var y)) _selected.Y = y / _height;
        if (float.TryParse(WBox.Text, out var w))
        {
            var width = Math.Max(1f, w) / _width;
            if (_selected.SizeLinked && _selected.Width > 0)
                _selected.Height = width * (_selected.Height / _selected.Width);
            _selected.Width = width;
        }
        if (float.TryParse(HBox.Text, out var h))
        {
            var height = Math.Max(1f, h) / _height;
            if (_selected.SizeLinked && _selected.Height > 0 && !ReferenceEquals(sender, WBox))
                _selected.Width = height * (_selected.Width / _selected.Height);
            _selected.Height = height;
        }
        if (int.TryParse(ZBox.Text, out var z)) _selected.Z = z;
        if (float.TryParse(OpBox.Text, out var op)) _selected.Opacity = Math.Clamp(op, 0, 1);
        DrawWireframe();
        FillNumeric();
        PushGpu();
    }

    private void AudioFollow_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        _selected.AudioFollow = AudioFollowBox.IsChecked == true;
        PushGpu();
    }

    private void WireCanvas_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        var pos = e.GetPosition(WireCanvas);
        if (e.OriginalSource is Rectangle { Tag: "handle" } && _selected is { Locked: false })
        {
            _resizing = true;
            _dragging = false;
            _last = pos;
            WireCanvas.CaptureMouse();
            return;
        }
        SceneLayer? hit = null;
        foreach (var layer in _scene.Layers.OrderByDescending(item => item.Z))
        {
            if (pos.X >= layer.X * WireCanvas.Width
                && pos.X <= (layer.X + layer.Width) * WireCanvas.Width
                && pos.Y >= layer.Y * WireCanvas.Height
                && pos.Y <= (layer.Y + layer.Height) * WireCanvas.Height)
            {
                hit = layer;
                break;
            }
        }
        _selected = hit;
        _dragging = hit is not null && hit is { Locked: false };
        _resizing = false;
        _last = pos;
        if (_dragging)
            WireCanvas.CaptureMouse();
        RefreshLayers();
    }

    private void WireCanvas_MouseMove(object sender, MouseEventArgs e)
    {
        if (_selected is null || (!_dragging && !_resizing) || e.LeftButton != MouseButtonState.Pressed)
            return;
        var pos = e.GetPosition(WireCanvas);
        var dx = (float)((pos.X - _last.X) / WireCanvas.Width);
        var dy = (float)((pos.Y - _last.Y) / WireCanvas.Height);
        _last = pos;
        if (_selected.Locked)
            return;
        if (_resizing)
        {
            var width = Math.Max(0.02f, _selected.Width + dx);
            if (_selected.SizeLinked && _selected.Width > 0)
                _selected.Height = Math.Max(0.02f, width * (_selected.Height / _selected.Width));
            else
                _selected.Height = Math.Max(0.02f, _selected.Height + dy);
            _selected.Width = width;
        }
        else
        {
            _selected.X += dx;
            _selected.Y += dy;
        }
        DrawWireframe();
        FillNumeric();
    }

    private void WireCanvas_MouseLeftButtonUp(object sender, MouseButtonEventArgs e)
    {
        if (_dragging || _resizing)
            PushGpu();
        _dragging = false;
        _resizing = false;
        WireCanvas.ReleaseMouseCapture();
    }

    private void Link_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        _selected.SizeLinked = LinkBox.IsChecked == true;
    }

    private void Lock_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null)
            return;
        _selected.Locked = LockBox.IsChecked == true;
        RefreshLayers();
    }

    private void FillPresets()
    {
        PresetBox.Items.Clear();
        PresetBox.Items.Add("Full");
        PresetBox.Items.Add("Split H");
        PresetBox.Items.Add("Split V");
        PresetBox.Items.Add("Quad");
        PresetBox.Items.Add("PiP TR");
        PresetBox.Items.Add("PiP TL");
        PresetBox.Items.Add("PiP BR");
        PresetBox.Items.Add("PiP BL");
        foreach (var preset in _session.ScenePresets)
            PresetBox.Items.Add(preset.Name);
        CopyFromBox.ItemsSource = _session.Scenes.Where(item => item.Id != _scene.Id).Select(item => item.Name).ToArray();
    }

    private void Preset_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (PresetBox.SelectedItem is not string name)
            return;
        ApplyPreset(name);
        PresetBox.SelectedIndex = -1;
    }

    private void CopyFrom_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (CopyFromBox.SelectedItem is not string name)
            return;
        var source = _session.Scenes.FirstOrDefault(item => item.Name == name);
        if (source is null)
            return;
        _scene.Layers.Clear();
        foreach (var layer in source.Layers)
            _scene.Layers.Add(Clone(layer));
        _selected = _scene.Layers.FirstOrDefault();
        RefreshLayers();
        PushGpu();
        CopyFromBox.SelectedIndex = -1;
    }

    private void SavePreset_Click(object sender, RoutedEventArgs e)
    {
        var name = $"Preset {_session.ScenePresets.Count + 1}";
        _session.ScenePresets.Add(new SceneLayoutPreset
        {
            Name = name,
            Layers = _scene.Layers.Select(Clone).ToList()
        });
        FillPresets();
    }

    private void ApplyPreset(string name)
    {
        var user = _session.ScenePresets.FirstOrDefault(item => item.Name == name);
        if (user is not null)
        {
            for (var i = 0; i < _scene.Layers.Count && i < user.Layers.Count; i++)
            {
                if (_scene.Layers[i].Locked)
                    continue;
                var src = user.Layers[i];
                _scene.Layers[i].X = src.X;
                _scene.Layers[i].Y = src.Y;
                _scene.Layers[i].Width = src.Width;
                _scene.Layers[i].Height = src.Height;
                _scene.Layers[i].Opacity = src.Opacity;
                _scene.Layers[i].Z = src.Z;
            }
            RefreshLayers();
            PushGpu();
            return;
        }
        var boxes = name switch
        {
            "Full" => new[] { (0f, 0f, 1f, 1f) },
            "Split H" => new[] { (0f, 0f, 0.5f, 1f), (0.5f, 0f, 0.5f, 1f) },
            "Split V" => new[] { (0f, 0f, 1f, 0.5f), (0f, 0.5f, 1f, 0.5f) },
            "Quad" => new[] { (0f, 0f, 0.5f, 0.5f), (0.5f, 0f, 0.5f, 0.5f), (0f, 0.5f, 0.5f, 0.5f), (0.5f, 0.5f, 0.5f, 0.5f) },
            "PiP TR" => new[] { (0f, 0f, 1f, 1f), (0.62f, 0.08f, 0.32f, 0.32f) },
            "PiP TL" => new[] { (0f, 0f, 1f, 1f), (0.06f, 0.08f, 0.32f, 0.32f) },
            "PiP BR" => new[] { (0f, 0f, 1f, 1f), (0.62f, 0.60f, 0.32f, 0.32f) },
            "PiP BL" => new[] { (0f, 0f, 1f, 1f), (0.06f, 0.60f, 0.32f, 0.32f) },
            _ => Array.Empty<(float, float, float, float)>()
        };
        for (var i = 0; i < _scene.Layers.Count && i < boxes.Length; i++)
        {
            if (_scene.Layers[i].Locked)
                continue;
            var (x, y, w, h) = boxes[i];
            _scene.Layers[i].X = x;
            _scene.Layers[i].Y = y;
            _scene.Layers[i].Width = w;
            _scene.Layers[i].Height = h;
        }
        RefreshLayers();
        PushGpu();
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        _scene.Name = string.IsNullOrWhiteSpace(NameBox.Text) ? _scene.Name : NameBox.Text.Trim();
        PushGpu();
        DialogResult = true;
    }

    protected override void OnClosed(EventArgs e)
    {
        if (DialogResult != true)
        {
            _scene.Layers.Clear();
            _scene.Layers.AddRange(_original);
            PushGpu();
        }
        base.OnClosed(e);
    }
}
