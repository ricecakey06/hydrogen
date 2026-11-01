using System.IO.Compression;
using System.Text.Json;
using System.Text.RegularExpressions;
using Launcher.Contracts;
using Launcher.Plugins.Storage;

namespace Launcher.Plugins.Minecraft;

public static class LaunchPlanBuilder
{
    public static ProcessRequest Build(PreparedGame game, JavaRuntimeInfo java, GameAccountSession account,
        LauncherSettings settings, PlatformTarget target, string gameDirectory, string nativesDirectory)
    {
        var root = game.Metadata.Document;
        var classpath = string.Join(target.ClasspathSeparator, game.Classpath.Append(game.ClientPath));
        var values = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["auth_player_name"] = account.PlayerName, ["auth_uuid"] = account.Uuid, ["auth_access_token"] = account.AccessToken,
            ["auth_xuid"] = account.Xuid, ["clientid"] = account.ClientId, ["user_type"] = account.UserType, ["user_properties"] = "{}",
            ["version_name"] = game.Metadata.Id, ["version_type"] = root.GetProperty("type").GetString()!,
            ["game_directory"] = gameDirectory, ["assets_root"] = game.AssetsDirectory, ["game_assets"] = game.AssetsDirectory,
            ["assets_index_name"] = game.AssetIndex, ["natives_directory"] = nativesDirectory,
            ["launcher_name"] = "Hydrogen", ["launcher_version"] = "1.0", ["classpath"] = classpath,
            ["classpath_separator"] = target.ClasspathSeparator.ToString(),
            ["resolution_width"] = settings.Window.Width.ToString(System.Globalization.CultureInfo.InvariantCulture),
            ["resolution_height"] = settings.Window.Height.ToString(System.Globalization.CultureInfo.InvariantCulture),
        };
        var features = new Dictionary<string, bool> { ["has_custom_resolution"] = settings.Window.Mode == GameWindowMode.Custom };
        var args = new List<string>();
        var arguments = root.GetProperty("arguments");
        args.AddRange(Expand(arguments.GetProperty("jvm"), target, features, values));
        args.Add($"-Xmx{settings.MaxMemoryMiB}M");
        if (game.LoggingArgument is not null) args.Add(game.LoggingArgument);
        args.AddRange(SettingsValidation.ParseJvmArguments(settings.JvmArguments));
        var mainClass = root.GetProperty("mainClass").GetString()!;
        if (!Regex.IsMatch(mainClass, "^[a-zA-Z_$][a-zA-Z0-9_.$]+$")) throw new InvalidDataException("Invalid game entry point.");
        args.Add(mainClass);
        args.AddRange(Expand(arguments.GetProperty("game"), target, features, values));
        if (settings.Window.Mode == GameWindowMode.Fullscreen) args.Add("--fullscreen");
        return new ProcessRequest(java.ExecutablePath, args, gameDirectory);
    }

    private static IEnumerable<string> Expand(JsonElement arguments, PlatformTarget target, IReadOnlyDictionary<string, bool> features, Dictionary<string, string> values)
    {
        foreach (var entry in arguments.EnumerateArray())
        {
            if (entry.ValueKind == JsonValueKind.String) yield return Substitute(entry.GetString()!, values);
            else if (MetadataRules.Allows(entry, target, features))
            {
                var value = entry.GetProperty("value");
                if (value.ValueKind == JsonValueKind.String) yield return Substitute(value.GetString()!, values);
                else foreach (var part in value.EnumerateArray()) yield return Substitute(part.GetString()!, values);
            }
        }
    }

    private static string Substitute(string text, Dictionary<string, string> values) => Regex.Replace(text, @"\$\{([^}]+)\}", match =>
        values.GetValueOrDefault(match.Groups[1].Value) ?? throw new GameLaunchException(LaunchFailureKind.PlatformUnsupported, "此版本包含尚未支持的启动参数。"));

    public static async Task ExtractNativesAsync(IReadOnlyList<NativeArchive> archives, string destination, CancellationToken cancellationToken)
    {
        Directory.CreateDirectory(destination);
        long extracted = 0;
        foreach (var archive in archives)
        {
            using var zip = ZipFile.OpenRead(archive.Path);
            foreach (var entry in zip.Entries)
            {
                cancellationToken.ThrowIfCancellationRequested();
                var relative = entry.FullName.Replace('\\', '/');
                if (relative.EndsWith('/') || relative.StartsWith("META-INF/", StringComparison.OrdinalIgnoreCase)
                    || archive.Excludes.Any(prefix => relative.StartsWith(prefix, StringComparison.Ordinal))) continue;
                if ((entry.ExternalAttributes >> 16 & 0xF000) == 0xA000) throw new InvalidDataException("Linked native files are not supported.");
                extracted += entry.Length;
                if (extracted > 512L * 1024 * 1024) throw new InvalidDataException("Native files exceed the extraction limit.");
                var path = JsonFiles.Within(destination, relative.Replace('/', Path.DirectorySeparatorChar));
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                await using var input = entry.Open();
                await using var output = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.None, 81920, true);
                await input.CopyToAsync(output, cancellationToken).ConfigureAwait(false);
            }
        }
    }
}
