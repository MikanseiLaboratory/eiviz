using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using System.Windows.Threading;
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
    private bool _suppress;
    private Point _last;
    private DateTime _lastGpuPush;
    private TextBox? _meterBox;
    private string _meterFormat = "0.#";
    private TagCheckPanel? _tags;

    public SceneEditorWindow(SceneEntry scene, Session session, uint width, uint height, ulong monitorId)
    {
        InitializeComponent();
        _scene = scene;
        _session = session;
        _commands = ((App)Application.Current).Commands;
        _width = width;
        _height = height;
        _original = scene.Layers.Select(Clone).ToList();
        _selected = scene.Layers.FirstOrDefault();
        NameBox.Text = scene.Name;
        _tags = new TagCheckPanel(TagPanel, session.SceneTags, scene.Tags, this);
        WireCanvas.Width = width;
        WireCanvas.Height = height;
        WireLabel.Text = $"Wireframe ({width}x{height})";
        InputPick.ItemsSource = session.Inputs;
        LayerInputBox.ItemsSource = session.Inputs;
        if (session.Inputs.Count > 0)
            InputPick.SelectedIndex = 0;
        PreviewAspect.RatioWidth = width;
        PreviewAspect.RatioHeight = height;
        Loaded += (_, _) =>
        {
            FillPresets();
            _scene.Layers.Sort((a, b) => b.Z.CompareTo(a.Z));
            NormalizeOrder();
            RefreshLayers();
            PushGpu();
            AttachDrags();
            ListReorder.Attach(LayerList, MoveLayer);
            Dispatcher.BeginInvoke(
                () => PreviewHost.RetargetMonitor(monitorId, scene.GpuId),
                DispatcherPriority.Loaded);
        };
    }

    private void AttachDrags()
    {
        void Bind(FrameworkElement handle, TextBox box, float scale, string format = "0.#", double min = 0, double max = 4096, Func<double>? maxOf = null)
        {
            void Preview() => ApplyNumeric(false, box);
            void Commit() => ApplyNumeric(true, box);
            NumericDrag.Attach(handle, box, scale, Preview, Commit, format, () => ToggleMeter((handle as TextBlock)?.Text ?? "Value", box, min, maxOf?.Invoke() ?? max, format));
            NumericDrag.AttachBox(box, scale, Preview, Commit, format);
        }
        Bind(PosXLabel, XBox, 2, min: -_width, max: _width * 2);
        Bind(PosYLabel, YBox, 2, min: -_height, max: _height * 2);
        Bind(SizeXLabel, WBox, 2, min: 1, max: _width * 2);
        Bind(SizeYLabel, HBox, 2, min: 1, max: _height * 2);
        Bind(CropXLabel, CropXBox, 2, min: 0, max: _width);
        Bind(CropYLabel, CropYBox, 2, min: 0, max: _height);
        Bind(CropWLabel, CropWBox, 2, min: 0, max: _width);
        Bind(CropHLabel, CropHBox, 2, min: 0, max: _height);
        Bind(OpLabel, OpBox, 400, "0.###", 0, 1);
    }

    private void ToggleMeter(string title, TextBox box, double min, double max, string format)
    {
        if (ReferenceEquals(_meterBox, box) && MeterHost.Visibility == Visibility.Visible)
        {
            MeterHost.Visibility = Visibility.Collapsed;
            _meterBox = null;
            return;
        }
        _meterBox = box;
        _meterFormat = format;
        MeterTitle.Text = title;
        MeterSlider.Minimum = min;
        MeterSlider.Maximum = Math.Max(max, min);
        _suppress = true;
        if (float.TryParse(box.Text, out var value))
            MeterSlider.Value = Math.Clamp(value, MeterSlider.Minimum, MeterSlider.Maximum);
        _suppress = false;
        MeterHost.Visibility = Visibility.Visible;
    }

    private void MeterSlider_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (_suppress || _meterBox is null)
            return;
        _meterBox.Text = e.NewValue.ToString(_meterFormat);
        ApplyNumeric(false, _meterBox);
    }

    private void MeterSlider_MouseUp(object sender, MouseButtonEventArgs e)
    {
        if (_meterBox is not null)
            ApplyNumeric(true, _meterBox);
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
        Hidden = layer.Hidden,
        SizeLinked = layer.SizeLinked,
        CropX = layer.CropX,
        CropY = layer.CropY,
        CropWidth = layer.CropWidth,
        CropHeight = layer.CropHeight
    };

    private void NormalizeOrder()
    {
        for (var i = 0; i < _scene.Layers.Count; i++)
            _scene.Layers[i].Z = _scene.Layers.Count - 1 - i;
    }

    private bool MoveLayer(int from, int to)
    {
        if (from < 0 || from >= _scene.Layers.Count)
            return false;
        to = Math.Clamp(to, 0, _scene.Layers.Count);
        if (to == from || to == from + 1)
            return false;
        var layer = _scene.Layers[from];
        _scene.Layers.RemoveAt(from);
        if (to > from)
            to--;
        _scene.Layers.Insert(to, layer);
        _selected = layer;
        NormalizeOrder();
        RefreshLayers();
        PushGpu();
        return true;
    }

    private void RefreshLayers()
    {
        LayerList.Items.Clear();
        foreach (var layer in _scene.Layers)
            LayerList.Items.Add(BuildLayerRow(layer));
        if (_selected is not null)
        {
            var index = _scene.Layers.IndexOf(_selected);
            if (index >= 0)
                LayerList.SelectedIndex = index;
        }
        DrawWireframe();
        FillNumeric();
    }

    private DockPanel BuildLayerRow(SceneLayer layer)
    {
        var hide = new Button
        {
            Content = layer.Hidden ? "–" : "👁",
            Width = 26,
            Height = 22,
            Padding = new Thickness(0),
            Tag = layer,
            ToolTip = "Hide"
        };
        var lockBtn = new Button
        {
            Content = layer.Locked ? "🔒" : "🔓",
            Width = 26,
            Height = 22,
            Padding = new Thickness(0),
            Margin = new Thickness(4, 0, 0, 0),
            Tag = layer,
            ToolTip = "Lock"
        };
        var audio = new Button
        {
            Content = layer.AudioFollow ? "🔊" : "🔇",
            Width = 26,
            Height = 22,
            Padding = new Thickness(0),
            Margin = new Thickness(4, 0, 0, 0),
            Tag = layer,
            ToolTip = "Audio Follow"
        };
        hide.Click += LayerHide_Click;
        lockBtn.Click += LayerLock_Click;
        audio.Click += LayerAudioFollow_Click;
        DockPanel.SetDock(hide, Dock.Left);
        DockPanel.SetDock(audio, Dock.Left);
        DockPanel.SetDock(lockBtn, Dock.Left);
        var name = new TextBlock
        {
            Text = LayerLabel(layer),
            Margin = new Thickness(8, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
            Foreground = layer.Hidden ? new SolidColorBrush(Color.FromRgb(0x88, 0x88, 0x88)) : Brushes.White
        };
        var row = new DockPanel { Tag = layer, LastChildFill = true };
        row.Children.Add(hide);
        row.Children.Add(audio);
        row.Children.Add(lockBtn);
        row.Children.Add(name);
        return row;
    }

    private string LayerLabel(SceneLayer layer)
    {
        var input = _session.Inputs.FirstOrDefault(item => item.Id == layer.InputId);
        var order = _scene.Layers.IndexOf(layer) + 1;
        return $"{order}. {input?.Name ?? layer.InputId.ToString()}";
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
        for (var i = _scene.Layers.Count - 1; i >= 0; i--)
        {
            var layer = _scene.Layers[i];
            var color = layer.Hidden ? Color.FromRgb(0x55, 0x55, 0x55) : hues[i % hues.Length];
            var rect = new Rectangle
            {
                Width = Math.Max(8, layer.Width * WireCanvas.Width),
                Height = Math.Max(8, layer.Height * WireCanvas.Height),
                Stroke = new SolidColorBrush(color),
                StrokeThickness = ReferenceEquals(layer, _selected) ? 4 : 2,
                StrokeDashArray = layer.Hidden ? new DoubleCollection { 4, 3 } : null,
                Fill = new SolidColorBrush(Color.FromArgb(layer.Hidden ? (byte)20 : (byte)40, color.R, color.G, color.B)),
                Tag = layer
            };
            Canvas.SetLeft(rect, layer.X * WireCanvas.Width);
            Canvas.SetTop(rect, layer.Y * WireCanvas.Height);
            WireCanvas.Children.Add(rect);
            var cropW = Math.Clamp(layer.CropWidth, 0.01f, 1f);
            var cropH = Math.Clamp(layer.CropHeight, 0.01f, 1f);
            var cropX = Math.Clamp(layer.CropX, 0f, 1f - cropW);
            var cropY = Math.Clamp(layer.CropY, 0f, 1f - cropH);
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
            if (ReferenceEquals(layer, _selected) && !layer.Locked)
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

    private double CropXMaxPx() => _width;
    private double CropYMaxPx() => _height;
    private double CropWMaxPx() => _width;
    private double CropHMaxPx() => _height;

    private void WriteCropBoxes()
    {
        if (_selected is null)
            return;
        WriteInsetBoxes(_selected);
    }

    private void WriteInsetBoxes(SceneLayer slot)
    {
        CropInsets.FromRect(slot.CropX, slot.CropY, slot.CropWidth, slot.CropHeight, out var left, out var up, out var right, out var down);
        CropXBox.Text = (left * _width).ToString("0.#");
        CropYBox.Text = (up * _height).ToString("0.#");
        CropWBox.Text = (right * _width).ToString("0.#");
        CropHBox.Text = (down * _height).ToString("0.#");
    }

    private void RefreshCropMeterRange()
    {
        if (_meterBox is null || MeterHost.Visibility != Visibility.Visible)
            return;
        double min;
        double max;
        if (ReferenceEquals(_meterBox, CropXBox))
        {
            min = 0;
            max = CropXMaxPx();
        }
        else if (ReferenceEquals(_meterBox, CropYBox))
        {
            min = 0;
            max = CropYMaxPx();
        }
        else if (ReferenceEquals(_meterBox, CropWBox))
        {
            min = 0;
            max = CropWMaxPx();
        }
        else if (ReferenceEquals(_meterBox, CropHBox))
        {
            min = 0;
            max = CropHMaxPx();
        }
        else
            return;
        _suppress = true;
        MeterSlider.Minimum = min;
        MeterSlider.Maximum = Math.Max(max, min);
        if (float.TryParse(_meterBox.Text, out var meter))
            MeterSlider.Value = Math.Clamp(meter, MeterSlider.Minimum, MeterSlider.Maximum);
        _suppress = false;
    }

    private void FillNumeric()
    {
        var layer = _selected;
        XBox.Text = ((layer?.X ?? 0) * _width).ToString("0.#");
        YBox.Text = ((layer?.Y ?? 0) * _height).ToString("0.#");
        WBox.Text = ((layer?.Width ?? 1) * _width).ToString("0.#");
        HBox.Text = ((layer?.Height ?? 1) * _height).ToString("0.#");
        if (layer is null)
        {
            CropXBox.Text = "0";
            CropYBox.Text = "0";
            CropWBox.Text = "0";
            CropHBox.Text = "0";
        }
        else
            WriteInsetBoxes(layer);
        OpBox.Text = (layer?.Opacity ?? 1).ToString("0.###");
        LinkBox.IsChecked = layer?.SizeLinked ?? true;
        _suppress = true;
        LayerInputBox.SelectedItem = layer is null
            ? null
            : _session.Inputs.FirstOrDefault(item => item.Id == layer.InputId);
        _suppress = false;
        var edit = layer is { Locked: false };
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
        LayerInputBox.IsEnabled = edit;
        if (_meterBox is not null && float.TryParse(_meterBox.Text, out var meter))
        {
            _suppress = true;
            MeterSlider.Value = Math.Clamp(meter, MeterSlider.Minimum, MeterSlider.Maximum);
            _suppress = false;
        }
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

    private void ZUp_Click(object sender, RoutedEventArgs e) => ShiftDisplay(-1);

    private void ZDown_Click(object sender, RoutedEventArgs e) => ShiftDisplay(1);

    private void ShiftDisplay(int delta)
    {
        if (_selected is null)
            return;
        var index = _scene.Layers.IndexOf(_selected);
        var target = index + delta;
        if (target < 0 || target >= _scene.Layers.Count)
            return;
        (_scene.Layers[index], _scene.Layers[target]) = (_scene.Layers[target], _scene.Layers[index]);
        NormalizeOrder();
        RefreshLayers();
        PushGpu();
    }

    private void LayerList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (LayerList.SelectedItem is FrameworkElement { Tag: SceneLayer layer })
            _selected = layer;
        else if (LayerList.SelectedIndex >= 0 && LayerList.SelectedIndex < _scene.Layers.Count)
            _selected = _scene.Layers[LayerList.SelectedIndex];
        else
            return;
        DrawWireframe();
        FillNumeric();
    }

    private void Numeric_LostFocus(object sender, RoutedEventArgs e) => ApplyNumeric(true, sender);

    private void ApplyNumeric(bool push, object? sender)
    {
        if (_selected is null || _selected.Locked)
            return;
        if (float.TryParse(XBox.Text, out var x)) _selected.X = x / _width;
        if (float.TryParse(YBox.Text, out var y)) _selected.Y = y / _height;
        var fromW = ReferenceEquals(sender, WBox);
        var fromH = ReferenceEquals(sender, HBox);
        if (fromW && float.TryParse(WBox.Text, out var w))
        {
            var width = Math.Max(1f, w) / _width;
            if (_selected.SizeLinked && _selected.Width > 0)
                _selected.Height = width * (_selected.Height / _selected.Width);
            _selected.Width = width;
        }
        else if (fromH && float.TryParse(HBox.Text, out var h))
        {
            var height = Math.Max(1f, h) / _height;
            if (_selected.SizeLinked && _selected.Height > 0)
                _selected.Width = height * (_selected.Width / _selected.Height);
            _selected.Height = height;
        }
        else
        {
            if (float.TryParse(WBox.Text, out var bothW))
                _selected.Width = Math.Max(1f, bothW) / _width;
            if (float.TryParse(HBox.Text, out var bothH))
                _selected.Height = Math.Max(1f, bothH) / _height;
        }
        if (ReferenceEquals(sender, CropXBox) && float.TryParse(CropXBox.Text, out var cx))
            _selected.SetCropInset(CropEdit.Left, cx / _width);
        else if (ReferenceEquals(sender, CropYBox) && float.TryParse(CropYBox.Text, out var cy))
            _selected.SetCropInset(CropEdit.Up, cy / _height);
        else if (ReferenceEquals(sender, CropWBox) && float.TryParse(CropWBox.Text, out var cw))
            _selected.SetCropInset(CropEdit.Right, cw / _width);
        else if (ReferenceEquals(sender, CropHBox) && float.TryParse(CropHBox.Text, out var ch))
            _selected.SetCropInset(CropEdit.Down, ch / _height);
        else
        {
            CropInsets.FromRect(_selected.CropX, _selected.CropY, _selected.CropWidth, _selected.CropHeight, out var left, out var up, out var right, out var down);
            if (float.TryParse(CropXBox.Text, out var allCx)) left = allCx / _width;
            if (float.TryParse(CropYBox.Text, out var allCy)) up = allCy / _height;
            if (float.TryParse(CropWBox.Text, out var allCw)) right = allCw / _width;
            if (float.TryParse(CropHBox.Text, out var allCh)) down = allCh / _height;
            _selected.SetCropInset(CropEdit.Left, left);
            _selected.SetCropInset(CropEdit.Up, up);
            _selected.SetCropInset(CropEdit.Right, right);
            _selected.SetCropInset(CropEdit.Down, down);
        }
        if (float.TryParse(OpBox.Text, out var op)) _selected.Opacity = Math.Clamp(op, 0, 1);
        DrawWireframe();
        WriteCropBoxes();
        RefreshCropMeterRange();
        if (push)
        {
            FillNumeric();
            PushGpu();
        }
        else
        {
            if (fromW)
                HBox.Text = (_selected.Height * _height).ToString("0.#");
            else if (fromH)
                WBox.Text = (_selected.Width * _width).ToString("0.#");
            if (DateTime.UtcNow - _lastGpuPush >= TimeSpan.FromMilliseconds(50))
            {
                _lastGpuPush = DateTime.UtcNow;
                PushGpu();
            }
        }
    }

    private void LayerAudioFollow_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: SceneLayer layer })
            return;
        layer.AudioFollow = !layer.AudioFollow;
        _selected = layer;
        RefreshLayers();
        PushGpu();
        e.Handled = true;
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
        var hit = HitLayer(pos);
        _selected = hit;
        _dragging = hit is { Locked: false };
        _resizing = false;
        _last = pos;
        if (_dragging)
            WireCanvas.CaptureMouse();
        RefreshLayers();
    }

    private SceneLayer? HitLayer(Point pos)
    {
        var hits = _scene.Layers.Where(layer =>
            pos.X >= layer.X * WireCanvas.Width
            && pos.X <= (layer.X + layer.Width) * WireCanvas.Width
            && pos.Y >= layer.Y * WireCanvas.Height
            && pos.Y <= (layer.Y + layer.Height) * WireCanvas.Height).ToList();
        if (hits.Count == 0)
            return null;
        if (_selected is not null && hits.Contains(_selected))
            return _selected;
        return hits.OrderByDescending(item => item.Z).First();
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
        if (DateTime.UtcNow - _lastGpuPush >= TimeSpan.FromMilliseconds(50))
        {
            _lastGpuPush = DateTime.UtcNow;
            PushGpu();
        }
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

    private void LayerLock_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: SceneLayer layer })
            return;
        layer.Locked = !layer.Locked;
        _selected = layer;
        RefreshLayers();
        e.Handled = true;
    }

    private void LayerHide_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: SceneLayer layer })
            return;
        layer.Hidden = !layer.Hidden;
        _selected = layer;
        RefreshLayers();
        PushGpu();
        e.Handled = true;
    }

    private void LayerInput_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppress || _selected is null || _selected.Locked || LayerInputBox.SelectedItem is not InputEntry input)
            return;
        _selected.InputId = input.Id;
        RefreshLayers();
        PushGpu();
    }

    private void ResetLayer_Click(object sender, RoutedEventArgs e)
    {
        if (_selected is null || _selected.Locked)
            return;
        _selected.ResetLayout();
        RefreshLayers();
        PushGpu();
    }

    private void FillPresets()
    {
        PresetCards.Children.Clear();
        foreach (var name in SceneLayoutPresets.BuiltIn)
            PresetCards.Children.Add(PresetCard(name, SceneLayoutPresets.Boxes(name)));
        foreach (var preset in _session.ScenePresets)
        {
            var boxes = preset.Layers
                .Select(layer => (layer.X, layer.Y, layer.Width, layer.Height))
                .ToArray();
            PresetCards.Children.Add(PresetCard(preset.Name, boxes, canDelete: true));
        }
        CopyFromBox.ItemsSource = _session.Scenes.Where(item => item.Id != _scene.Id).Select(item => item.Name).ToArray();
    }

    private UIElement PresetCard(string name, IReadOnlyList<(float X, float Y, float W, float H)> boxes, bool canDelete = false)
    {
        const double width = 112;
        const double height = 63;
        var card = new StackPanel { Width = width, Margin = new Thickness(0, 0, 8, 8), Cursor = Cursors.Hand };
        card.Children.Add(new Border
        {
            BorderBrush = new SolidColorBrush(Color.FromRgb(0x44, 0x44, 0x44)),
            BorderThickness = new Thickness(1),
            Child = SceneLayoutPresets.Mosaic(boxes, width, height)
        });
        var caption = new DockPanel { Margin = new Thickness(0, 2, 0, 0) };
        if (canDelete)
        {
            var del = new Button
            {
                Content = "×",
                Width = 18,
                Height = 18,
                Padding = new Thickness(0),
                FontSize = 11,
                ToolTip = "Delete saved preset"
            };
            del.Click += (_, e) =>
            {
                e.Handled = true;
                DeletePreset(name);
            };
            DockPanel.SetDock(del, Dock.Right);
            caption.Children.Add(del);
        }
        caption.Children.Add(new TextBlock
        {
            Text = name,
            FontSize = 10,
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis
        });
        card.Children.Add(caption);
        card.MouseLeftButtonUp += (_, _) => ApplyPreset(name);
        return card;
    }

    private void DeletePreset(string name)
    {
        _session.ScenePresets.RemoveAll(item => item.Name == name);
        FillPresets();
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
                _scene.Layers[i].CropX = src.CropX;
                _scene.Layers[i].CropY = src.CropY;
                _scene.Layers[i].CropWidth = src.CropWidth;
                _scene.Layers[i].CropHeight = src.CropHeight;
                _scene.Layers[i].SizeLinked = src.SizeLinked;
                _scene.Layers[i].AudioFollow = src.AudioFollow;
                _scene.Layers[i].ClampCrop();
            }
            _scene.Layers.Sort((a, b) => b.Z.CompareTo(a.Z));
            NormalizeOrder();
            RefreshLayers();
            PushGpu();
            return;
        }
        if (name == "Full")
        {
            foreach (var layer in _scene.Layers.Where(item => !item.Locked))
            {
                layer.X = 0;
                layer.Y = 0;
                layer.Width = 1;
                layer.Height = 1;
                layer.ResetLayoutExtras();
            }
            RefreshLayers();
            PushGpu();
            return;
        }
        var boxes = SceneLayoutPresets.Boxes(name);
        var unlocked = _scene.Layers.Where(item => !item.Locked).ToList();
        for (var i = 0; i < unlocked.Count && i < boxes.Count; i++)
        {
            var (x, y, w, h) = boxes[i];
            unlocked[i].X = x;
            unlocked[i].Y = y;
            unlocked[i].Width = w;
            unlocked[i].Height = h;
            unlocked[i].ResetLayoutExtras();
        }
        RefreshLayers();
        PushGpu();
    }

    private void Ok_Click(object sender, RoutedEventArgs e)
    {
        _scene.Name = string.IsNullOrWhiteSpace(NameBox.Text) ? _scene.Name : NameBox.Text.Trim();
        if (_tags is { } tags)
            TagCatalog.Replace(_scene.Tags, tags.Selected);
        PushGpu();
        DialogResult = true;
    }

    private void AddTag_Click(object sender, RoutedEventArgs e) => _tags?.PromptAdd();

    protected override void OnClosed(EventArgs e)
    {
        PreviewHost.ReleaseNative();
        if (DialogResult != true)
        {
            _scene.Layers.Clear();
            _scene.Layers.AddRange(_original);
            PushGpu();
        }
        base.OnClosed(e);
    }
}
