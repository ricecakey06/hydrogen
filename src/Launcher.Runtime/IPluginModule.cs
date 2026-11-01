namespace Launcher.Runtime;

/// <summary>The entry point of a local Hydrogen plugin package. Called only during application startup.</summary>
public interface IPluginModule
{
    PluginDefinition CreateDefinition();
}
