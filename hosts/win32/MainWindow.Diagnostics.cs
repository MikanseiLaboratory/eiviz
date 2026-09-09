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
        foreach (var bus in _session.Buses)
            AddBusMeter(bus);
        foreach (var input in _session.Inputs)
            AddInputMeter(input);
    }

    private void AddBusMeter(AudioBusEntry bus)
    {
        var strip = new MeterStrip(MeterKind.Bus, bus.Id, bus.Name, bus.Gain, bus.Mute);
        strip.FaderChanged += (_, gain, mute) =>
        {
            bus.Gain = gain;
            bus.Mute = mute;
            ((App)Application.Current).Backend.SetBusGain(bus.Id, gain, mute);
        };
        _meters[MixerNative.AudioBusPeakBase | bus.Id] = strip;
        MeterPanel.Children.Add(strip);
    }

    private void AddInputMeter(InputEntry input)
    {
        var strip = new MeterStrip(
            MeterKind.Input, input.Id, input.ListLabel, input.Gain, input.Mute,
            showFader: false, showOpen: true);
        strip.SetBuses(_session.Buses, input.BusMask == 0 ? 1u : input.BusMask);
        strip.BusMaskChanged += (_, mask) => ApplyInputAudio(input, mask, input.Gain, input.Mute);
        strip.FaderChanged += (_, gain, mute) =>
            ApplyInputAudio(input, input.BusMask == 0 ? 1u : input.BusMask, gain, mute);
        strip.OpenRequested += _ => OpenAudioInput(input);
        _meters[input.Id] = strip;
        MeterPanel.Children.Add(strip);
        if (_audioInputs.TryGetValue(input.Id, out var window))
        {
            window.SetBuses(_session.Buses, input.BusMask == 0 ? 1u : input.BusMask);
            window.Sync(input.Gain, input.Mute, input.BusMask == 0 ? 1u : input.BusMask);
        }
    }

    private void OpenAudioInput(InputEntry input)
    {
        if (_audioInputs.TryGetValue(input.Id, out var existing))
        {
            existing.Activate();
            return;
        }
        var window = new AudioInputWindow(input, _session.Buses) { Owner = this };
        window.Changed += ApplyInputAudio;
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

    private void ApplyInputAudio(InputEntry input, uint mask, float gain, bool mute)
    {
        input.BusMask = input.Kind == InputKind.Mix ? 0u : (mask == 0 ? 1u : mask);
        input.Gain = gain;
        input.Mute = mute;
        ((App)Application.Current).Backend.SetInputGain(
            input.Id, input.BusMask, MixerNative.MixerGain(input.Gain), input.Mute);
        if (_meters.TryGetValue(input.Id, out var strip))
        {
            strip.SyncFrom(input.Gain, input.Mute);
            if (strip.BusMask != input.BusMask)
                strip.SetBuses(_session.Buses, input.BusMask);
        }
        if (_audioInputs.TryGetValue(input.Id, out var window))
            window.Sync(input.Gain, input.Mute, input.BusMask);
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
