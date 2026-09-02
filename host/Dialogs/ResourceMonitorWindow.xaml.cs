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
        var buffer = new SourceUsage[128];
        unsafe
        {
            fixed (SourceUsage* ptr = buffer)
            {
                var n = MixerNative.CopySourceUsage(ptr, (uint)buffer.Length);
                for (var i = 0; i < n && i < buffer.Length; i++)
                    usages[buffer[i].SourceId] = buffer[i];
            }
        }

        MixerStats stats = default;
        unsafe
        {
            MixerNative.CopyStats(&stats);
        }
        ulong totalRam = stats.RamBytes;
        ulong totalVram = stats.VramBytes;
        if (totalRam == 0 && totalVram == 0)
        {
            foreach (var usage in usages.Values)
            {
                totalRam += usage.RamBytes;
                totalVram += usage.VramBytes;
            }
        }
        if (totalRam == 0) totalRam = 1;
        if (totalVram == 0) totalVram = 1;
        var gpuLoad = GpuUtilization.Percent();

        var rows = new List<Row>();
        foreach (var input in session.Inputs)
        {
            usages.TryGetValue(input.Id, out var usage);
            var ram = usage.RamBytes;
            var vram = usage.VramBytes;
            var cpu = input.Kind is InputKind.Omt or InputKind.Ndi or InputKind.Uvc or InputKind.Video ? "live" : "—";
            rows.Add(new Row(
                input.Name,
                input.Kind.ToString(),
                usage.Width == 0 ? "—" : $"{usage.Width}x{usage.Height}",
                cpu,
                "—",
                FormatBytes(ram),
                FormatBytes(vram)));
        }
        foreach (var scene in session.Scenes)
        {
            usages.TryGetValue(scene.GpuId, out var usage);
            rows.Add(new Row(
                scene.Name,
                "Scene",
                usage.Width == 0 ? "—" : $"{usage.Width}x{usage.Height}",
                "—",
                usage.GpuPct > 0 ? $"{usage.GpuPct:0}%" : "—",
                "—",
                FormatBytes(usage.VramBytes)));
        }
        UsageList.ItemsSource = rows;
        var ramText = FormatBytes(totalRam == 1 ? 0 : totalRam);
        var vramText = FormatBytes(totalVram == 1 ? 0 : totalVram);
        var extra = stats.ComposeVramBytes > 0 || stats.DelayVramBytes > 0
            ? $"    Compose {FormatBytes(stats.ComposeVramBytes)}    Delay {FormatBytes(stats.DelayVramBytes)}"
            : "";
        SummaryText.Text = $"Inputs {session.Inputs.Count}    GPU {gpuLoad:0}%    RAM {ramText}    VRAM {vramText}{extra}    Render {stats.RenderMs:0.0} / {stats.FrameBudgetMs:0.0} ms";
    }

    private static string FormatBytes(ulong bytes)
    {
        if (bytes == 0)
            return "—";
        if (bytes < 1024)
            return $"{bytes} B";
        if (bytes < 1024 * 1024)
            return $"{bytes / 1024.0:0.0} KB";
        if (bytes < 1024UL * 1024 * 1024)
            return $"{bytes / (1024.0 * 1024.0):0.0} MB";
        return $"{bytes / (1024.0 * 1024.0 * 1024.0):0.00} GB";
    }

    private sealed record Row(string Name, string Kind, string Size, string Cpu, string Gpu, string Ram, string Vram);
}
