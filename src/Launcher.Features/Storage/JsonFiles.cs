using System.Text.Json;

namespace Launcher.Plugins.Storage;

internal static class JsonFiles
{
    internal static readonly JsonSerializerOptions Options = new(JsonSerializerDefaults.Web) { WriteIndented = true };

    internal static string Within(string root, string relative)
    {
        if (string.IsNullOrWhiteSpace(relative) || Path.IsPathRooted(relative)) throw new IOException("Invalid relative path.");
        var prefix = Path.TrimEndingDirectorySeparator(Path.GetFullPath(root)) + Path.DirectorySeparatorChar;
        var path = Path.GetFullPath(Path.Combine(root, relative));
        var comparison = OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal;
        if (!path.StartsWith(prefix, comparison)) throw new IOException("The path leaves the data directory.");
        for (var current = path; current is not null && current.Length >= prefix.Length - 1; current = Path.GetDirectoryName(current))
            if ((File.Exists(current) || Directory.Exists(current)) && (File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                throw new IOException("Linked data paths are not supported.");
        return path;
    }

    internal static async Task<T?> ReadAsync<T>(string path, CancellationToken token)
    {
        if (!File.Exists(path)) return default;
        await using var input = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read, 4096, true);
        if (input.Length > 16 * 1024 * 1024) throw new InvalidDataException("Configuration is too large.");
        using var document = await JsonDocument.ParseAsync(input, cancellationToken: token).ConfigureAwait(false);
        if (document.RootElement.ValueKind != JsonValueKind.Object || !document.RootElement.TryGetProperty("schemaVersion", out var schema)
            || schema.ValueKind != JsonValueKind.Number || !schema.TryGetInt32(out var version) || version != 1)
            throw new InvalidDataException("Unsupported configuration schema.");
        return document.RootElement.Deserialize<T>(Options) ?? throw new InvalidDataException("Empty configuration.");
    }

    internal static async Task WriteAsync<T>(string path, T value, CancellationToken token, bool protectExisting = true)
    {
        // Recheck at commit time too: another process may have upgraded the format after activation.
        if (protectExisting && File.Exists(path)) _ = await ReadAsync<JsonElement>(path, token).ConfigureAwait(false);
        await WriteBytesAsync(path, JsonSerializer.SerializeToUtf8Bytes(value, Options), token).ConfigureAwait(false);
    }

    internal static async Task WriteBytesAsync(string path, byte[] value, CancellationToken token)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            await using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None, 4096,
                FileOptions.Asynchronous | FileOptions.WriteThrough))
            {
                await output.WriteAsync(value, token).ConfigureAwait(false);
                await output.FlushAsync(token).ConfigureAwait(false);
            }
            token.ThrowIfCancellationRequested();
            File.Move(temporary, path, overwrite: true);
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
}
