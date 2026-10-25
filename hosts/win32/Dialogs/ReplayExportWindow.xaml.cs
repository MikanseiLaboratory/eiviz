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
    private readonly Func<string, ReplayExportRequest> _buildRequest;
    private readonly Func<(uint Width, uint Height, uint FpsNum, uint FpsDen, uint Bitrate)> _format;
    private readonly Func<IReadOnlyList<(uint Id, string Label)>> _angles;
    private readonly Func<bool> _isRecording;
    private readonly Action<string, string> _start;

    /// <summary>
    /// Builds the window.
    /// </summary>
    /// <param name="modes">What the operator may export, already localised.</param>
    /// <param name="buildRequest">What a mode key exports, in order.</param>
    /// <param name="format">Picture format and bitrate for the file.</param>
    /// <param name="angles">Angles an all-angles export writes, one file each.</param>
    /// <param name="isRecording">Whether a recording runs: the export asks first.</param>
    /// <param name="start">Hands one export body to the mixer, with a label for the UI.</param>
    public ReplayExportWindow(
        IReadOnlyList<ExportModeChoice> modes,
        Func<string, ReplayExportRequest> buildRequest,
        Func<(uint Width, uint Height, uint FpsNum, uint FpsDen, uint Bitrate)> format,
        Func<IReadOnlyList<(uint Id, string Label)>> angles,
        Func<bool> isRecording,
        Action<string, string> start)
    {
        _modes = modes;
        _buildRequest = buildRequest;
        _format = format;
        _angles = angles;
        _isRecording = isRecording;
        _start = start;
        InitializeComponent();
        ExportModeBox.ItemsSource = _modes;
        ExportModeBox.DisplayMemberPath = nameof(ExportModeChoice.Label);
        ExportModeBox.SelectedIndex = 0;
        // The window follows its own export. The deck reports progress for the files it
        // queued, but this window can start one nothing else is tracking — an export begun
        // here while the deck's queue was empty was never reported, so the button stayed
        // disabled and the operator saw no sentence at all.
        _tick = new System.Windows.Threading.DispatcherTimer
        {
            Interval = TimeSpan.FromMilliseconds(200),
        };
        _tick.Tick += (_, _) => Poll();
    }

    private readonly System.Windows.Threading.DispatcherTimer _tick;

    private void Poll()
    {
        if (!IsBusy)
            return;
        var progress = Backend?.ReplayExportProgress() ?? "";
        if (progress.Length == 0)
            return;
        var step = ReplayExportProgress.Parse(progress);
        ShowProgress(step.State switch
        {
            1 => Loc.Format("replay.exportRunning", step.Done, step.Total, step.Skipped),
            2 => Loc.Format("replay.exportDone", step.Done, step.Skipped),
            3 => Loc.Format("replay.exportError", step.Message),
            4 => Loc.T("replay.exportCancelled"),
            _ => "",
        });
        // The state is what says the file is finished, not the sentence: the sentence is
        // localised and formatted, so matching its text would only work in one language.
        if (step.State is 2 or 3 or 4)
            Release();
    }

    private static IEivizBackend? Backend => (Application.Current as App)?.Backend;

    /// <summary>Shows what a running export is doing. Called by the deck's poll.</summary>
    public void ShowProgress(string text)
    {
        if (ExportStatusText is not null)
            ExportStatusText.Text = text;
    }

    /// <summary>Whether the window is waiting on an export to finish.</summary>
    public bool IsBusy { get; private set; }

    /// <summary>Lets the operator start another file.</summary>
    private void Release()
    {
        if (!IsBusy)
            return;
        IsBusy = false;
        _tick.Stop();
        ExportStartButton.IsEnabled = true;
    }

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
        var request = _buildRequest(mode.Key);
        if (request.IsEmpty)
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
        var (segments, clips) = request.Members();
        string Body(string target) =>
            $"{{\"path\":{JsonSerializer.Serialize(target)},\"width\":{width},\"height\":{height}," +
            $"\"fpsNum\":{fpsNum},\"fpsDen\":{fpsDen},\"bitrate\":{bitrate},\"sampleRate\":48000," +
            $"\"channels\":2,\"withAudio\":true,\"codec\":\"{codec}\"," +
            $"\"segments\":{segments},\"clips\":{clips}}}";
        IsBusy = true;
        ExportStartButton.IsEnabled = false;
        _tick.Start();
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

    protected override void OnClosed(EventArgs e)
    {
        // The timer outlives the window's controls, and a tick after they are gone would
        // write to them.
        _tick.Stop();
        base.OnClosed(e);
    }
}

/// <summary>One entry of the "what to export" list.</summary>
public sealed record ExportModeChoice(string Key, string Label);
