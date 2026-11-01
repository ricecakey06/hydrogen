using System.Text.Json;
using System.Text.RegularExpressions;
using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Plugins;

/// <summary>Trusted release metadata and assembly bytes, embedded by the host rather than supplied by a plugin.</summary>
public sealed record BundledPlugin(string Id, string Name, string Description, string EntryAssembly,
    string EntryType, byte[] AssemblyBytes, string Version = "0.1.0", string Author = "Hydrogen", bool EnabledByDefault = true)
{
    internal PluginPackage Package => new()
    {
        Id = Id, Name = Name, Description = Description, Version = Version, Author = Author,
        EntryAssembly = EntryAssembly, EntryType = EntryType,
    };
}

/// <summary>Metadata can be inspected and installed without loading or executing the package's assembly.</summary>
internal sealed record PluginPackage
{
    public int SchemaVersion { get; init; } = 1;
    public int ApiVersion { get; init; } = 1;
    public string Id { get; init; } = "";
    public string Name { get; init; } = "";
    public string Version { get; init; } = "";
    public string Description { get; init; } = "";
    public string Author { get; init; } = "";
    public string EntryAssembly { get; init; } = "";
    public string EntryType { get; init; } = "";
    public string[] Dependencies { get; init; } = [];

    internal static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web) { WriteIndented = true };

    internal static bool IsId(string id) => Regex.IsMatch(id, "^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$", RegexOptions.CultureInvariant)
        && id.Length <= 100;

    public static PluginPackage Read(string path)
    {
        if (new FileInfo(path).Length > 65536) throw new PluginManagementException("插件清单超过 64 KB。 ");
        try
        {
            var package = JsonSerializer.Deserialize<PluginPackage>(File.ReadAllText(path), Json)
                ?? throw new PluginManagementException("插件清单为空。 ");
            package.Validate();
            return package;
        }
        catch (JsonException) { throw new PluginManagementException("plugin.json 不是有效的 JSON 插件清单。 "); }
    }

    private void Validate()
    {
        if (SchemaVersion != 1 || ApiVersion != 1) throw new PluginManagementException("此插件的清单或 API 版本与启动器不兼容。 ");
        if (Id is null || !IsId(Id)) throw new PluginManagementException("插件 ID 必须是小写字母、数字和点号等组成的稳定标识。 ");
        if (string.IsNullOrWhiteSpace(Name) || Name.Length > 120 || string.IsNullOrWhiteSpace(Version)
            || !Regex.IsMatch(Version, @"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.+-]+)?$", RegexOptions.CultureInvariant)
            || Description is null || Description.Length > 2000 || Author is null || Author.Length > 120)
            throw new PluginManagementException("插件名称、版本、作者或描述无效。 ");
        if (string.IsNullOrWhiteSpace(EntryType) || EntryType.Length > 300 || string.IsNullOrWhiteSpace(EntryAssembly)
            || !EntryAssembly.EndsWith(".dll", StringComparison.OrdinalIgnoreCase))
            throw new PluginManagementException("插件必须声明有效的 entryAssembly 和 entryType。 ");
        PackageFiles.ValidateRelativePath(EntryAssembly);
        if (Dependencies is null || Dependencies.Length > 64 || Dependencies.Any(id => id is null || !IsId(id) || id == Id)
            || Dependencies.Distinct(StringComparer.Ordinal).Count() != Dependencies.Length)
            throw new PluginManagementException("插件依赖清单无效。 ");
    }
}
