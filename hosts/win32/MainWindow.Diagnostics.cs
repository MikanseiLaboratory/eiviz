using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Data;
using System.Windows.Input;
using System.Windows.Threading;
using Eiviz.Host.Dialogs;
using Eiviz.Host.I18n;
using Eiviz.Host.Interop;
using Eiviz.Host.Media;
using Eiviz.Host.Preview;

namespace Eiviz.Host;

public partial class MainWindow
{
    private void RebuildMeters()
    {
        MeterPanel.Children.Clear();
        _meters.Clear();
        foreach (var unit in _session.Units)
            AddUnitMeter(unit);
        foreach (var input in _session.Inputs)
            AddInputMeter(input);
        if (_session.HeadphoneListenKind == MixerNative.ListenUnit
            && _session.Units.All(unit => unit.Id != _session.HeadphoneListenId)
            || _session.HeadphoneListenKind == MixerNative.ListenInput
            && _session.Inputs.All(input => input.Id != _session.HeadphoneListenId))
        {
            _session.HeadphoneListenKind = MixerNative.ListenOff;
            _session.HeadphoneListenId = 0;
            if (!HostRole.IsRemote)
                MixerNative.AudioSetHeadphoneListen(MixerNative.ListenOff, 0);
            foreach (var meter in _meters.Values)
                meter.SetListening(false);
        }
    }

    private void AddUnitMeter(MixingUnitEntry unit)
    {
        var strip = new MeterStrip(MeterKind.Bus, unit.Id, unit.Name, unit.Audio.Gain, unit.Audio.Mute, showRoutes: false);
        strip.ListenRequested += ToggleListen;
        strip.SetListening(IsListening(strip));
        strip.FaderChanged += (_, gain, mute) =>
        {
            unit.Audio.Gain = gain;
            unit.Audio.Mute = mute;
            ((App)Application.Current).Backend.SetBusGain(unit.Id, gain, mute);
        };
        _meters[MixerNative.AudioBusPeakBase | unit.Id] = strip;
        MeterPanel.Children.Add(strip);
    }

    private void AddInputMeter(InputEntry input)
    {
        var strip = new MeterStrip(
            MeterKind.Input, input.Id, input.ListLabel, input.Gain, input.Mute,
            showFader: false, showOpen: true, showRoutes: input.Kind != InputKind.Mix);
        if (input.Kind != InputKind.Mix)
        {
            strip.SetRoutes(_session.Units, input.AudioUnits, FollowUnits(input));
            strip.FollowCleared += ClearInputFollow;
        }
        strip.ListenRequested += ToggleListen;
        strip.SetListening(IsListening(strip));
        strip.RoutesChanged += (_, routes) => ApplyInputAudio(input, routes, input.Gain, input.Mute);
        strip.FaderChanged += (_, gain, mute) =>
            ApplyInputAudio(input, input.AudioUnits, gain, mute);
        strip.OpenRequested += _ => OpenAudioInput(input);
        _meters[input.Id] = strip;
        MeterPanel.Children.Add(strip);
        if (_audioInputs.TryGetValue(input.Id, out var window))
        {
            window.SetRoutes(input.AudioUnits, FollowUnits(input));
            window.Sync(input.Gain, input.Mute, input.AudioUnits, FollowUnits(input));
        }
    }

    private void OpenAudioInput(InputEntry input)
    {
        if (_audioInputs.TryGetValue(input.Id, out var existing))
        {
            existing.Activate();
            return;
        }
        var window = new AudioInputWindow(input, _session.Units) { Owner = this };
        window.Changed += ApplyInputAudio;
        window.FollowCleared += ClearInputFollow;
        window.ListenRequested += ToggleListen;
        if (input.Kind != InputKind.Mix)
            window.SetRoutes(input.AudioUnits, FollowUnits(input));
        window.SetListening(IsListeningInput(input.Id));
        window.Closed += (_, _) => _audioInputs.Remove(input.Id);
        _audioInputs[input.Id] = window;
        window.Show();
    }

    private void CloseAudioInput(ulong inputId)
    {
        if (!_audioInputs.TryGetValue(inputId, out var window))
            return;
        _audioInputs.Remove(inputId);
        window.Close();
    }

    private void ApplyInputAudio(InputEntry input, IReadOnlyList<ulong> routes, float gain, bool mute)
    {
        input.AudioUnits = input.Kind == InputKind.Mix ? [] : routes.Distinct().Order().ToList();
        input.Gain = gain;
        input.Mute = mute;
        ((App)Application.Current).Backend.SetInputGain(
            input.Id, input.AudioUnits, MixerNative.MixerGain(input.Gain), input.Mute);
        if (_meters.TryGetValue(input.Id, out var strip))
        {
            strip.SyncFrom(input.Gain, input.Mute);
            if (input.Kind != InputKind.Mix)
                strip.SetRoutes(_session.Units, input.AudioUnits, FollowUnits(input));
        }
        if (_audioInputs.TryGetValue(input.Id, out var window))
        {
            window.Sync(input.Gain, input.Mute, input.AudioUnits, FollowUnits(input));
            window.SetListening(IsListeningInput(input.Id));
        }
    }

    private IReadOnlyList<ulong> FollowUnits(InputEntry input)
    {
        if (input.Kind == InputKind.Mix || Application.Current is not App app)
            return [];
        var units = new List<ulong>();
        foreach (var unit in _session.Units)
        {
            app.Backend.BusSources(unit.Id, out _, out var program);
            if (SceneCarries(program, input.Id))
            {
                units.Add(unit.Id);
                continue;
            }
            foreach (var overlayId in unit.OverlaysOnAir)
            {
                var slot = _session.Overlays.FirstOrDefault(item => item.Id == overlayId);
                if (slot is null || !slot.AudioFollow)
                    continue;
                if (slot.SourceKind == OverlaySourceKind.Input && slot.SceneGpuId == input.Id
                    || slot.SourceKind == OverlaySourceKind.Scene && SceneCarries(slot.SceneGpuId, input.Id))
                {
                    units.Add(unit.Id);
                    break;
                }
            }
        }
        return units;
    }

    private bool SceneCarries(ulong sceneGpuId, ulong inputId)
    {
        var scene = _session.Scenes.FirstOrDefault(item => item.GpuId == sceneGpuId);
        return scene is not null && scene.Layers.Any(layer => layer.AudioFollow && LayerCarries(layer.InputId, inputId, 0));
    }

    private bool LayerCarries(ulong sourceId, ulong inputId, int depth)
    {
        if (sourceId == inputId)
            return true;
        if (depth >= 4)
            return false;
        var scene = _session.Scenes.FirstOrDefault(item => item.GpuId == sourceId);
        return scene is not null && scene.Layers.Any(layer => layer.AudioFollow && LayerCarries(layer.InputId, inputId, depth + 1));
    }

    private void ClearInputFollow(ulong inputId, ulong unitId)
    {
        var input = _session.Inputs.FirstOrDefault(item => item.Id == inputId);
        var unit = _session.Units.FirstOrDefault(item => item.Id == unitId);
        if (input is null || unit is null || Application.Current is not App app)
            return;
        app.Backend.BusSources(unit.Id, out _, out var program);
        ClearSceneFollow(program, input.Id);
        var overlays = false;
        foreach (var overlayId in unit.OverlaysOnAir.ToArray())
        {
            var slot = _session.Overlays.FirstOrDefault(item => item.Id == overlayId);
            if (slot is null || !slot.AudioFollow)
                continue;
            if (slot.SourceKind == OverlaySourceKind.Input && slot.SceneGpuId == input.Id)
            {
                slot.AudioFollow = false;
                overlays = true;
            }
            else if (slot.SourceKind == OverlaySourceKind.Scene)
            {
                ClearSceneFollow(slot.SceneGpuId, input.Id);
            }
        }
        if (overlays)
            MixerApply.PushOverlays(_session, unit);
        _overlay?.Reload(unit);
    }

    private void ClearSceneFollow(ulong sceneGpuId, ulong inputId)
    {
        var scene = _session.Scenes.FirstOrDefault(item => item.GpuId == sceneGpuId);
        if (scene is null)
            return;
        var changed = false;
        foreach (var layer in scene.Layers)
        {
            if (!layer.AudioFollow)
                continue;
            if (layer.InputId == inputId)
            {
                layer.AudioFollow = false;
                changed = true;
                continue;
            }
            if (_session.Scenes.Any(item => item.GpuId == layer.InputId) && LayerCarries(layer.InputId, inputId, 0))
            {
                ClearSceneFollow(layer.InputId, inputId);
                changed = true;
            }
        }
        if (changed)
            MixerApply.DefineScene(scene, SelectedUnit.Width, SelectedUnit.Height);
    }

    private bool IsListening(MeterStrip strip)
    {
        var kind = strip.Kind == MeterKind.Bus ? MixerNative.ListenUnit : MixerNative.ListenInput;
        return _session.HeadphoneListenKind == kind && _session.HeadphoneListenId == strip.TargetId;
    }

    private bool IsListeningInput(ulong inputId) =>
        _session.HeadphoneListenKind == MixerNative.ListenInput && _session.HeadphoneListenId == inputId;

    private void ToggleListen(MeterStrip strip)
    {
        if (IsListening(strip))
        {
            _session.HeadphoneListenKind = MixerNative.ListenOff;
            _session.HeadphoneListenId = 0;
        }
        else
        {
            _session.HeadphoneListenKind = strip.Kind == MeterKind.Bus
                ? MixerNative.ListenUnit
                : MixerNative.ListenInput;
            _session.HeadphoneListenId = strip.TargetId;
        }
        if (!HostRole.IsRemote)
            MixerNative.AudioSetHeadphoneListen(_session.HeadphoneListenKind, _session.HeadphoneListenId);
        foreach (var meter in _meters.Values)
            meter.SetListening(IsListening(meter));
        foreach (var window in _audioInputs.Values)
            window.SetListening(IsListeningInput(window.InputId));
    }

    private void TickMeters()
    {
        if (HandleMixerFatal())
            return;
        var peaks = new Dictionary<ulong, (float L, float R)>();
        if (Application.Current is App { Backend.IsRemote: true } meterApp)
        {
            foreach (var pair in meterApp.Backend.Peaks)
                peaks[pair.Key] = pair.Value;
        }
        else
        {
            var buffer = new AudioPeak[64];
            unsafe
            {
                fixed (AudioPeak* ptr = buffer)
                {
                    var n = MixerNative.CopyAudioPeaks(ptr, (uint)buffer.Length);
                    for (var i = 0; i < n && i < buffer.Length; i++)
                        peaks[buffer[i].SourceId] = (buffer[i].Left, buffer[i].Right);
                }
            }
        }
        foreach (var (_, strip) in _meters)
        {
            var key = strip.Kind == MeterKind.Bus ? MixerNative.AudioBusPeakBase | strip.TargetId : strip.TargetId;
            if (strip.Kind == MeterKind.Bus && strip.TargetId == 1 && peaks.TryGetValue(0, out var master))
            {
                strip.SetLevels(master.L, master.R);
                continue;
            }
            if (peaks.TryGetValue(key, out var pair))
            {
                if (strip.Kind == MeterKind.Input)
                {
                    var post = MeterStrip.PostPeak(pair.L, pair.R, strip.Gain, strip.Mute);
                    strip.SetLevels(post.L, post.R);
                    if (_audioInputs.TryGetValue(strip.TargetId, out var window))
                        window.SetPeaks(pair.L, pair.R);
                }
                else
                {
                    strip.SetLevels(pair.L, pair.R);
                }
            }
            else
            {
                strip.Decay();
                if (strip.Kind == MeterKind.Input && _audioInputs.TryGetValue(strip.TargetId, out var window))
                    window.SetPeaks(0, 0);
            }
        }
        MixerStats stats = default;
        unsafe
        {
            if (MixerNative.CopyStats(&stats) == 0)
                FlipBudget.ObserveLost(stats.SurfaceLost);
        }
        ResourceText.Text = _resources.Line();
        RefreshStatusBar();
        TickVideo();
    }

    private bool HandleMixerFatal()
    {
        if (_fatalHandled)
            return true;
        var fatal = MixerNative.TakeFatalText();
        if (string.IsNullOrEmpty(fatal))
            return false;
        _fatalHandled = true;
        _meterTimer.Stop();
        try
        {
            var dir = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "eiviz");
            Directory.CreateDirectory(dir);
            SessionStore.Save(_session, Path.Combine(dir, "recovered-session.eivz"));
        }
        catch (Exception ex)
        {
            HostLog.WriteException(ex);
        }
        MessageBox.Show(this, Loc.T("error.mixerFatal"), Loc.T("app.title"));
        Application.Current.Shutdown();
        return true;
    }

    private void SyncTBarsFromMixer()
    {
        ApplyTBarFromMixer(SelectedUnit.Id, TBar, ref _tbarLatching, ref _tbarLocked);
        foreach (var window in _switchers.Values)
            window.ApplyMixerMix();
        var program = CurrentProgramSceneId();
        var previewGpu = CurrentPreviewSceneGpuId();
        MixerApply.CaptureSceneBuses(_session);
        var previewId = _session.Scenes.FirstOrDefault(item => item.GpuId == previewGpu)?.Id ?? 0;
        if (program != _shownProgramId || previewId != _shownPreviewId)
            RefreshSceneTiles();
    }

    internal static void ApplyTBarFromMixer(ulong unitId, Slider tbar, ref bool latching, ref bool locked)
    {
        if (tbar.IsMouseCaptureWithin || locked || latching)
            return;
        if (Application.Current is App app && app.Backend.TryGetMix(unitId, out var mix))
        {
            if (Math.Abs(tbar.Value - mix) < 0.002)
                return;
            latching = true;
            tbar.Value = mix;
            latching = false;
            return;
        }
        unsafe
        {
            UnitState state = default;
            if (MixerNative.GetUnitState(unitId, &state) != 0)
                return;
            if (Math.Abs(tbar.Value - state.Mix) < 0.002)
                return;
            latching = true;
            tbar.Value = state.Mix;
            latching = false;
        }
    }


    private void Logs_Click(object sender, RoutedEventArgs e) => OpenLogs();

    private void ResourceHud_MouseUp(object sender, MouseButtonEventArgs e) => OpenResources();

    private void OpenResources()
    {
        if (_resourcesWindow is not null)
        {
            _resourcesWindow.Activate();
            return;
        }
        _resourcesWindow = new ResourceMonitorWindow { Owner = this };
        _resourcesWindow.Closed += (_, _) => _resourcesWindow = null;
        _resourcesWindow.Show();
    }

    private void OpenLogs()
    {
        if (_logWindow is not null)
        {
            _logWindow.Activate();
            return;
        }
        _logWindow = new LogWindow { Owner = this };
        _logWindow.Closed += (_, _) => _logWindow = null;
        _logWindow.Show();
    }
}
