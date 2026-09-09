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
        var strip = new MeterStrip(MeterKind.Input, input.Id, input.Name, input.Gain, input.Mute);
        strip.SetBuses(_session.Buses, input.BusMask == 0 ? 1u : input.BusMask);
        strip.BusMaskChanged += (_, mask) =>
        {
            input.BusMask = mask;
            ((App)Application.Current).Backend.SetInputGain(
                input.Id, mask, MixerNative.MixerGain(input.Gain), input.Mute);
        };
        strip.FaderChanged += (_, gain, mute) =>
        {
            input.Gain = gain;
            input.Mute = mute;
            ((App)Application.Current).Backend.SetInputGain(
                input.Id, input.BusMask == 0 ? 1u : input.BusMask, gain, mute);
        };
        _meters[input.Id] = strip;
        MeterPanel.Children.Add(strip);
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
                strip.SetLevels(pair.L, pair.R);
            else
                strip.Decay();
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
