using System.Windows;
using System.Windows.Threading;
using Eiviz.Host.Interop;

namespace Eiviz.Host.Dialogs;

public partial class ResourceMonitorWindow : Window
{
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(400) };

    public ResourceMonitorWindow()
    {
        InitializeComponent();
        _timer.Tick += (_, _) => Refresh();
        Loaded += (_, _) =>
        {
            Refresh();
            _timer.Start();
        };
        Closed += (_, _) => _timer.Stop();
    }

    private void Refresh()
    {
        var app = (App)Application.Current;
        var session = app.Session;
        var usages = new Dictionary<ulong, SourceUsage>();
        var buffer = new SourceUsage[64];
        unsafe
        {
            fixed (SourceUsage* ptr = buffer)
            {
                var n = MixerNative.CopySourceUsage(ptr, (uint)buffer.Length);
                for (var i = 0; i < n && i < buffer.Length; i++)
                    usages[buffer[i].SourceId] = buffer[i];
            }
        }

        ulong totalRam = 0, totalVram = 0;
        foreach (var usage in usages.Values)
        {
            totalRam += usage.RamBytes;
            totalVram += usage.VramBytes;
        }
        if (totalRam == 0) totalRam = 1;
        if (totalVram == 0) totalVram = 1;

        MixerStats stats = default;
        unsafe
        {
            MixerNative.CopyStats(&stats);
        }
        var gpuLoad = stats.FrameBudgetMs > 0.1f
            ? Math.Min(100f, stats.RenderMs / stats.FrameBudgetMs * 100f)
            : 0f;

        var rows = new List<Row>();
        foreach (var input in session.Inputs)
        {
            usages.TryGetValue(input.Id, out var usage);
            var ram = usage.RamBytes;
            var vram = usage.VramBytes;
            var video = app.Commands.TryGetVideo(input.Id);
            var cpu = video is { IsFile: true }
                ? $"{video.CpuShareMs:0.0} ms"
                : input.Kind is InputKind.Omt or InputKind.Uvc or InputKind.Video ? "live" : "—";
            var gpu = vram == 0 ? "—" : $"{vram / (double)totalVram * gpuLoad:0}%";
            rows.Add(new Row(
                input.Name,
                input.Kind.ToString(),
                usage.Width == 0 ? "—" : $"{usage.Width}x{usage.Height}",
                cpu,
                gpu,
                FormatBytes(ram),
                FormatBytes(vram)));
        }
        UsageList.ItemsSource = rows;
        SummaryText.Text = $"Inputs {session.Inputs.Count}    RAM {FormatBytes(totalRam)}    VRAM {FormatBytes(totalVram)}    Render {stats.RenderMs:0.0} / {stats.FrameBudgetMs:0.0} ms";
    }

    private static string FormatBytes(ulong bytes)
    {
        if (bytes == 0)
            return "—";
        if (bytes < 1024)
            return $"{bytes} B";
        if (bytes < 1024 * 1024)
            return $"{bytes / 1024.0:0.0} KB";
        return $"{bytes / (1024.0 * 1024.0):0.0} MB";
    }

    private sealed record Row(string Name, string Kind, string Size, string Cpu, string Gpu, string Ram, string Vram);
}
