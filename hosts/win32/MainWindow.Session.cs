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
    private void AddInput_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new AddInputWindow { Owner = this };
        dialog.BindTags(_session);
        if (dialog.ShowDialog() != true)
            return;
        if (dialog.Kind is not (InputKind.Color or InputKind.Bars or InputKind.Mix or InputKind.Audio) && dialog.ResultPath is null)
            return;
        var id = _session.NextInputId++;
        var input = new InputEntry
        {
            Id = id,
            Name = dialog.ResultName ?? $"Input {id}",
            Kind = dialog.Kind,
            BusMask = dialog.Kind == InputKind.Mix ? 0u : 1u
        };
        try
        {
            if (App.IsRemote && dialog.Kind is InputKind.Still or InputKind.Video)
            {
                if (!((App)Application.Current).Backend.UploadMedia(
                    dialog.ResultPath!,
                    dialog.Kind == InputKind.Still ? "still" : "video",
                    dialog.ResultName ?? "",
                    dialog.ResultVideoLoop,
                    ((App)Application.Current).Backend.Revision,
                    out var error))
                {
                    MessageBox.Show(this, error, Loc.T("msg.addInput"));
                    return;
                }
                RefreshInputList();
                RebuildMeters();
                return;
            }
            if (App.IsRemote)
            {
                ApplyInputSource(input, dialog, replacing: false);
                RefreshInputList();
                RebuildMeters();
                return;
            }
            _session.Inputs.Add(input);
            ApplyInputSource(input, dialog, replacing: false);
        }
        catch (Exception ex)
        {
            _session.Inputs.Remove(input);
            if (!App.IsRemote)
            {
                MixerApply.DropSource(input.Id);
                MixerNative.FlushAudio(input.Id);
            }
            RefreshInputList();
            MessageBox.Show(this, ex.Message, Loc.T("msg.addInput"));
            return;
        }
        MixerNative.AudioSetInput(input.Id, input.BusMask, 1, 0);
        RefreshInputList();
        RebuildMeters();
    }

    private void InputList_DoubleClick(object sender, MouseButtonEventArgs e) => EditInput_Click(sender, e);

    private void EditInput_Click(object sender, RoutedEventArgs e)
    {
        if (InputList.SelectedItem is not InputEntry input)
        {
            MessageBox.Show(this, Loc.T("msg.selectInputEdit"));
            return;
        }
        var dialog = new AddInputWindow { Owner = this };
        dialog.BindTags(_session, input.Tags);
        dialog.Load(input);
        if (dialog.ShowDialog() != true)
            return;
        if (dialog.Kind is not (InputKind.Color or InputKind.Bars or InputKind.Mix or InputKind.Audio) && dialog.ResultPath is null)
            return;
        try
        {
            ApplyInputSource(input, dialog, replacing: true);
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, Loc.T("msg.editInput"));
            return;
        }
        RefreshInputList();
        RebuildMeters();
        TickVideo();
        RefreshMultiviewLabels();
        if (_inputPreviews.TryGetValue(input.Id, out var preview))
            preview.SetTitle(input.ListLabel);
    }

    private void PreviewInput_Click(object sender, RoutedEventArgs e)
    {
        if (App.IsRemote)
        {
            MessageBox.Show(this, Loc.T("msg.remoteNoInputPreview"), Loc.T("chrome.previewInput"));
            return;
        }
        if (InputList.SelectedItem is not InputEntry input)
        {
            MessageBox.Show(this, Loc.T("msg.selectInputPreview"));
            return;
        }
        OpenInputPreview(input);
    }

    private void SnapshotInput_Click(object sender, RoutedEventArgs e)
    {
        if (InputList.SelectedItem is not InputEntry input)
        {
            MessageBox.Show(this, Loc.T("msg.selectInputScreenshot"));
            return;
        }
        SnapshotInput(input);
    }

    private void OpenInputPreview(InputEntry input) => OpenSourcePreview(input.Id, input.ListLabel);

    private void OpenSourcePreview(ulong sourceId, string name)
    {
        if (_inputPreviews.TryGetValue(sourceId, out var existing))
        {
            existing.Activate();
            return;
        }
        var window = new InputPreviewWindow(name, sourceId, SelectedUnit.Width, SelectedUnit.Height)
        {
            Owner = this
        };
        window.Closed += (_, _) => _inputPreviews.Remove(sourceId);
        _inputPreviews[sourceId] = window;
        window.Show();
    }

    private void CloseInputPreview(ulong inputId)
    {
        if (!_inputPreviews.TryGetValue(inputId, out var window))
            return;
        _inputPreviews.Remove(inputId);
        window.Close();
    }

    private void ApplyInputSource(InputEntry input, AddInputWindow dialog, bool replacing)
    {
        var previousKind = input.Kind;
        var wasGenerator = previousKind is InputKind.Color or InputKind.Bars or InputKind.Black;
        var nowGenerator = dialog.Kind is InputKind.Color or InputKind.Bars;
        var keepLive = replacing
            && input.Kind == dialog.Kind
            && (
                (dialog.Kind == InputKind.Video
                    && input.PathOrAddress == dialog.ResultPath
                    && input.FrameBufferFrames == dialog.ResultFrameBufferFrames)
                || (dialog.Kind == InputKind.UVC
                    && input.PathOrAddress == dialog.ResultPath
                    && input.CaptureWidth == dialog.ResultCaptureWidth
                    && input.CaptureHeight == dialog.ResultCaptureHeight
                    && input.CaptureFpsNum == dialog.ResultCaptureFpsNum
                    && input.CaptureFpsDen == dialog.ResultCaptureFpsDen
                    && input.FrameBufferFrames == dialog.ResultFrameBufferFrames)
                || (dialog.Kind is InputKind.OMT or InputKind.NDI
                    && input.PathOrAddress == dialog.ResultPath
                    && input.UseGpu == (dialog.Kind == InputKind.OMT && dialog.ResultUseGpu)
                    && input.FrameBufferFrames == dialog.ResultFrameBufferFrames
                    && (dialog.Kind != InputKind.NDI || input.NdiBandwidth == dialog.ResultNdiBandwidth))
                || (dialog.Kind == InputKind.Mix
                    && input.MixSource == dialog.ResultMixSource
                    && input.MixTargetId == dialog.ResultMixTargetId
                    && input.MixAudioBusId == dialog.ResultMixAudioBusId
                    && input.FrameBufferFrames == dialog.ResultFrameBufferFrames));
        if (!App.IsRemote && replacing && !keepLive && !input.IsBuiltin && (!wasGenerator || !nowGenerator))
        {
            MixerApply.DropSource(input.Id);
            MixerNative.FlushAudio(input.Id);
        }
        input.Name = dialog.ResultName ?? input.Name;
        TagCatalog.Replace(input.Tags, dialog.ResultTags);
        TagCatalog.MergeInto(_session.InputTags, input.Tags);
        input.Kind = dialog.Kind;
        input.PathOrAddress = dialog.ResultPath;
        input.ColorR = dialog.ColorR;
        input.ColorG = dialog.ColorG;
        input.ColorB = dialog.ColorB;
        input.Scroll = dialog.Scroll;
        input.ToneHz = dialog.Kind is InputKind.Color or InputKind.Bars ? dialog.ResultToneHz : 0;
        input.ToneLevelDbfs = dialog.Kind is InputKind.Color or InputKind.Bars ? dialog.ResultToneLevelDbfs : -20;
        input.UseGpu = dialog.Kind == InputKind.OMT && dialog.ResultUseGpu;
        input.FrameBufferFrames = dialog.Kind is InputKind.OMT or InputKind.NDI or InputKind.Video or InputKind.UVC or InputKind.Mix
            ? dialog.ResultFrameBufferFrames
            : 1;
        input.MixSource = dialog.Kind == InputKind.Mix ? dialog.ResultMixSource : MixSource.MuProgram;
        input.MixTargetId = dialog.Kind == InputKind.Mix ? dialog.ResultMixTargetId : 0;
        input.MixAudioBusId = dialog.Kind == InputKind.Mix ? dialog.ResultMixAudioBusId : 0;
        if (dialog.Kind == InputKind.Audio)
        {
            input.AudioCaptureMode = dialog.ResultAudioCaptureMode;
            input.AudioDeviceKind = dialog.ResultAudioDeviceKind;
            input.AudioDeviceId = dialog.ResultAudioDeviceId ?? "";
            input.AudioMapLeft = dialog.ResultAudioMapLeft;
            input.AudioMapRight = dialog.ResultAudioMapRight;
            input.AudioProcessExe = dialog.ResultAudioProcessExe ?? "";
            input.AudioProcessAumid = dialog.ResultAudioProcessAumid ?? "";
        }
        if (dialog.Kind == InputKind.Mix)
            input.BusMask = 0;
        input.BandwidthSave = dialog.Kind == InputKind.OMT
            ? dialog.ResultSaveMode
            : BandwidthSave.NotOnPreviewOrProgram;
        input.KeepFullOnMultiview = dialog.Kind == InputKind.OMT
            && dialog.ResultKeepFullOnMultiview;
        input.OmtQuality = dialog.Kind == InputKind.OMT ? dialog.ResultOmtQuality : OmtQuality.Default;
        input.NdiBandwidth = dialog.Kind == InputKind.NDI ? dialog.ResultNdiBandwidth : NdiBandwidth.Highest;
        input.VideoLoop = dialog.Kind == InputKind.Video && dialog.ResultVideoLoop;
        input.VideoPlayWhen = dialog.Kind == InputKind.Video ? dialog.ResultVideoPlayWhen : VideoPlayWhen.Never;
        input.VideoRestartWhen = dialog.Kind == InputKind.Video ? dialog.ResultVideoRestartWhen : VideoTriggerWhen.Never;
        input.VideoPauseWhen = dialog.Kind == InputKind.Video ? dialog.ResultVideoPauseWhen : VideoTriggerWhen.Never;
        if (App.IsRemote)
        {
            RemoteMutate(MutationJson.UpsertInput(input), Loc.T("msg.addInput"));
            return;
        }
        if (keepLive)
        {
            if (dialog.Kind == InputKind.OMT)
            {
                MixerApply.LiveSave(
                    input.Id,
                    input.BandwidthSave,
                    input.KeepFullOnMultiview,
                    input.OmtQuality);
            }
            if (dialog.Kind == InputKind.Video)
                MixerNative.VideoSetLoop(input.Id, input.VideoLoop ? 1u : 0u);
            return;
        }
        switch (dialog.Kind)
        {
            case InputKind.Color:
            case InputKind.Bars:
                MixerApply.DefineGenerator(
                    input.Id,
                    dialog.Kind == InputKind.Bars ? MixerNative.GenBars : MixerNative.GenSolid,
                    dialog.ColorR,
                    dialog.ColorG,
                    dialog.ColorB,
                    dialog.Scroll,
                    input.ToneHz,
                    input.ToneLevelDbfs);
                break;
            case InputKind.Still:
                if (string.IsNullOrWhiteSpace(dialog.ResultPath) || !File.Exists(dialog.ResultPath))
                    throw new InvalidOperationException(Loc.MissingFile("Still load"));
                MixerApply.LoadStill(input.Id, dialog.ResultPath!);
                break;
            case InputKind.Video:
                if (string.IsNullOrWhiteSpace(dialog.ResultPath) || !File.Exists(dialog.ResultPath))
                    throw new InvalidOperationException(Loc.MissingFile("Video start"));
                MixerApply.StartVideo(
                    input.Id,
                    dialog.ResultPath!,
                    input.VideoLoop,
                    input.VideoStartsPlaying,
                    input.FrameBufferFrames);
                break;
            case InputKind.OMT:
                MixerApply.ConnectOmt(
                    input.Id,
                    dialog.ResultPath!,
                    dialog.ResultUseGpu,
                    dialog.ResultFrameBufferFrames,
                    input.BandwidthSave,
                    input.KeepFullOnMultiview,
                    input.OmtQuality);
                break;
            case InputKind.NDI:
                MixerApply.ConnectNdi(
                    input.Id,
                    dialog.ResultPath!,
                    dialog.ResultFrameBufferFrames,
                    input.NdiBandwidth);
                break;
            case InputKind.UVC:
                input.CaptureWidth = dialog.ResultCaptureWidth;
                input.CaptureHeight = dialog.ResultCaptureHeight;
                input.CaptureFpsNum = dialog.ResultCaptureFpsNum;
                input.CaptureFpsDen = dialog.ResultCaptureFpsDen;
                MixerApply.StartUvc(input.Id, dialog.ResultPath!, dialog.ResultCaptureWidth, dialog.ResultCaptureHeight, dialog.ResultCaptureFpsNum, dialog.ResultCaptureFpsDen, input.FrameBufferFrames);
                break;
            case InputKind.Mix:
                if (dialog.ResultMixSource != MixSource.SessionMultiview)
                {
                    var unit = _session.Units.FirstOrDefault(item => item.Id == dialog.ResultMixTargetId);
                    if (unit is not null && InputKindNames.UnitUsesSource(_session, unit, input.Id))
                        throw new InvalidOperationException(Loc.T("msg.mixCycle"));
                }
                MixerApply.DefineMixInput(
                    input.Id,
                    dialog.ResultMixTargetId,
                    InputKindNames.MixSourceKind(dialog.ResultMixSource),
                    dialog.ResultFrameBufferFrames,
                    dialog.ResultMixAudioBusId);
                break;
            case InputKind.Audio:
                input.AudioCaptureMode = dialog.ResultAudioCaptureMode;
                input.AudioDeviceKind = dialog.ResultAudioDeviceKind;
                input.AudioDeviceId = dialog.ResultAudioDeviceId ?? "";
                input.AudioMapLeft = dialog.ResultAudioMapLeft;
                input.AudioMapRight = dialog.ResultAudioMapRight;
                input.AudioProcessExe = dialog.ResultAudioProcessExe ?? "";
                input.AudioProcessAumid = dialog.ResultAudioProcessAumid ?? "";
                MixerApply.StartAudioCapture(input);
                break;
            default:
                throw new InvalidOperationException($"{dialog.Kind} is not available.");
        }
        SessionStore.Publish(_session);
    }

    private void RemoveInput_Click(object sender, RoutedEventArgs e)
    {
        if (InputList.SelectedItem is not InputEntry input)
        {
            MessageBox.Show(this, Loc.T("msg.selectInputDelete"));
            return;
        }
        if (input.IsBuiltin)
        {
            MessageBox.Show(this, Loc.T("msg.builtinDelete"));
            return;
        }
        if (TryRemoteMutate(MutationJson.DeleteInput(input.Id), Loc.T("msg.selectInputDelete")))
            return;
        CloseInputPreview(input.Id);
        CloseAudioInput(input.Id);
        MixerApply.DropSource(input.Id);
        MixerNative.FlushAudio(input.Id);
        foreach (var scene in _session.Scenes)
            scene.Layers.RemoveAll(layer => layer.InputId == input.Id);
        foreach (var layout in _session.Multiviews)
        {
            foreach (var tile in layout.Tiles)
            {
                if (tile.Kind == MvSlotKind.Input && tile.SourceId == input.Id)
                {
                    tile.Kind = MvSlotKind.None;
                    tile.SourceId = 0;
                }
            }
            MixerApply.PushMultiview(layout, SelectedUnit.Width, SelectedUnit.Height);
        }
        foreach (var unit in _session.Units)
        {
            unit.Overlays.RemoveAll(slot => slot.SourceKind == OverlaySourceKind.Input && slot.SceneGpuId == input.Id);
            MixerApply.PatchAux(unit.Id, unit);
        }
        foreach (var scene in _session.Scenes)
            MixerApply.TryDefineScene(scene, SceneWidth, SceneHeight);
        _session.Inputs.Remove(input);
        RefreshInputList();
        RebuildMeters();
        RebuildScenes();
        _overlay?.Reload(SelectedUnit);
        RebuildOverlayToggles();
        TickVideo();
        SessionStore.Publish(_session);
    }

    private void AddScene_Click(object sender, RoutedEventArgs e)
    {
        if (App.IsRemote)
        {
            var draft = new SceneEntry
            {
                Id = _session.NextSceneId,
                Name = $"Scene {_session.NextSceneId}"
            };
            TryRemoteMutate(MutationJson.UpsertScene(draft), Loc.T("chrome.scenes"));
            return;
        }
        var scene = _session.AddScene($"Scene {_session.NextSceneId}");
        MixerApply.TryDefineScene(scene, SceneWidth, SceneHeight);
        RebuildScenes();
        SelectScene(scene);
        OpenSceneEditor(scene);
        SessionStore.Publish(_session);
    }

    private void RemoveScene_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedScene is null)
        {
            MessageBox.Show(this, Loc.T("msg.selectSceneDelete"));
            return;
        }
        DeleteScene(_selectedScene);
    }

    private void DeleteScene(SceneEntry removed)
    {
        if (_session.Scenes.Count <= 1)
        {
            MessageBox.Show(this, Loc.T("msg.oneScene"));
            return;
        }
        if (TryRemoteMutate(MutationJson.DeleteScene(removed.Id), Loc.T("msg.selectSceneDelete")))
            return;
        CloseInputPreview(removed.GpuId);
        MixerApply.DestroyScene(removed.GpuId);
        _session.Scenes.Remove(removed);
        foreach (var layout in _session.Multiviews)
        {
            foreach (var tile in layout.Tiles)
            {
                if (tile.Kind == MvSlotKind.Scene && tile.SourceId == removed.GpuId)
                {
                    tile.Kind = MvSlotKind.None;
                    tile.SourceId = 0;
                }
            }
            MixerApply.PushMultiview(layout, SelectedUnit.Width, SelectedUnit.Height);
        }
        foreach (var unit in _session.Units)
        {
            unit.Overlays.RemoveAll(slot => slot.SourceKind == OverlaySourceKind.Scene && slot.SceneGpuId == removed.GpuId);
            MixerApply.PatchAux(unit.Id, unit);
        }
        var fallback = _session.Scenes[0];
        unsafe
        {
            UnitState state = default;
            if (MixerNative.GetUnitState(SelectedUnit.Id, &state) == 0)
            {
                if (state.ProgramSource == removed.GpuId)
                    state.ProgramSource = fallback.GpuId;
                if (state.PreviewSource == removed.GpuId)
                    state.PreviewSource = fallback.GpuId;
                MixerNative.SetUnitState(SelectedUnit.Id, &state);
            }
        }
        RebuildScenes();
        SelectScene(fallback);
        _overlay?.Reload(SelectedUnit);
        RebuildOverlayToggles();
        SessionStore.Publish(_session);
    }

    private void EditScene_Click(object sender, RoutedEventArgs e)
    {
        if (_selectedScene is not null)
            OpenSceneEditor(_selectedScene);
    }

    private void OpenSceneEditor(SceneEntry scene)
    {
        var now = Environment.TickCount64;
        if (_sceneEditorOpen || now - _sceneEditorGuard < 400)
            return;
        if (!HostRole.IsRemote && !FlipBudget.TryOpen(1, this))
            return;
        _sceneEditorOpen = true;
        try
        {
            var monitorId = _session.NextMonitorId++;
            var dialog = new SceneEditorWindow(scene, _session, SceneWidth, SceneHeight, monitorId) { Owner = this };
            dialog.ShowDialog();
        }
        finally
        {
            _sceneEditorOpen = false;
            _sceneEditorGuard = Environment.TickCount64;
        }
        if (Application.Current is not App appAfter || appAfter.Session is null || _fatalHandled)
            return;
        RefreshSceneList();
        SelectScene(scene);
        RefreshMultiviewLabels();
    }

    private void UnitBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressUnitChange || UnitBox.SelectedItem is not MixingUnitEntry unit)
            return;
        _session.SelectedUnitId = unit.Id;
        BindMainVideo();
        ApplyAspect();
        _overlay?.Reload(unit);
        _tbarPresetIndex = 0;
        RebuildTransitions();
        RebuildOverlayToggles();
        if (!HostRole.IsRemote)
            MixerNative.AudioSetHeadphoneCue(unit.Id);
        SyncSelectedSceneFromMixer();
        RefreshSceneTiles();
    }

    private void OpenSwitcher_Click(object sender, RoutedEventArgs e) =>
        OpenSwitcher(SelectedUnit);

    internal void OpenSwitcher(MixingUnitEntry unit)
    {
        if (_switchers.TryGetValue(unit.Id, out var existing))
        {
            existing.Activate();
            return;
        }
        if (!FlipBudget.TryOpen(2, this))
            return;
        var window = new SwitcherWindow(unit);
        if (unit.AlwaysOnTop)
            window.Owner = this;
        window.Topmost = unit.AlwaysOnTop;
        window.Closed += (_, _) => _switchers.Remove(unit.Id);
        _switchers[unit.Id] = window;
        window.Show();
    }

    private void CloseSwitcher(ulong unitId)
    {
        if (!_switchers.TryGetValue(unitId, out var window))
            return;
        _switchers.Remove(unitId);
        window.Close();
    }

    private void CloseAllSwitchers()
    {
        foreach (var window in _switchers.Values.ToArray())
            window.Close();
        _switchers.Clear();
    }

    private void AddUnit_Click(object sender, RoutedEventArgs e)
    {
        var draft = new MixingUnitEntry
        {
            Id = _session.NextUnitId,
            Name = $"Mixing Unit {_session.NextUnitId}",
            Width = _session.Settings.DefaultWidth,
            Height = _session.Settings.DefaultHeight,
            FpsNum = _session.Settings.MasterFpsNum,
            FpsDen = _session.Settings.MasterFpsDen
        };
        draft.EnsureDefaultTransitions();
        var dialog = new MixingUnitWindow(draft, _session.Buses) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        var unit = dialog.Result;
        unit.Id = _session.NextUnitId++;
        unit.EnsureDefaultTransitions();
        unit.AudioBusId = dialog.Result.AudioBusId == 0 ? 1 : dialog.Result.AudioBusId;
        unit.AudioLink = dialog.Result.AudioLink;
        if (TryRemoteMutate(MutationJson.UpsertUnit(unit), Loc.T("chrome.mixingUnit")))
            return;
        MixerNative.ThrowIfFailed(MixerNative.CreateUnit(unit.Id, unit.Width, unit.Height), "Create Mixing Unit");
        MixerNative.ThrowIfFailed(
            MixerNative.ConfigureUnit(unit.Id, unit.Width, unit.Height, unit.FpsNum, unit.FpsDen),
            "Configure Mixing Unit");
        MixerNative.AudioSetUnitLink(unit.Id, unit.AudioBusId, (uint)unit.AudioLink);
        var preview = _session.Scenes.Count > 0 ? _session.Scenes[0].GpuId : MixerNative.Bars;
        var program = _session.Scenes.Count > 1 ? _session.Scenes[1].GpuId : preview;
        MixerApply.PushUnitState(unit.Id, MixerApply.BuildState(unit, program, preview, 0, MixerNative.TransitionFade));
        _session.Units.Add(unit);
        UnitBox.Items.Refresh();
        UnitBox.SelectedItem = unit;
    }

    private void EditUnit_Click(object sender, RoutedEventArgs e)
    {
        var unit = SelectedUnit;
        var dialog = new MixingUnitWindow(unit, _session.Buses) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        unit.Name = dialog.Result.Name;
        unit.Width = dialog.Result.Width;
        unit.Height = dialog.Result.Height;
        unit.FpsNum = dialog.Result.FpsNum;
        unit.FpsDen = dialog.Result.FpsDen;
        unit.AudioBusId = dialog.Result.AudioBusId;
        unit.AudioLink = dialog.Result.AudioLink;
        if (TryRemoteMutate(MutationJson.UpsertUnit(unit), Loc.T("chrome.mixingUnit")))
            return;
        MixerNative.ThrowIfFailed(
            MixerNative.ConfigureUnit(unit.Id, unit.Width, unit.Height, unit.FpsNum, unit.FpsDen),
            "Configure Mixing Unit");
        MixerNative.AudioSetUnitLink(unit.Id, unit.AudioBusId, (uint)unit.AudioLink);
        foreach (var scene in _session.Scenes)
            MixerApply.TryDefineScene(scene, unit.Width, unit.Height);
        foreach (var layout in _session.Multiviews)
            MixerApply.PushMultiview(layout, unit.Width, unit.Height);
        UnitBox.Items.Refresh();
        ApplyAspect();
        if (_switchers.TryGetValue(unit.Id, out var switcher))
            switcher.SyncFromUnit();
    }

    private void DeleteUnit_Click(object sender, RoutedEventArgs e)
    {
        if (_session.Units.Count <= 1)
        {
            MessageBox.Show(this, Loc.T("msg.oneUnit"));
            return;
        }
        var unit = SelectedUnit;
        if (TryRemoteMutate(MutationJson.DeleteUnit(unit.Id), Loc.T("chrome.delete")))
            return;
        foreach (var output in _session.Outputs.Where(item => item.UnitId == unit.Id).ToArray())
        {
            MixerApply.RemoveOutput(output.Id);
            _session.Outputs.Remove(output);
        }
        MixerNative.ThrowIfFailed(MixerNative.DestroyUnit(unit.Id), "Delete Mixing Unit");
        CloseSwitcher(unit.Id);
        _session.Units.Remove(unit);
        UnitBox.Items.Refresh();
        UnitBox.SelectedIndex = 0;
    }

    private void SaveSession_Click(object sender, RoutedEventArgs e)
    {
        if (HostRole.IsRemote)
        {
            SaveRemoteSession();
            return;
        }
        var current = SessionStore.CurrentPath();
        if (SessionStore.CanOverwrite(current))
        {
            SaveSessionTo(current!);
            return;
        }
        SaveSessionAs();
    }

    private void SaveSessionMenu_Click(object sender, RoutedEventArgs e)
    {
        if (HostRole.IsRemote)
            return;
        var menu = new ContextMenu();
        var saveAs = new MenuItem { Header = Loc.T("chrome.saveAs") };
        saveAs.Click += (_, _) => SaveSessionAs();
        var export = new MenuItem { Header = Loc.T("chrome.export") };
        export.Click += (_, _) => ExportSession();
        menu.Items.Add(saveAs);
        menu.Items.Add(export);
        menu.PlacementTarget = SaveSessionMenuButton;
        menu.Placement = PlacementMode.Bottom;
        menu.IsOpen = true;
    }

    private void SaveSessionAs()
    {
        var last = SessionStore.CurrentPath() ?? AppPrefs.Current.RecentSessions.FirstOrDefault();
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            Filter = Loc.T("filter.sessionSave"),
            FileName = string.IsNullOrEmpty(last)
                ? "session.eivz"
                : System.IO.Path.GetFileNameWithoutExtension(last) + ".eivz"
        };
        if (dialog.ShowDialog(this) != true)
            return;
        SaveSessionTo(dialog.FileName);
    }

    private void SaveSessionTo(string path)
    {
        try
        {
            SessionStore.Save(_session, path);
            AppPrefs.Current.RememberSession(path);
            StatusToast.Show(this, Loc.T("msg.saved"));
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, Loc.T("action.Save session"));
        }
    }

    private void SaveRemoteSession()
    {
        if (Application.Current is not App { Backend: RemoteEivizBackend remote } || !remote.Connected)
            return;
        try
        {
            using var payload = JsonDocument.Parse(MixerRemote.SaveSessionJson(remote.Handle));
            var path = payload.RootElement.TryGetProperty("path", out var pathEl)
                ? pathEl.GetString() ?? ""
                : "";
            var history = payload.RootElement.TryGetProperty("historyCount", out var countEl)
                ? countEl.GetUInt32()
                : 0;
            StatusToast.Show(this, Loc.Format("msg.remoteSaved", path, history));
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, Loc.T("action.Save session"));
        }
    }

    private void ExportSession()
    {
        var last = SessionStore.CurrentPath() ?? AppPrefs.Current.RecentSessions.FirstOrDefault();
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            Filter = Loc.T("filter.sessionExport"),
            FileName = string.IsNullOrEmpty(last)
                ? "session.eivzx"
                : System.IO.Path.GetFileNameWithoutExtension(last) + ".eivzx"
        };
        if (dialog.ShowDialog(this) != true)
            return;
        try
        {
            SessionStore.Export(_session, dialog.FileName);
            AppPrefs.Current.RememberSession(dialog.FileName);
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, Loc.T("action.Export session"));
        }
    }

    private void NewSession_Click(object sender, RoutedEventArgs e)
    {
        MixerNative.SessionClearCurrent();
        ((App)Application.Current).ReloadSession(Session.Default());
    }

    private void LoadSession_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFileDialog { Filter = Loc.T("filter.session") };
        if (dialog.ShowDialog(this) != true)
            return;
        uint? historyIndex = null;
        try
        {
            var entries = SessionHistoryDialog.Parse(MixerNative.SessionHistoryText(dialog.FileName));
            if (entries.Count > 0)
            {
                var picker = new SessionHistoryDialog(entries) { Owner = this };
                if (picker.ShowDialog() != true)
                    return;
                historyIndex = picker.HistoryIndex;
            }
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, Loc.T("msg.loadSession"));
            return;
        }
        LoadSessionFrom(dialog.FileName, historyIndex);
    }

    private void LoadLastSession_Click(object sender, RoutedEventArgs e)
    {
        var last = AppPrefs.Current.ExistingSessions().FirstOrDefault();
        if (string.IsNullOrEmpty(last))
        {
            MessageBox.Show(this, Loc.T("msg.noLastSession"), Loc.T("chrome.loadLast"));
            return;
        }
        LoadSessionFrom(last);
    }

    private void RelinkMedia_Click(object sender, RoutedEventArgs e)
    {
        var directory = PickRelinkDirectory();
        if (string.IsNullOrEmpty(directory))
            return;
        var dirs = new[] { directory };
        var before = _session.Inputs.ToDictionary(input => input.Id, input => input.PathOrAddress);
        int updated;
        if (TryRemoteMutate(MutationJson.RelinkMedia(dirs), Loc.T("input.relinkFolder")))
        {
            updated = _session.Inputs.Count(input =>
                before.TryGetValue(input.Id, out var path) && path != input.PathOrAddress);
        }
        else
        {
            updated = SessionStore.RelinkMissingMedia(_session, dirs);
            SessionStore.Publish(_session);
            RefreshInputList();
        }
        MessageBox.Show(this, Loc.Format("msg.relinked", updated), Loc.T("input.relinkFolder"));
    }

    private void RelinkInputFile_Click(object sender, RoutedEventArgs e)
    {
        if (InputList.SelectedItem is not InputEntry input
            || input.Kind is not (InputKind.Still or InputKind.Video))
        {
            MessageBox.Show(this, Loc.T("msg.selectInputRelink"));
            return;
        }
        var path = PickRelinkFile(input);
        if (string.IsNullOrEmpty(path))
            return;
        if (HostRole.IsRemote)
        {
            input.PathOrAddress = path;
            RemoteMutate(MutationJson.UpsertInput(input), Loc.T("input.relinkFile"));
            return;
        }
        if (!SessionStore.RelinkInput(input, path))
            return;
        SessionStore.Publish(_session);
        RefreshInputList();
        MessageBox.Show(this, Loc.Format("msg.relinked", 1), Loc.T("input.relinkFile"));
    }

    private string? PickRelinkDirectory()
    {
        if (HostRole.IsRemote)
        {
            var prompt = new RelinkMediaDialog { Owner = this };
            return prompt.ShowDialog() == true ? prompt.Directory : null;
        }
        var dialog = new Microsoft.Win32.OpenFolderDialog { Title = Loc.T("input.relinkFolder") };
        return dialog.ShowDialog(this) == true ? dialog.FolderName : null;
    }

    internal string? PickRelinkFile(InputEntry input, Window? owner = null)
    {
        owner ??= this;
        if (HostRole.IsRemote)
        {
            var prompt = new RelinkMediaDialog(Loc.T("input.relinkFile"), Loc.T("input.relinkFileHost"))
            {
                Owner = owner
            };
            return prompt.ShowDialog() == true ? prompt.Directory : null;
        }
        var dialog = new Microsoft.Win32.OpenFileDialog
        {
            Title = Loc.T("input.relinkFile"),
            Filter = input.Kind == InputKind.Still
                ? "Images|*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff|All|*.*"
                : "Video|*.mp4;*.mov;*.mkv;*.avi;*.wmv;*.mxf|All|*.*"
        };
        return dialog.ShowDialog(owner) == true ? dialog.FileName : null;
    }

    private void LoadSessionFrom(string path, uint? historyIndex = null)
    {
        try
        {
            if (historyIndex is null && SessionStore.FileHasAssets(path))
            {
                if (!TryImportExport(path, out var imported, out var dest))
                    return;
                ((App)Application.Current).ReloadSession(imported);
                AppPrefs.Current.RememberSession(dest);
                return;
            }
            ((App)Application.Current).ReloadSession(SessionStore.Load(path, historyIndex));
            AppPrefs.Current.RememberSession(path);
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, Loc.T("msg.loadSession"));
        }
    }

    private bool TryImportExport(string exportPath, out Session session, out string dest)
    {
        session = null!;
        dest = "";
        var prompt = new ImportExportDialog(exportPath) { Owner = this };
        if (prompt.ShowDialog() != true)
            return false;
        session = SessionStore.Import(exportPath, prompt.SessionPath, prompt.MediaDirectory);
        dest = prompt.SessionPath;
        return true;
    }

    internal void PromptMissingMedia()
    {
        if (HostRole.IsRemote)
            return;
        var missing = SessionStore.MissingMedia(_session);
        if (missing.Count == 0)
            return;
        var dialog = new MissingMediaDialog(_session) { Owner = this };
        dialog.ShowDialog();
        SessionStore.Publish(_session);
        RefreshInputList();
    }

    private void ShowInputInExplorer_Click(object sender, RoutedEventArgs e)
    {
        if (InputList.SelectedItem is not InputEntry input)
            return;
        ShowPathInExplorer(input.PathOrAddress);
    }

    internal static void ShowPathInExplorer(string? path)
    {
        if (string.IsNullOrWhiteSpace(path))
            return;
        var target = path.Trim();
        if (!File.Exists(target) && !Directory.Exists(target))
        {
            var parent = Path.GetDirectoryName(target);
            if (string.IsNullOrEmpty(parent) || !Directory.Exists(parent))
                return;
            target = parent;
        }
        Process.Start(new ProcessStartInfo
        {
            FileName = "explorer.exe",
            Arguments = Directory.Exists(target) ? $"\"{target}\"" : $"/select,\"{target}\"",
            UseShellExecute = true
        });
    }

    internal void CloseOwnedSurfaces()
    {
        _overlay?.Close();
        _resourcesWindow?.Close();
        _logWindow?.Close();
        CloseAllSwitchers();
        foreach (var window in _multiviews.ToArray())
            window.Close();
        foreach (var preview in _inputPreviews.Values.ToArray())
            preview.Close();
        foreach (var audio in _audioInputs.Values.ToArray())
            audio.Close();
        PreviewHost.AutoAttach = false;
        ProgramHost.AutoAttach = false;
        MainMultiviewHost.AutoAttach = false;
        PreviewHost.ReleaseNative();
        ProgramHost.ReleaseNative();
        MainMultiviewHost.ReleaseNative();
        foreach (var tile in ScenePanel.Children.OfType<SceneTile>())
            tile.SetThumbWanted(false);
    }

    private void Preferences_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new PreferencesWindow { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        var app = (App)Application.Current;
        if (dialog.RendererChanged)
            app.ReloadSession(_session);
        else if (HostRole.IsRemote && dialog.RemoteOmtDecodeChanged)
        {
            BindMainVideo();
            app.Backend.SyncPublishedVideo();
        }
    }

    private void Settings_Click(object sender, RoutedEventArgs e) => OpenSettings(0);

    internal void OpenSettings(int category)
    {
        var dialog = new SettingsWindow(_session, category) { Owner = this };
        if (dialog.ShowDialog() != true)
            return;
        if (App.IsRemote)
        {
            RemoteMutate(
                MutationJson.SetSettings(
                    dialog.Settings,
                    dialog.Outputs,
                    dialog.Buses,
                    dialog.HeadphoneCopyMaster,
                    dialog.NextOutputId,
                    dialog.NextBusId),
                Loc.T("chrome.settings"));
            return;
        }
        var restartMedia = _session.Settings.InternalColorFormat != dialog.Settings.InternalColorFormat
            || _session.Settings.FrameBufferFrames != dialog.Settings.FrameBufferFrames;
        _session.Settings.MasterFpsNum = dialog.Settings.MasterFpsNum;
        _session.Settings.MasterFpsDen = dialog.Settings.MasterFpsDen;
        _session.Settings.DefaultWidth = dialog.Settings.DefaultWidth;
        _session.Settings.DefaultHeight = dialog.Settings.DefaultHeight;
        _session.Settings.DefaultMultiviewUnitId = dialog.Settings.DefaultMultiviewUnitId;
        _session.Settings.FrameBufferFrames = dialog.Settings.FrameBufferFrames;
        _session.Settings.DefaultPresentInterval = dialog.Settings.DefaultPresentInterval;
        _session.Settings.FlipSwapchainLimit = dialog.Settings.FlipSwapchainLimit;
        FlipBudget.Configure(_session.Settings.FlipSwapchainLimit);
        _session.Settings.InternalColorFormat = dialog.Settings.InternalColorFormat;
        _session.Settings.RebarOptimization = dialog.Settings.RebarOptimizationEnabled;
        _session.Settings.NdiGpuUpload = dialog.Settings.NdiGpuUploadEnabled;
        _session.Settings.PreviewColor = RgbColor.FromOrDefault(dialog.Settings.PreviewColor, RgbColor.PreviewDefault);
        _session.Settings.ProgramColor = RgbColor.FromOrDefault(dialog.Settings.ProgramColor, RgbColor.ProgramDefault);
        _session.Settings.InactiveColor = RgbColor.FromOrDefault(dialog.Settings.InactiveColor, RgbColor.InactiveDefault);
        _session.Settings.MultiviewLabelSize = dialog.Settings.MultiviewLabelSize;
        _session.Settings.MultiviewLabelUnit = dialog.Settings.MultiviewLabelUnit;
        _session.Settings.MultiviewLabelAnchor = dialog.Settings.MultiviewLabelAnchor;
        _session.Settings.VmixApiEnabled = dialog.Settings.VmixApiEnabledValue;
        _session.Settings.VmixApiPort = dialog.Settings.VmixApiPort == 0 ? 8088 : dialog.Settings.VmixApiPort;
        _session.Settings.VmixApiUser = dialog.Settings.VmixApiUser ?? "";
        _session.Settings.VmixApiPassword = dialog.Settings.VmixApiPassword ?? "";
        _session.Settings.VmixTcpEnabled = dialog.Settings.VmixTcpEnabledValue;
        _session.Settings.NativeApiEnabled = dialog.Settings.NativeApiEnabledValue;
        _session.Settings.NativeApiPort = dialog.Settings.NativeApiPort == 0 ? 9400 : dialog.Settings.NativeApiPort;
        AppPrefs.Current.NativeApiEnabled = _session.Settings.NativeApiEnabledValue;
        AppPrefs.Current.NativeApiPort = _session.Settings.NativeApiPort;
        AppPrefs.Current.Save();
        BusTheme.PushMultiviewLabels(_session);
        ApplyBusColors();
        RefreshSceneTiles();
        RebuildTransitions();
        foreach (var window in _switchers.Values)
            window.ApplyBusColors();
        _session.HeadphoneCopyMaster = dialog.HeadphoneCopyMaster;
        _session.Buses.Clear();
        foreach (var bus in dialog.Buses)
            _session.Buses.Add(bus);
        AudioGraphSync.Push(_session);
        MixerNative.ThrowIfFailed(
            MixerNative.SetFrameBuffer(_session.Settings.FrameBufferFrames),
            "Set frame buffer");
        MixerNative.ThrowIfFailed(
            MixerNative.SetRebarOptimization(_session.Settings.RebarOptimizationEnabled ? 1u : 0u),
            "Set ReBAR optimization");
        MixerNative.ThrowIfFailed(
            MixerNative.SetNdiGpuUpload(_session.Settings.NdiGpuUploadEnabled ? 1u : 0u),
            "Set NDI GPU upload");
        foreach (var layout in _session.Multiviews)
        {
            layout.PushPresentInterval(_session.Settings);
            MixerApply.PushMultiview(layout, SelectedUnit.Width, SelectedUnit.Height);
        }
        foreach (var window in _multiviews)
            window.SyncPresentInterval();
        PushScenePresentIntervals();
        MixerNative.VideoFormat = _session.Settings.InternalColorFormat == InternalColorFormat.Bgra
            ? MixerNative.FormatBgra
            : MixerNative.FormatUyvy;
        if (restartMedia)
            RestartMediaPumps();
        ApplyOutputs(dialog.Outputs);
        RebuildMeters();
        App.ApplyVmixApi();
        SessionStore.Publish(_session);
    }

    private void RestartMediaPumps()
    {
        foreach (var input in _session.Inputs)
        {
            if (string.IsNullOrWhiteSpace(input.PathOrAddress))
                continue;
            if (input.Kind == InputKind.Video)
            {
                var playing = input.VideoStartsPlaying;
                var position = 0L;
                if (TryVideoInfo(input.Id, out var info))
                {
                    playing = info.Playing != 0;
                    position = info.PositionHns;
                }
                MixerApply.StartVideo(
                    input.Id,
                    input.PathOrAddress,
                    input.VideoLoop,
                    playing,
                    input.FrameBufferFrames,
                    position);
            }
            else if (input.Kind == InputKind.UVC)
                MixerApply.StartUvc(input.Id, input.PathOrAddress, input.CaptureWidth, input.CaptureHeight, input.CaptureFpsNum, input.CaptureFpsDen, input.FrameBufferFrames);
        }
    }

    internal void RemoteMutate(string json, string title, bool reloadDocument = true)
    {
        if (!App.IsRemote || Application.Current is not App app)
            return;
        var backend = app.Backend;
        var dispatcher = Dispatcher;
        Task.Run(() =>
        {
            var revision = backend.Revision;
            var ok = backend.Mutate(json, revision, out var error);
            dispatcher.BeginInvoke(() =>
            {
                if (reloadDocument)
                    backend.Poll();
                else if (backend is RemoteEivizBackend remote)
                    remote.AcknowledgeDocument();
                if (!ok)
                    MessageBox.Show(this, error, title);
            });
        });
    }

    private bool TryRemoteMutate(string json, string title)
    {
        if (!App.IsRemote)
            return false;
        RemoteMutate(json, title);
        return true;
    }

    private void ApplyOutputs(IReadOnlyList<OutputEntry> outputs)
    {
        var next = outputs.ToList();
        var previous = _session.Outputs.ToList();
        var nextIds = next.Select(item => item.Id).ToHashSet();
        foreach (var existing in previous.Where(item => !nextIds.Contains(item.Id)))
            MixerApply.RemoveOutput(existing.Id);
        foreach (var output in next)
        {
            var prior = previous.FirstOrDefault(item => item.Id == output.Id);
            if (prior is not null && SameOutput(prior, output))
                continue;
            if (prior is not null)
                MixerApply.RemoveOutput(output.Id);
            if (!output.Enabled)
                continue;
            if (output.Transport is OutputTransport.Omt or OutputTransport.Ndi)
            {
                MixerApply.TryAddOutput(output);
                continue;
            }
            try
            {
                MixerApply.AddOutput(output);
            }
            catch (Exception ex)
            {
                MessageBox.Show(this, ex.Message, Loc.T("msg.output"));
            }
        }
        _session.Outputs.Clear();
        foreach (var output in next)
            _session.Outputs.Add(output);
    }

    private static bool SameOutput(OutputEntry left, OutputEntry right) =>
        left.Id == right.Id
        && left.Name == right.Name
        && left.Transport == right.Transport
        && left.SourceKind == right.SourceKind
        && left.SourceId == right.SourceId
        && left.UnitId == right.UnitId
        && left.UseGpu == right.UseGpu
        && left.Enabled == right.Enabled
        && left.AudioBusId == right.AudioBusId
        && left.SkipEncodeWhenNoReceivers == right.SkipEncodeWhenNoReceivers
        && left.Width == right.Width
        && left.Height == right.Height
        && left.FpsNum == right.FpsNum
        && left.FpsDen == right.FpsDen;
}
