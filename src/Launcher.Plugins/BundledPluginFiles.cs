using System.Security.Cryptography;
using System.Text.Json;

namespace Launcher.Plugins;

internal static class BundledPluginFiles
{
    public static bool IsCurrent(string root, BundledPlugin plugin)
    {
        var directory = PackageFiles.Within(root, plugin.Id);
        if (!Directory.Exists(directory) || Directory.Exists(PackageFiles.Within(root, ".previous-" + plugin.Id))) return false;
        PackageFiles.CheckNoLinks(directory);
        return Directory.GetFiles(directory, "*", SearchOption.AllDirectories).Length == 2
            && Matches(Path.Combine(directory, "plugin.json"), JsonSerializer.SerializeToUtf8Bytes(plugin.Package, PluginPackage.Json))
            && Matches(PackageFiles.Within(directory, plugin.EntryAssembly), plugin.AssemblyBytes);
    }

    // Run before loading any assembly. A failed swap retains the previous complete directory;
    // the deterministic backup also recovers a process interrupted between the two renames.
    public static async Task SynchronizeAsync(string root, BundledPlugin plugin, CancellationToken token)
    {
        var destination = PackageFiles.Within(root, plugin.Id);
        var backup = PackageFiles.Within(root, ".previous-" + plugin.Id);
        if (Directory.Exists(backup))
        {
            PackageFiles.CheckNoLinks(backup);
            if (!Directory.Exists(destination)) Directory.Move(backup, destination);
            else PackageFiles.DeleteOwnedDirectory(root, backup);
        }
        var files = new Dictionary<string, byte[]>(StringComparer.Ordinal)
        {
            ["plugin.json"] = JsonSerializer.SerializeToUtf8Bytes(plugin.Package, PluginPackage.Json),
            [plugin.EntryAssembly] = plugin.AssemblyBytes,
        };
        if (IsCurrent(root, plugin)) return;
        var stage = PackageFiles.Within(root, ".install-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(stage);
        try
        {
            foreach (var (name, bytes) in files)
            {
                var path = PackageFiles.Within(stage, name);
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                await File.WriteAllBytesAsync(path, bytes, token).ConfigureAwait(false);
            }
            PluginPackage.Read(Path.Combine(stage, "plugin.json"));
            token.ThrowIfCancellationRequested();
            if (Directory.Exists(destination)) Directory.Move(destination, backup);
            try { Directory.Move(stage, destination); }
            catch
            {
                if (Directory.Exists(backup)) Directory.Move(backup, destination);
                throw;
            }
            PackageFiles.DeleteOwnedDirectory(root, backup);
        }
        finally { PackageFiles.DeleteOwnedDirectory(root, stage); }
    }

    private static bool Matches(string path, byte[] bytes)
    {
        if (!File.Exists(path) || new FileInfo(path).Length != bytes.Length) return false;
        using var input = File.OpenRead(path);
        return SHA256.HashData(input).AsSpan().SequenceEqual(SHA256.HashData(bytes));
    }
}
