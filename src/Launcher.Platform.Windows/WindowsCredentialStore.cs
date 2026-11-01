using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.Security.Cryptography;
using Launcher.Contracts;

namespace Launcher.Platform.Windows;

/// <summary>DPAPI with the current user's key and UI disabled; no machine-wide or plain-text fallback.</summary>
[SupportedOSPlatform("windows")]
public sealed class WindowsCredentialStore(ILauncherStorage storage) : ICredentialStore
{
    private readonly SemaphoreSlim _gate = new(1, 1);
    public bool IsPersistent => true;

    public async Task<byte[]?> ReadAsync(CancellationToken cancellationToken)
    {
        using var usage = storage.AcquireUsage();
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var path = storage.Resolve("credentials.bin");
            if (!File.Exists(path)) return null;
            if (new FileInfo(path).Length > 1024 * 1024) throw new CryptographicException("Invalid encrypted credentials.");
            return Transform(await File.ReadAllBytesAsync(path, cancellationToken).ConfigureAwait(false), protect: false);
        }
        finally { _gate.Release(); }
    }

    public async Task SaveAsync(byte[] credentials, CancellationToken cancellationToken)
    {
        using var usage = storage.AcquireUsage();
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var path = storage.Resolve("credentials.bin");
            var encrypted = Transform(credentials, protect: true);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            var temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
            try
            {
                await File.WriteAllBytesAsync(temporary, encrypted, cancellationToken).ConfigureAwait(false);
                cancellationToken.ThrowIfCancellationRequested();
                File.Move(temporary, path, overwrite: true);
            }
            finally { if (File.Exists(temporary)) File.Delete(temporary); }
        }
        finally { _gate.Release(); }
    }

    public async Task DeleteAsync(CancellationToken cancellationToken)
    {
        using var usage = storage.AcquireUsage();
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try { File.Delete(storage.Resolve("credentials.bin")); }
        finally { _gate.Release(); }
    }

    private static byte[] Transform(byte[] value, bool protect)
    {
        var input = new DataBlob { Length = value.Length, Data = Marshal.AllocHGlobal(value.Length) };
        var output = new DataBlob();
        try
        {
            Marshal.Copy(value, 0, input.Data, value.Length);
            var success = protect
                ? CryptProtectData(ref input, null, 0, 0, 0, 1, out output)
                : CryptUnprotectData(ref input, 0, 0, 0, 0, 1, out output);
            if (!success) throw new CryptographicException("The current user cannot protect or read these credentials.");
            var result = new byte[output.Length];
            Marshal.Copy(output.Data, result, 0, result.Length);
            return result;
        }
        finally
        {
            Marshal.Copy(new byte[value.Length], 0, input.Data, value.Length);
            Marshal.FreeHGlobal(input.Data);
            if (output.Data != 0)
            {
                Marshal.Copy(new byte[output.Length], 0, output.Data, output.Length);
                _ = LocalFree(output.Data);
            }
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct DataBlob { public int Length; public nint Data; }
    [DllImport("crypt32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CryptProtectData(ref DataBlob input, string? description, nint entropy, nint reserved, nint prompt, uint flags, out DataBlob output);
    [DllImport("crypt32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CryptUnprotectData(ref DataBlob input, nint description, nint entropy, nint reserved, nint prompt, uint flags, out DataBlob output);
    [DllImport("kernel32.dll")]
    private static extern nint LocalFree(nint memory);
}
