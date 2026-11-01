using Launcher.Runtime;

namespace Launcher.UI;

public sealed class ShellModule : IPluginModule { public PluginDefinition CreateDefinition() => UiPlugins.Shell; }
public sealed class LauncherModule : IPluginModule { public PluginDefinition CreateDefinition() => UiPlugins.LauncherUi; }
