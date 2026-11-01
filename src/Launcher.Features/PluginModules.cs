using System.Reflection;
using Launcher.Plugins.Infrastructure;
using Launcher.Runtime;

namespace Launcher.Plugins;

// Small package entry points retain the existing logical plugin IDs and independent toggles.
internal static class FeatureModule
{
    public static PluginDefinition Get(string id)
    {
        var clientId = Environment.GetEnvironmentVariable("HYDROGEN_MICROSOFT_CLIENT_ID")
            ?? Assembly.GetEntryAssembly()?.GetCustomAttributes<AssemblyMetadataAttribute>()
                .FirstOrDefault(attribute => attribute.Key == "MicrosoftClientId")?.Value;
        return FunctionalPlugins.Create(new(clientId)).Single(definition => definition.Id == id);
    }
}

public sealed class StorageModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("storage"); }
public sealed class NetworkModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("network"); }
public sealed class ProcessModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("process"); }
public sealed class DiagnosticsModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("diagnostics"); }
public sealed class ContentModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("minecraft.content"); }
public sealed class FilesModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("minecraft.files"); }
public sealed class InstancesModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("minecraft.instances"); }
public sealed class JavaModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("java.runtime"); }
public sealed class InstallModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("minecraft.install"); }
public sealed class MicrosoftModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("account.microsoft"); }
public sealed class AccountModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("account.manager"); }
public sealed class LaunchModule : IPluginModule { public PluginDefinition CreateDefinition() => FeatureModule.Get("minecraft.launch"); }
public sealed class OfflineModule : IPluginModule { public PluginDefinition CreateDefinition() => FunctionalPlugins.OfflineAccount; }
public sealed class SessionCredentialsModule : IPluginModule { public PluginDefinition CreateDefinition() => FunctionalPlugins.SessionCredentials; }
public sealed class PlatformModule : IPluginModule { public PluginDefinition CreateDefinition() => FunctionalPlugins.Platform(new PortableEnvironment()); }
