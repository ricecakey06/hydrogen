using System.IO.Compression;
using System.Text.RegularExpressions;
using Launcher.Contracts;

namespace Launcher.Plugins;

internal static class PackageFiles
{
    private const long MaximumSize = 256L * 1024 * 1024;

    public static void ValidateRelativePath(string path)
    {
        var segments = path.Replace('\\', '/').Split('/');
        if (Path.IsPathRooted(path) || segments.Any(segment => segment.Length == 0 || segment is "." or ".."
            || segment.EndsWith('.') || segment.EndsWith(' ') || segment.Any(c => c < 32 || "<>:\"|?*".Contains(c))
            || Regex.IsMatch(segment, @"^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)))
            throw new PluginManagementException("插件包包含不安全或不支持的文件路径。 ");
    }

    public static string Within(string root, string relative)
    {
        ValidateRelativePath(relative);
        var basePath = Path.GetFullPath(root).TrimEnd(Path.DirectorySeparatorChar) + Path.DirectorySeparatorChar;
        var path = Path.GetFullPath(Path.Combine(basePath, relative.Replace('/', Path.DirectorySeparatorChar).Replace('\\', Path.DirectorySeparatorChar)));
        if (!path.StartsWith(basePath, OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal))
            throw new PluginManagementException("插件文件不能位于插件目录之外。 ");
        return path;
    }

    public static void CheckNoLinks(string path)
    {
        if ((File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0)
            throw new PluginManagementException("插件目录和文件不能使用符号链接或目录联接。 ");
        if (!Directory.Exists(path)) return;
        foreach (var child in Directory.EnumerateFileSystemEntries(path)) CheckNoLinks(child);
    }

    public static void DeleteOwnedDirectory(string root, string directory)
    {
        var path = Within(root, Path.GetFileName(directory));
        if (!string.Equals(path, Path.GetFullPath(directory), OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal))
            throw new PluginManagementException("无法清理插件目录。 ");
        if (!Directory.Exists(path)) return;
        CheckNoLinks(path);
        Directory.Delete(path, recursive: true);
    }

    public static async Task ExtractAsync(string archivePath, string destination, CancellationToken token)
    {
        if (new FileInfo(archivePath).Length > MaximumSize) throw new PluginManagementException("插件包不能超过 256 MB。 ");
        using var archive = ZipFile.OpenRead(archivePath);
        long declaredSize = 0;
        foreach (var entry in archive.Entries)
        {
            if (entry.Length < 0 || entry.Length > MaximumSize - declaredSize)
                throw new PluginManagementException("插件包解压后超过 256 MB。 ");
            declaredSize += entry.Length;
        }
        if (archive.Entries.Count > 4096)
            throw new PluginManagementException("插件包解压后超过大小或文件数量限制。 ");
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        long writtenSize = 0;
        var buffer = new byte[81920];
        foreach (var entry in archive.Entries)
        {
            token.ThrowIfCancellationRequested();
            var relative = entry.FullName.Replace('\\', '/').TrimEnd('/');
            ValidateRelativePath(relative);
            if (!seen.Add(relative) || ((entry.ExternalAttributes >> 16) & 0xF000) == 0xA000
                || (entry.ExternalAttributes & (int)FileAttributes.ReparsePoint) != 0)
                throw new PluginManagementException("插件包包含重复文件、符号链接或目录联接。 ");
            var path = Within(destination, relative);
            if (entry.FullName.EndsWith('/') || entry.FullName.EndsWith('\\'))
            {
                Directory.CreateDirectory(path);
                continue;
            }
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            await using var output = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None, 81920, useAsync: true);
            await using var input = entry.Open();
            int read;
            while ((read = await input.ReadAsync(buffer, token).ConfigureAwait(false)) > 0)
            {
                writtenSize += read;
                if (writtenSize > MaximumSize) throw new PluginManagementException("插件包解压后超过 256 MB。 ");
                await output.WriteAsync(buffer.AsMemory(0, read), token).ConfigureAwait(false);
            }
        }
        if (!File.Exists(Path.Combine(destination, "plugin.json")))
            throw new PluginManagementException("插件包根目录缺少 plugin.json。请选择 Hydrogen 插件包。 ");
    }
}
