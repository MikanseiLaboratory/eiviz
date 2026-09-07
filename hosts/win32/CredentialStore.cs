using System.Runtime.InteropServices;
using System.Text;

namespace Eiviz.Host;

internal static class CredentialStore
{
    private const uint CredTypeGeneric = 1;
    private const uint CredPersistLocalMachine = 2;

    public static void Save(string endpoint, string token)
    {
        var target = Target(endpoint);
        if (string.IsNullOrEmpty(token))
        {
            CredDelete(target, CredTypeGeneric, 0);
            return;
        }
        var blob = Encoding.UTF8.GetBytes(token);
        var cred = new NativeCredential
        {
            Type = CredTypeGeneric,
            TargetName = target,
            CredentialBlobSize = (uint)blob.Length,
            CredentialBlob = Marshal.AllocHGlobal(blob.Length),
            Persist = CredPersistLocalMachine,
            UserName = "eiviz"
        };
        try
        {
            Marshal.Copy(blob, 0, cred.CredentialBlob, blob.Length);
            if (!CredWrite(ref cred, 0))
                throw new InvalidOperationException($"Credential write failed ({Marshal.GetLastWin32Error()}).");
        }
        finally
        {
            Marshal.FreeHGlobal(cred.CredentialBlob);
        }
    }

    public static string Load(string endpoint)
    {
        if (!CredRead(Target(endpoint), CredTypeGeneric, 0, out var ptr) || ptr == nint.Zero)
            return "";
        try
        {
            var cred = Marshal.PtrToStructure<NativeCredential>(ptr);
            if (cred.CredentialBlob == nint.Zero || cred.CredentialBlobSize == 0)
                return "";
            var blob = new byte[cred.CredentialBlobSize];
            Marshal.Copy(cred.CredentialBlob, blob, 0, blob.Length);
            return Encoding.UTF8.GetString(blob);
        }
        finally
        {
            CredFree(ptr);
        }
    }

    private static string Target(string endpoint) => "eiviz/api/" + endpoint.Trim();

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredWrite(ref NativeCredential credential, uint flags);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredRead(string target, uint type, uint flags, out nint credential);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredDelete(string target, uint type, uint flags);

    [DllImport("advapi32.dll")]
    private static extern void CredFree(nint buffer);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NativeCredential
    {
        public uint Flags;
        public uint Type;
        public string TargetName;
        public string? Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public nint CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public nint Attributes;
        public string? TargetAlias;
        public string? UserName;
    }
}
