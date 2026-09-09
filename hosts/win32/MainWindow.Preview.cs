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
    private void OpenMultiview_Click(object sender, RoutedEventArgs e)
    {
        if (HostRole.IsRemote)
        {
            OpenSettings(3);
            return;
        }
        var menu = new ContextMenu();
        foreach (var layout in _session.Multiviews)
        {
            var item = new MenuItem { Header = layout.Name, Tag = layout };
            item.Click += (_, _) => OpenMultiviewWindow(layout);
            menu.Items.Add(item);
        }
        menu.Items.Add(new Separator());
        var create = new MenuItem { Header = Loc.T("chrome.newMultiview") };
        create.Click += (_, _) => OpenNewMultiview(_session.Settings.DefaultMultiviewUnitId);
        menu.Items.Add(create);
        menu.PlacementTarget = MultiviewButton;
        menu.Placement = PlacementMode.Bottom;
        menu.IsOpen = true;
    }

    private void OpenRecent_Click(object sender, RoutedEventArgs e)
    {
        var menu = new ContextMenu();
        if (HostRole.IsRemote)
        {
            var remotes = AppPrefs.Current.RecentRemotes;
            if (remotes.Count == 0)
                menu.Items.Add(new MenuItem { Header = Loc.T("chrome.connectRecent"), IsEnabled = false });
            else
            {
                menu.Items.Add(new MenuItem { Header = Loc.T("chrome.connectRecent"), IsEnabled = false });
                foreach (var url in remotes)
                {
                    var item = new MenuItem { Header = RemoteEndpoint.Display(url), Tag = url };
                    item.Click += (_, _) => ConnectTo(url, CredentialStore.Load(url));
                    menu.Items.Add(item);
                }
            }
        }
        else
        {
            var recent = AppPrefs.Current.ExistingSessions().ToList();
            if (recent.Count == 0)
            {
                menu.Items.Add(new MenuItem { Header = Loc.T("chrome.openRecent"), IsEnabled = false });
            }
            else
            {
                var header = new MenuItem { Header = Loc.T("chrome.openRecent"), IsEnabled = false };
                menu.Items.Add(header);
                foreach (var path in recent)
                {
                    var item = new MenuItem { Header = System.IO.Path.GetFileName(path), Tag = path };
                    item.Click += (_, _) => LoadSessionFrom(path);
                    menu.Items.Add(item);
                }
            }
        }
        menu.PlacementTarget = OpenRecentButton;
        menu.Placement = PlacementMode.Bottom;
        menu.IsOpen = true;
    }

    internal void OpenNewMultiview(ulong unitId)
    {
        if (!HostRole.IsRemote && !FlipBudget.TryOpen(1, this))
            return;
        var unit = _session.Units.FirstOrDefault(item => item.Id == unitId) ?? SelectedUnit;
        if (App.IsRemote)
        {
            var draft = DraftMultiview(unit.Id);
            RemoteMutate(MutationJson.UpsertMultiview(draft), Loc.T("chrome.multiview"));
            return;
        }
        var layout = _session.AddMultiview(unitId: unit.Id);
        MixerApply.PushMultiview(layout, unit.Width, unit.Height);
        OpenMultiviewWindow(layout);
    }

    private MultiviewLayout DraftMultiview(ulong unitId)
    {
        var layout = new MultiviewLayout
        {
            Id = _session.NextMultiviewId,
            Name = $"Multiview {_session.NextMultiviewId}",
            PreviewUnitId = unitId,
            ProgramUnitId = unitId,
            LabelAnchor = _session.Settings.MultiviewLabelAnchor,
            LabelSize = _session.Settings.MultiviewLabelSize,
            LabelUnit = _session.Settings.MultiviewLabelUnit,
            AlwaysOnTop = true
        };
        layout.EnsureTiles();
        layout.SeedDefaultBuses(unitId);
        return layout;
    }

    internal void OpenMultiviewWindow(MultiviewLayout layout)
    {
        if (HostRole.IsRemote)
        {
            OpenSettings(3);
            return;
        }
        var existing = _multiviews.FirstOrDefault(item => item.LayoutId == layout.Id);
        if (existing is not null)
        {
            existing.Activate();
            return;
        }
        if (!FlipBudget.TryOpen(1, this))
            return;
        var unit = SelectedUnit;
        MixerApply.PushMultiview(layout, unit.Width, unit.Height);
        var window = new MultiviewWindow(_session, layout);
        if (layout.AlwaysOnTop)
            window.Owner = this;
        window.Topmost = layout.AlwaysOnTop;
        window.Closed += (_, _) => _multiviews.Remove(window);
        _multiviews.Add(window);
        window.Show();
    }

    internal void ApplyMultiviewTopmost(MultiviewLayout layout)
    {
        foreach (var window in _multiviews)
        {
            if (window.LayoutId != layout.Id)
                continue;
            window.Topmost = layout.AlwaysOnTop;
            window.Owner = layout.AlwaysOnTop ? this : null;
        }
    }

    internal void SyncMultiviewPresent(MultiviewLayout layout)
    {
        layout.PushPresentInterval(_session.Settings);
        foreach (var window in _multiviews)
        {
            if (window.LayoutId == layout.Id)
                window.SyncPresentInterval();
        }
    }

    internal void OpenMultiviewFor(ulong unitId) => OpenNewMultiview(unitId);

    internal void CloseMultiview(ulong layoutId)
    {
        foreach (var window in _multiviews.Where(item => item.LayoutId == layoutId).ToArray())
            window.Close();
    }

    private void TickVideo()
    {
        if (InputList.SelectedItem is not InputEntry { Kind: InputKind.Video } input)
        {
            VideoBar.Visibility = Visibility.Collapsed;
            return;
        }
        if (!TryVideoInfo(input.Id, out var info) || info.IsFile == 0)
        {
            VideoBar.Visibility = Visibility.Collapsed;
            return;
        }
        VideoBar.Visibility = Visibility.Visible;
        VideoTitle.Text = input.Name;
        var duration = info.DurationHns;
        var position = Math.Max(0, info.PositionHns);
        if (duration > 0)
        {
            position = Math.Min(position, duration);
            VideoSeek.IsEnabled = true;
            VideoTimeText.Text = $"{FormatHns(position)} / {FormatHns(duration - position)} / {FormatHns(duration)}";
            if (!_videoSeeking)
            {
                _videoSeekSuppress = true;
                VideoSeek.Value = position / (double)duration;
                _videoSeekSuppress = false;
            }
        }
        else
        {
            VideoSeek.IsEnabled = false;
            VideoTimeText.Text = $"{FormatHns(position)} / -- / --";
            if (!_videoSeeking)
            {
                _videoSeekSuppress = true;
                VideoSeek.Value = 0;
                _videoSeekSuppress = false;
            }
        }
        VideoPlay.Content = info.Playing != 0 ? "❚❚" : "▶";
    }

    private static bool TryVideoInfo(ulong id, out MixerVideoInfo info) =>
        MixerNative.TryCopyVideoInfo(id, out info);

    private ulong? SelectedVideoId() =>
        InputList.SelectedItem is InputEntry { Kind: InputKind.Video } input
            ? input.Id
            : null;

    private void InputList_SelectionChanged(object sender, SelectionChangedEventArgs e) => TickVideo();

    private static string FormatHns(long hns)
    {
        var time = TimeSpan.FromTicks(Math.Max(0, hns));
        return time.ToString(time.TotalHours >= 1 ? @"h\:mm\:ss" : @"mm\:ss");
    }

    private void VideoPlay_Click(object sender, RoutedEventArgs e)
    {
        if (SelectedVideoId() is not ulong id)
            return;
        if (App.IsRemote)
        {
            ((App)Application.Current).Backend.VideoPlay(id, true);
            return;
        }
        if (!TryVideoInfo(id, out var info))
            return;
        MixerNative.VideoSetPlaying(id, info.Playing == 0 ? 1u : 0u);
        TickVideo();
    }

    private void VideoRestart_Click(object sender, RoutedEventArgs e)
    {
        if (SelectedVideoId() is not ulong id)
            return;
        if (App.IsRemote)
            ((App)Application.Current).Backend.VideoSeek(id, 0);
        else
            MixerNative.VideoSeek(id, 0);
        TickVideo();
    }

    private void VideoSeek_DragStarted(object sender, DragStartedEventArgs e) => _videoSeeking = true;

    private void VideoSeek_DragCompleted(object sender, DragCompletedEventArgs e)
    {
        SeekFromSlider(force: true);
        _videoSeeking = false;
    }

    private void VideoSeek_ClickSeek(object sender, MouseButtonEventArgs e)
    {
        if (_videoSeeking)
            return;
        SeekFromSlider(force: true);
    }

    private void VideoSeek_ValueChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (_videoSeekSuppress)
            return;
        if (SelectedVideoId() is not ulong id || !TryVideoInfo(id, out var info) || info.DurationHns <= 0)
            return;
        var duration = info.DurationHns;
        VideoTimeText.Text = $"{FormatHns((long)(e.NewValue * duration))} / {FormatHns((long)((1 - e.NewValue) * duration))} / {FormatHns(duration)}";
        SeekFromSlider(force: !_videoSeeking);
    }

    private void SeekFromSlider(bool force)
    {
        if (SelectedVideoId() is not ulong id || !TryVideoInfo(id, out var info) || info.DurationHns <= 0)
            return;
        if (!force)
        {
            var now = Environment.TickCount64;
            if (now - _lastSeekSentMs < 120)
                return;
            _lastSeekSentMs = now;
        }
        else
        {
            _lastSeekSentMs = Environment.TickCount64;
        }
        if (App.IsRemote)
        {
            ((App)Application.Current).Backend.VideoSeek(id, (long)(Math.Clamp(VideoSeek.Value, 0, 1) * Math.Max(1, info.DurationHns)));
            return;
        }
        MixerNative.VideoSeek(id, (long)(Math.Clamp(VideoSeek.Value, 0, 1) * info.DurationHns));
    }

    private void ApplyAspect()
    {
        var unit = SelectedUnit;
        PreviewAspect.RatioWidth = unit.Width;
        PreviewAspect.RatioHeight = unit.Height;
        ProgramAspect.RatioWidth = unit.Width;
        ProgramAspect.RatioHeight = unit.Height;
    }

    private void ApplyBusColors()
    {
        BusTheme.Apply(_session.Settings, PreviewFrame, PreviewHeader, PreviewHeaderText, preview: true);
        BusTheme.Apply(_session.Settings, ProgramFrame, ProgramHeader, ProgramHeaderText, preview: false);
    }

    private void RefreshMultiviewLabels()
    {
        var unit = SelectedUnit;
        foreach (var layout in _session.Multiviews)
            MixerApply.PushMultiview(layout, unit.Width, unit.Height);
    }

    private void Snapshot_Click(object sender, RoutedEventArgs e) =>
        SaveSnapshot(SelectedUnit.Id, MixerNative.OutputProgram, SelectedUnit.Name);

    internal void SnapshotScene(SceneEntry scene) =>
        SaveSnapshot(scene.GpuId, 0, scene.Name);

    internal void SnapshotInput(InputEntry input) =>
        SaveSnapshot(input.Id, MixerNative.OutputSource, input.Name);

    private void SaveSnapshot(ulong sourceId, uint kind, string name)
    {
        if (HostRole.IsRemote)
            return;
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            Filter = Loc.T("filter.snapshot"),
            DefaultExt = ".png",
            AddExtension = true,
            FileName = SnapshotFileName(name)
        };
        if (dialog.ShowDialog(this) != true)
            return;
        try
        {
            MixerNative.ThrowIfFailed(
                MixerNative.Snapshot(sourceId, kind, dialog.FileName),
                "Screenshot");
        }
        catch (Exception ex)
        {
            MessageBox.Show(this, ex.Message, Loc.T("chrome.screenshot"));
        }
    }

    private static string SnapshotFileName(string name)
    {
        var invalid = Path.GetInvalidFileNameChars();
        var cleaned = new string(name.Select(ch => invalid.Contains(ch) ? '_' : ch).ToArray());
        if (string.IsNullOrWhiteSpace(cleaned))
            cleaned = "eiviz";
        return cleaned + ".png";
    }

}
