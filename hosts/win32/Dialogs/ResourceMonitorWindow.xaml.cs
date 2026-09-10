using System.Windows;
using System.Windows.Threading;
using Eiviz.Host.I18n;
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
        MixerRuntimeStats runtime = default;
        var inputs = new Dictionary<ulong, InputRuntimeStats>();
        var outputs = new List<OutputRuntimeStats>();
        unsafe
        {
            MixerNative.CopyStats(&stats);
            MixerNative.CopyRuntimeStats(&runtime);
            var inputBuf = new InputRuntimeStats[128];
            fixed (InputRuntimeStats* ptr = inputBuf)
            {
                var n = MixerNative.CopyInputStats(ptr, (uint)inputBuf.Length);
                for (var i = 0; i < n && i < inputBuf.Length; i++)
                    inputs[inputBuf[i].SourceId] = inputBuf[i];
            }
            var outputBuf = new OutputRuntimeStats[64];
            fixed (OutputRuntimeStats* ptr = outputBuf)
            {
                var n = MixerNative.CopyOutputStats(ptr, (uint)outputBuf.Length);
                for (var i = 0; i < n && i < outputBuf.Length; i++)
                    outputs.Add(outputBuf[i]);
            }
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
        var gpuLoad = GpuUtilization.Last();
        var gpuText = gpuLoad is { } gpu ? $"{gpu:0}%" : Loc.T("resources.unmeasured");

        var rows = new List<Row>();
        foreach (var input in session.Inputs)
        {
            usages.TryGetValue(input.Id, out var usage);
            inputs.TryGetValue(input.Id, out var live);
            var ram = usage.RamBytes;
            var vram = usage.VramBytes;
            var cpu = input.Kind is InputKind.OMT or InputKind.NDI or InputKind.UVC or InputKind.Video ? "live" : "—";
            rows.Add(new Row(
                input.ListLabel,
                input.Kind.ToString(),
                usage.Width == 0 ? "—" : $"{usage.Width}x{usage.Height}",
                FormatUptime(live.UptimeMs),
                live.QueueDropped == 0 ? "—" : live.QueueDropped.ToString(),
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
                "—",
                "—",
                usage.GpuPct > 0 ? $"{usage.GpuPct:0}%" : "—",
                "—",
                FormatBytes(usage.VramBytes)));
        }
        foreach (var output in session.Outputs)
        {
            var live = outputs.FirstOrDefault(item => item.OutputId == output.Id);
            var kind = output.Transport.ToString();
            rows.Add(new Row(
                output.Name,
                kind,
                output.Width == 0 ? "—" : $"{output.Width}x{output.Height}",
                live.OutputId == 0 ? "—" : FormatUptime(live.UptimeMs),
                live.Connections == 0 ? "—" : live.Connections.ToString(),
                output.Enabled ? "on" : "off",
                "—",
                "—",
                "—"));
        }
        UsageList.ItemsSource = rows;
        var ramText = FormatBytes(totalRam == 1 ? 0 : totalRam);
        var vramText = FormatBytes(totalVram == 1 ? 0 : totalVram);
        var extra = stats.ComposeVramBytes > 0 || stats.DelayVramBytes > 0
            ? $"    Compose {FormatBytes(stats.ComposeVramBytes)}    Delay {FormatBytes(stats.DelayVramBytes)}"
            : "";
        SummaryText.Text =
            $"Uptime {FormatUptime(runtime.UptimeMs)}    Skipped {runtime.RenderSkipped}    Lost {runtime.InputQueueDropped}    " +
            $"OMT {runtime.OutputOmt} (sub {runtime.OutputOmtSubscribed})    NDI {runtime.OutputNdi} (conn {runtime.OutputNdiConnections})    " +
            $"Inputs {session.Inputs.Count}    GPU {gpuText}    RAM {ramText}    VRAM {vramText}{extra}    " +
            $"Render {stats.RenderMs:0.0} / {stats.FrameBudgetMs:0.0} ms    " +
            "VRAM is max(DXGI process memory, tracked textures)";
    }

    private static string FormatUptime(ulong ms)
    {
        if (ms == 0)
            return "—";
        var span = TimeSpan.FromMilliseconds(ms);
        if (span.TotalHours >= 1)
            return $"{(int)span.TotalHours:00}:{span.Minutes:00}:{span.Seconds:00}";
        return $"{span.Minutes:00}:{span.Seconds:00}";
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

    private sealed record Row(string Name, string Kind, string Size, string Uptime, string Lost, string Cpu, string Gpu, string Ram, string Vram);
}
