using System.Diagnostics;
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Threading;

namespace Eiviz.Host.Dialogs;

public partial class LogWindow : Window
{
    private const int MaxLines = 4000;
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(200) };
    private readonly LogTail _hostTail = new(HostLog.HostFilePath);
    private readonly LogTail _mixerTail = new(HostLog.MixerFilePath);
    private readonly List<(string Source, string Line)> _lines = [];
    private bool _hostLive;

    public LogWindow()
    {
        InitializeComponent();
        PathText.Text = HostLog.DirectoryPath;
        Loaded += (_, _) =>
        {
            Pump();
            HostLog.LineWritten += OnHostLine;
            _hostLive = true;
            _timer.Start();
        };
        Closed += (_, _) =>
        {
            _timer.Stop();
            HostLog.LineWritten -= OnHostLine;
        };
        _timer.Tick += (_, _) =>
        {
            if (PauseBox.IsChecked == true)
                return;
            Pump();
        };
    }

    private void OnHostLine(string line)
    {
        if (!_hostLive)
            return;
        Dispatcher.BeginInvoke(() =>
        {
            if (PauseBox.IsChecked == true)
            {
                if (!string.IsNullOrEmpty(line))
                    _lines.Add(("host", line));
                Trim();
                return;
            }
            Append("host", [line]);
        });
    }

    private void Pump()
    {
        if (!_hostLive)
            Append("host", _hostTail.ReadNew());
        Append("mixer", _mixerTail.ReadNew());
    }

    private void Append(string source, IReadOnlyList<string> incoming)
    {
        if (incoming.Count == 0)
            return;
        var show = source == "host" ? HostBox.IsChecked == true : MixerBox.IsChecked == true;
        var builder = show ? new StringBuilder() : null;
        foreach (var line in incoming)
        {
            if (string.IsNullOrEmpty(line))
                continue;
            _lines.Add((source, line));
            if (builder is not null)
                builder.Append(source).Append(' ').Append(line).Append('\n');
        }
        var overflow = _lines.Count > MaxLines;
        Trim();
        if (overflow)
        {
            Rebuild();
            return;
        }
        if (builder is null || builder.Length == 0)
            return;
        LogText.AppendText(builder.ToString());
        ScrollIfNeeded();
    }

    private void Rebuild()
    {
        var host = HostBox.IsChecked == true;
        var mixer = MixerBox.IsChecked == true;
        var builder = new StringBuilder();
        foreach (var (source, line) in _lines)
        {
            if (source == "host" && !host)
                continue;
            if (source == "mixer" && !mixer)
                continue;
            builder.Append(source).Append(' ').Append(line).Append('\n');
        }
        LogText.Text = builder.ToString();
        ScrollIfNeeded();
    }

    private void ScrollIfNeeded()
    {
        if (AutoScrollBox.IsChecked == true)
            LogText.ScrollToEnd();
    }

    private void Filter_Changed(object sender, RoutedEventArgs e)
    {
        if (IsLoaded)
            Rebuild();
    }

    private void Trim()
    {
        if (_lines.Count > MaxLines)
            _lines.RemoveRange(0, _lines.Count - MaxLines);
    }

    private void Pause_Unchecked(object sender, RoutedEventArgs e)
    {
        if (!IsLoaded)
            return;
        Pump();
        Rebuild();
    }

    private void Clear_Click(object sender, RoutedEventArgs e)
    {
        _lines.Clear();
        LogText.Clear();
    }

    private void Folder_Click(object sender, RoutedEventArgs e)
    {
        Directory.CreateDirectory(HostLog.DirectoryPath);
        Process.Start(new ProcessStartInfo
        {
            FileName = HostLog.DirectoryPath,
            UseShellExecute = true
        });
    }
}
