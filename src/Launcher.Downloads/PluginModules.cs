using Launcher.Runtime;

namespace Launcher.Downloads;

public sealed class DownloadModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => DownloadPlugins.GameDownload;
}
