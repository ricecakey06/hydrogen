using System.Text.Json;
using System.Text.RegularExpressions;
using Launcher.Contracts;

namespace Launcher.Plugins.Minecraft;

public static class MetadataRules
{
    public static bool Allows(JsonElement owner, PlatformTarget target, IReadOnlyDictionary<string, bool>? features = null)
    {
        if (!owner.TryGetProperty("rules", out var rules)) return true;
        var allowed = false;
        foreach (var rule in rules.EnumerateArray())
        {
            var matches = true;
            if (rule.TryGetProperty("os", out var os))
            {
                if (os.TryGetProperty("name", out var name)) matches &= name.GetString() == target.OperatingSystem;
                if (os.TryGetProperty("arch", out var arch))
                {
                    var javaArch = target.Architecture switch { "x64" => "x86_64", "arm64" => "aarch64", _ => target.Architecture };
                    matches &= Match(javaArch, "^(?:" + arch.GetString() + ")$");
                }
                if (os.TryGetProperty("version", out var version)) matches &= Match(target.Version, version.GetString()!);
                if (os.TryGetProperty("versionRange", out var range))
                {
                    if (!Version.TryParse(target.Version, out var actual)) matches = false;
                    else
                    {
                        if (range.TryGetProperty("min", out var min)) matches &= actual >= Version.Parse(min.GetString()!);
                        if (range.TryGetProperty("max", out var max)) matches &= actual < Version.Parse(max.GetString()!);
                    }
                }
            }
            if (rule.TryGetProperty("features", out var conditions))
                foreach (var feature in conditions.EnumerateObject())
                    matches &= (features?.GetValueOrDefault(feature.Name) ?? false) == feature.Value.GetBoolean();
            if (matches) allowed = rule.GetProperty("action").GetString() switch
            { "allow" => true, "disallow" => false, _ => throw new InvalidDataException("Unknown metadata rule.") };
        }
        return allowed;
    }

    private static bool Match(string value, string expression)
    {
        try { return Regex.IsMatch(value, expression, RegexOptions.CultureInvariant, TimeSpan.FromMilliseconds(100)); }
        catch (Exception error) when (error is ArgumentException or RegexMatchTimeoutException)
        { throw new InvalidDataException("Unsupported metadata rule."); }
    }
}
