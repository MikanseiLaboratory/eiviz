using System.IO;
using System.Text.Json;
using System.Windows;
using Eiviz.Host.I18n;

namespace Eiviz.Host.Dialogs;

/// <summary>
/// The export settings, in their own window. The deck keeps one button for it: the
/// mode, the codec and the path are set here, and the progress of a running export is
/// shown here too.
///
/// The window owns no recording state. It is handed the segments to write, or a
/// callback that builds them when the operator presses Export, so the deck stays the
/// only place that decides what "the selected clip" or "this angle" means.
/// </summary>
public partial class ReplayExportWindow : Window
{
    private readonly IReadOnlyList<ExportModeChoice> _modes;
    private readonly Func<string, List<(uint AngleId, ulong From, ulong To)>> _buildSegments;
    private readonly Func<(uint Width, uint Height, uint FpsNum, uint FpsDen, uint Bitrate)> _format;
    private readonly Func<IReadOnlyList<(uint Id, string Label)>> _angles;
    private readonly Func<bool> _isRecording;
    private readonly Action<string, string> _start;

    /// <summary>
    /// Builds the window.
    /// </summary>
    /// <param name="modes">What the operator may export, already localised.</param>
    /// <param name="buildSegments">Segments for a mode key, in order.</param>
    /// <param name="format">Picture format and bitrate for the file.</param>
    /// <param name="angles">Angles an all-angles export writes, one file each.</param>
    /// <param name="isRecording">Whether a recording runs: the export asks first.</param>
    /// <param name="start">Hands one export body to the mixer, with a label for the UI.</param>
    public ReplayExportWindow(
        IReadOnlyList<ExportModeChoice> modes,
        Func<string, List<(uint AngleId, ulong From, ulong To)>> buildSegments,
        Func<(uint Width, uint Height, uint FpsNum, uint FpsDen, uint Bitrate)> format,
        Func<IReadOnlyList<(uint Id, string Label)>> angles,
        Func<bool> isRecording,
        Action<string, string> start)
    {
        _modes = modes;
        _buildSegments = buildSegments;
        _format = format;
        _angles = angles;
        _isRecording = isRecording;
        _start = start;
        InitializeComponent();
        ExportModeBox.ItemsSource = _modes;
        ExportModeBox.DisplayMemberPath = nameof(ExportModeChoice.Label);
        ExportModeBox.SelectedIndex = 0;
    }

    /// <summary>Shows what a running export is doing. Called by the deck's poll.</summary>
    public void ShowProgress(string text)
    {
        if (ExportStatusText is not null)
            ExportStatusText.Text = text;
    }

    /// <summary>Whether the window is waiting on an export to finish.</summary>
    public bool IsBusy { get; private set; }

    private void Browse_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            FileName = $"replay-{DateTime.Now:yyyyMMdd-HHmmss}.mp4",
            Filter = "MP4|*.mp4",
            OverwritePrompt = true
        };
        if (dialog.ShowDialog(this) == true)
            ExportPathBox.Text = dialog.FileName;
    }

    private void Start_Click(object sender, RoutedEventArgs e)
    {
        if (ExportModeBox.SelectedItem is not ExportModeChoice mode)
        {
            // Saying nothing here left the button looking dead.
            ExportStatusText.Text = Loc.T("replay.exportNeedMode");
            return;
        }
        var path = ExportPathBox.Text.Trim();
        if (path.Length == 0)
        {
            ExportStatusText.Text = Loc.T("replay.exportNeedPath");
            return;
        }
        // A folder is a reasonable thing to type, and the MP4 writer answers it with an
        // opaque E_INVALIDARG. Put a name inside it instead of failing.
        if (Directory.Exists(path))
        {
            path = Path.Combine(path, $"replay-{DateTime.Now:yyyyMMdd-HHmmss}.mp4");
            ExportPathBox.Text = path;
        }
        else if (!Path.HasExtension(path))
        {
            // The MP4 writer picks its container from the extension; a path without one
            // fails with a bare E_INVALIDARG that says nothing about the cause.
            path += ".mp4";
            ExportPathBox.Text = path;
        }
        var segments = string.Join(",", _buildSegments(mode.Key).Select(segment =>
            $"{{\"angleId\":{segment.AngleId},\"fromFrame\":{segment.From},\"toFrame\":{segment.To}}}"));
        if (segments.Length == 0)
        {
            ExportStatusText.Text = Loc.T("replay.exportNothing");
            return;
        }
        if (_isRecording())
        {
            var answer = MessageBox.Show(
                this,
                Loc.T("replay.exportWhileRecording"),
                Loc.T("replay.export"),
                MessageBoxButton.OKCancel,
                MessageBoxImage.Warning);
            if (answer != MessageBoxResult.OK)
                return;
        }
        var codec = ExportCodecBox.SelectedIndex == 1 ? "Hevc" : "H264";
        var (width, height, fpsNum, fpsDen, bitrate) = _format();
        string Body(string target) =>
            $"{{\"path\":{JsonSerializer.Serialize(target)},\"width\":{width},\"height\":{height}," +
            $"\"fpsNum\":{fpsNum},\"fpsDen\":{fpsDen},\"bitrate\":{bitrate},\"sampleRate\":48000," +
            $"\"channels\":2,\"withAudio\":true,\"codec\":\"{codec}\",\"segments\":[{segments}]}}";
        IsBusy = true;
        ExportStartButton.IsEnabled = false;
        if (mode.Key == "AllAngles")
        {
            // One file per angle, so a multi-camera export stays usable.
            foreach (var angle in _angles())
            {
                var target = Path.Combine(
                    Path.GetDirectoryName(path) ?? "",
                    $"{Path.GetFileNameWithoutExtension(path)}-angle{angle.Id}{Path.GetExtension(path)}");
                _start(Body(target), $"{angle.Label} → {Path.GetFileName(target)}");
            }
        }
        else
        {
            _start(Body(path), Path.GetFileName(path));
        }
    }

    private void Close_Click(object sender, RoutedEventArgs e) => Close();
}

/// <summary>One entry of the "what to export" list.</summary>
public sealed record ExportModeChoice(string Key, string Label);
