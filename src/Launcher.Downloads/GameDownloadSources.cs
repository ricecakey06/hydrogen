using Launcher.Contracts;

namespace Launcher.Downloads;

public static class GameDownloadSources
{
    public static Uri BmclapiBaseUri { get; } = new("https://bmclapi2.bangbang93.com/");

    public static Uri ObjectEndpoint(Uri uri)
    {
        ArgumentNullException.ThrowIfNull(uri);
        if (!uri.IsAbsoluteUri) throw new InvalidDataException("An absolute game object URL is required.");
        var parts = uri.AbsolutePath.Split('/', StringSplitOptions.RemoveEmptyEntries);
        if (uri.Scheme != "https" || !uri.IsDefaultPort || uri.UserInfo.Length != 0 || uri.Host != "piston-data.mojang.com"
            || parts.Length != 4 || parts[0] != "v1" || parts[1] != "objects" || parts[2].Length != 40 || !parts[2].All(Uri.IsHexDigit)) return uri;
        return new UriBuilder(uri) { Host = "launcher.mojang.com" }.Uri;
    }

    public static Uri Resolve(Uri original, GameDownloadSource source)
    {
        ArgumentNullException.ThrowIfNull(original);
        if (!original.IsAbsoluteUri) throw new InvalidDataException("An absolute content URL is required.");
        if (!Enum.IsDefined(source)) throw new InvalidDataException("Unknown game download source.");
        if (source == GameDownloadSource.Official || !original.IsDefaultPort || original.UserInfo.Length != 0
            || original.Scheme is not ("http" or "https")) return original;
        var prefix = original.Host.ToLowerInvariant() switch
        {
            "resources.download.minecraft.net" => "/assets",
            "libraries.minecraft.net" => "/maven",
            "launchermeta.mojang.com" or "launcher.mojang.com" or "piston-meta.mojang.com" or "piston-data.mojang.com" => "",
            _ => null,
        };
        return prefix is null ? original : new UriBuilder(BmclapiBaseUri) { Path = prefix + original.AbsolutePath, Query = original.Query }.Uri;
    }
}
