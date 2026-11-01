using System.Runtime.Versioning;
using Launcher.Plugins.Infrastructure;
using Launcher.Runtime;

namespace Launcher.Platform.Windows;

[SupportedOSPlatform("windows")]
public sealed class EnvironmentModule : IPluginModule { public PluginDefinition CreateDefinition() => WindowsPlugins.Environment(new PortableEnvironment()); }
[SupportedOSPlatform("windows")]
public sealed class CredentialsModule : IPluginModule { public PluginDefinition CreateDefinition() => WindowsPlugins.Credentials; }
[SupportedOSPlatform("windows")]
public sealed class WindowModule : IPluginModule { public PluginDefinition CreateDefinition() => WindowsPlugins.MainWindowBackdrop; }
