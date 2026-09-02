using System.Diagnostics;
using System.Runtime.InteropServices;
using Eiviz.Host.Interop;

namespace Eiviz.Host;

internal sealed class ResourceMonitor : IDisposable
{
    private readonly Process _process = Process.GetCurrentProcess();
    private TimeSpan _lastCpu;
    private DateTime _lastSample = DateTime.UtcNow;
    private float _cpu;
    private float _ram;
    private float _gpu;
    private float _vram;
    private float _renderMs;
    private float _budgetMs = 16.67f;

    public void Sample()
    {
        var now = DateTime.UtcNow;
        _process.Refresh();
        var cpu = _process.TotalProcessorTime;
        var elapsed = (now - _lastSample).TotalMilliseconds;
        if (elapsed > 1)
        {
            _cpu = (float)((cpu - _lastCpu).TotalMilliseconds / (elapsed * Environment.ProcessorCount) * 100.0);
            _lastCpu = cpu;
            _lastSample = now;
        }
        _ram = SampleRam(_process.WorkingSet64);
        MixerStats stats = default;
        unsafe
        {
            if (MixerNative.CopyStats(&stats) == 0)
            {
                _renderMs = stats.RenderMs;
                _budgetMs = stats.FrameBudgetMs > 0.1f ? stats.FrameBudgetMs : 16.67f;
            }
        }
        try
        {
            SampleVideoMemory(out _vram);
            _gpu = GpuUtilization.Percent();
        }
        catch
        {
            _vram = 0;
            _gpu = GpuUtilization.Percent();
        }
    }

    public string Line()
    {
        return $"CPU {_cpu:0}%   GPU {_gpu:0}%   RAM {_ram:0}%   VRAM {_vram:0}%   Render {_renderMs:0.0} ms / {_budgetMs:0.0} ms";
    }

    public string? Warning()
    {
        var hits = new List<string>();
        if (_cpu >= 85) hits.Add($"CPU {_cpu:0}%");
        if (_gpu >= 85) hits.Add($"GPU {_gpu:0}%");
        if (_ram >= 85) hits.Add($"RAM {_ram:0}%");
        if (_vram >= 85) hits.Add($"VRAM {_vram:0}%");
        if (_budgetMs > 0 && _renderMs >= _budgetMs * 0.85f)
            hits.Add($"Render {_renderMs:0.0} ms");
        return hits.Count == 0 ? null : "High load: " + string.Join("  ", hits);
    }

    public void Dispose() => _process.Dispose();

    private static float SampleRam(long workingSet)
    {
        var status = new MemoryStatusEx { Length = (uint)Marshal.SizeOf<MemoryStatusEx>() };
        if (!GlobalMemoryStatusEx(ref status) || status.TotalPhys == 0)
            return 0;
        return (float)(workingSet / (double)status.TotalPhys * 100.0);
    }

    private static void SampleVideoMemory(out float vramPercent)
    {
        vramPercent = 0;
        var iidFactory1 = new Guid("770aae78-f26f-4dba-a829-253c83d1b387");
        if (CreateDXGIFactory1(in iidFactory1, out var factory) != 0 || factory == nint.Zero)
            return;
        try
        {
            ulong bestBudget = 0;
            ulong bestUsage = 0;
            ulong bestDedicated = 0;
            for (uint i = 0; ; i++)
            {
                var hr = VTable.EnumAdapters1(factory, i, out var adapter);
                if (hr != 0 || adapter == nint.Zero)
                    break;
                try
                {
                    if (VTable.IsSoftwareAdapter(adapter))
                        continue;
                    VTable.TryGetDedicated(adapter, out var dedicated);
                    if (dedicated > bestDedicated)
                        bestDedicated = dedicated;
                    if (!VTable.TryQueryUsage(adapter, out var budget, out var usage) || budget == 0)
                        continue;
                    if (budget > bestBudget)
                    {
                        bestBudget = budget;
                        bestUsage = usage;
                    }
                }
                finally
                {
                    Marshal.Release(adapter);
                }
            }
            if (bestBudget > 0)
                vramPercent = (float)(bestUsage / (double)bestBudget * 100.0);
            else if (bestDedicated > 0)
                vramPercent = 0;
        }
        finally
        {
            Marshal.Release(factory);
        }
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GlobalMemoryStatusEx(ref MemoryStatusEx status);

    [DllImport("dxgi.dll")]
    private static extern int CreateDXGIFactory1(in Guid riid, out nint factory);

    [StructLayout(LayoutKind.Sequential)]
    private struct MemoryStatusEx
    {
        public uint Length;
        public uint MemoryLoad;
        public ulong TotalPhys;
        public ulong AvailPhys;
        public ulong TotalPageFile;
        public ulong AvailPageFile;
        public ulong TotalVirtual;
        public ulong AvailVirtual;
        public ulong AvailExtendedVirtual;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct DxgiMemoryInfo
    {
        public ulong Budget;
        public ulong CurrentUsage;
        public ulong AvailableForReservation;
        public ulong CurrentReservation;
    }

    private static class VTable
    {
        private static readonly Guid Adapter3 = new("645967a4-1392-4310-a798-8053ce3e93fd");

        internal static int EnumAdapters1(nint factory, uint index, out nint adapter)
        {
            var dlg = Marshal.GetDelegateForFunctionPointer<EnumAdapters1Dlg>(Slot(factory, 12));
            return dlg(factory, index, out adapter);
        }

        internal static bool IsSoftwareAdapter(nint adapter)
        {
            if (!TryGetDesc(adapter, out var vendor, out var description, out _))
                return false;
            if (vendor == 0x1414)
                return true;
            return description.Contains("Basic Render", StringComparison.OrdinalIgnoreCase)
                || description.Contains("Microsoft Basic", StringComparison.OrdinalIgnoreCase);
        }

        internal static bool TryGetDedicated(nint adapter, out ulong dedicated)
        {
            return TryGetDesc(adapter, out _, out _, out dedicated) && dedicated > 0;
        }

        internal static bool TryQueryUsage(nint adapter, out ulong budget, out ulong usage)
        {
            budget = 0;
            usage = 0;
            var iid = Adapter3;
            if (Marshal.QueryInterface(adapter, in iid, out var adapter3) != 0 || adapter3 == nint.Zero)
                return false;
            try
            {
                if (Query(adapter3, 0, 0, out var local) == 0 && local.Budget > 0)
                {
                    budget = local.Budget;
                    usage = local.CurrentUsage;
                    return true;
                }
                if (Query(adapter3, 0, 1, out var shared) == 0 && shared.Budget > 0)
                {
                    budget = shared.Budget;
                    usage = shared.CurrentUsage;
                    return true;
                }
                return false;
            }
            finally
            {
                Marshal.Release(adapter3);
            }
        }

        private static bool TryGetDesc(nint adapter, out uint vendorId, out string description, out ulong dedicated)
        {
            vendorId = 0;
            description = "";
            dedicated = 0;
            var buffer = new byte[320];
            var dlg = Marshal.GetDelegateForFunctionPointer<GetDescDlg>(Slot(adapter, 8));
            unsafe
            {
                fixed (byte* ptr = buffer)
                {
                    if (dlg(adapter, (nint)ptr) != 0)
                        return false;
                }
            }
            description = System.Text.Encoding.Unicode.GetString(buffer, 0, 256).TrimEnd('\0');
            vendorId = BitConverter.ToUInt32(buffer, 256);
            dedicated = BitConverter.ToUInt64(buffer, 272);
            return true;
        }

        private static int Query(nint adapter3, uint node, int group, out DxgiMemoryInfo info)
        {
            var dlg = Marshal.GetDelegateForFunctionPointer<QueryVramDlg>(Slot(adapter3, 14));
            return dlg(adapter3, node, group, out info);
        }

        private static nint Slot(nint obj, int index) =>
            Marshal.ReadIntPtr(Marshal.ReadIntPtr(obj), index * nint.Size);

        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int EnumAdapters1Dlg(nint factory, uint index, out nint adapter);

        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int GetDescDlg(nint adapter, nint desc);

        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int QueryVramDlg(nint adapter, uint node, int group, out DxgiMemoryInfo info);
    }
}
