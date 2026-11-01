using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Downloads;

public static class DownloadPlugins
{
    public static PluginDefinition GameDownload { get; } = new("minecraft.download", async (context, token) =>
    {
        var provider = new GameDownloadProvider(context.GetRequiredService<INetworkService>(), context.GetRequiredService<ILauncherSettingsService>());
        context.Own(await context.GetRequiredService<IGameDownloadRegistry>().RegisterAsync(provider, token).ConfigureAwait(false));
        context.Provide<IGameDownloadProvider>(provider);
    }, requires: [typeof(INetworkService), typeof(IGameDownloadRegistry), typeof(ILauncherSettingsService)], provides: [typeof(IGameDownloadProvider)]);
}
