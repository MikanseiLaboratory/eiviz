using System.Runtime.InteropServices;

namespace Eiviz.Host.Media;

internal static class MfFramePump
{
    public static List<(string Name, string Link)> EnumerateCameras()
    {
        var list = new List<(string, string)>();
        Native.EnsureStarted();
        if (Native.MFCreateAttributes(out var attrs, 1) != 0 || attrs == nint.Zero)
            return list;
        try
        {
            var hr = Native.IMFAttributesSetGUID(
                attrs,
                Native.MfDevsourceAttributeSourceType,
                Native.MfDevsourceAttributeSourceTypeVidcap);
            if (hr != 0)
                return list;
            hr = Native.MFEnumDeviceSources(attrs, out var devices, out var count);
            if (hr != 0 || devices == nint.Zero)
                return list;
            try
            {
                for (var i = 0; i < count; i++)
                {
                    var activate = Marshal.ReadIntPtr(devices, i * nint.Size);
                    if (activate == nint.Zero)
                        continue;
                    var name = Native.GetString(activate, Native.MfDevsourceAttributeFriendlyName);
                    var link = Native.GetString(activate, Native.MfDevsourceAttributeSourceTypeVidcapSymbolic);
                    if (!string.IsNullOrEmpty(name) && !string.IsNullOrEmpty(link))
                        list.Add((name, link));
                    Marshal.Release(activate);
                }
            }
            finally
            {
                Marshal.FreeCoTaskMem(devices);
            }
        }
        finally
        {
            Marshal.Release(attrs);
        }
        return list;
    }

    private static class Native
    {
        private static int _startup;

        internal static readonly Guid MfDevsourceAttributeSourceType = new("c60ac5fe-252a-478f-a0ef-bc8fa5f67663");
        internal static readonly Guid MfDevsourceAttributeSourceTypeVidcap = new("8ac3587a-4ae7-42d8-99e0-0a6013eef90f");
        internal static readonly Guid MfDevsourceAttributeFriendlyName = new("60d0e559-52f8-4fa2-bbce-acdb34a8ec01");
        internal static readonly Guid MfDevsourceAttributeSourceTypeVidcapSymbolic = new("58f0aad8-22bf-4f8a-bb3d-d2c4978c6e2f");

        internal static void EnsureStarted()
        {
            if (Interlocked.CompareExchange(ref _startup, 1, 0) == 0)
            {
                var hr = MFStartup(0x00020070, 1);
                if (hr != 0)
                    throw new InvalidOperationException($"MFStartup failed ({hr:X8}).");
            }
        }

        internal static string GetString(nint attrs, Guid key)
        {
            var buffer = new char[512];
            var hr = IMFAttributesGetString(attrs, key, buffer, buffer.Length, out var length);
            if (hr != 0 || length <= 0)
                return string.Empty;
            return new string(buffer, 0, length);
        }

        [DllImport("mfplat.dll")] private static extern int MFStartup(int version, int flags);
        [DllImport("mfplat.dll")] internal static extern int MFCreateAttributes(out nint attrs, uint size);
        [DllImport("mf.dll")] internal static extern int MFEnumDeviceSources(nint attrs, out nint devices, out uint count);

        internal static int IMFAttributesSetGUID(nint attrs, Guid key, Guid value)
        {
            var vtable = Marshal.ReadIntPtr(attrs);
            var fn = Marshal.ReadIntPtr(vtable, 24 * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<SetGuidDlg>(fn);
            return dlg(attrs, ref key, ref value);
        }

        internal static int IMFAttributesGetString(nint attrs, Guid key, char[] buffer, int cap, out int length)
        {
            length = 0;
            var vtable = Marshal.ReadIntPtr(attrs);
            var fn = Marshal.ReadIntPtr(vtable, 12 * nint.Size);
            var dlg = Marshal.GetDelegateForFunctionPointer<GetStringDlg>(fn);
            return dlg(attrs, ref key, buffer, cap, out length);
        }

        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int SetGuidDlg(nint obj, ref Guid key, ref Guid value);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        private delegate int GetStringDlg(nint obj, ref Guid key, [MarshalAs(UnmanagedType.LPArray)] char[] buffer, int cap, out int length);
    }
}
