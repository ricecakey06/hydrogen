using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.PluginFixtures;

public interface IRequiredService;
public sealed class RequiredService : IRequiredService;
public sealed class CoreAppModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => new("core.app", (_, _) => ValueTask.CompletedTask,
        requires: [typeof(IPluginManager), typeof(IRequiredService)]);
}
public sealed class CoreDataModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => new("core.data", (context, _) =>
    { context.Provide<IRequiredService>(new RequiredService()); return ValueTask.CompletedTask; }, provides: [typeof(IRequiredService)]);
}
public sealed class OptionalEffectModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => new("optional.effect", (_, _) => ValueTask.CompletedTask);
}

public sealed class EchoModule : IPluginModule
{
    public EchoModule()
    {
        if (AppContext.GetData("Hydrogen.PluginFixture.Marker") is string marker) File.WriteAllText(marker, "constructed");
    }
    public PluginDefinition CreateDefinition() => new("example.echo", (_, _) => ValueTask.CompletedTask);
}

public sealed class DependentModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => new("example.dependent", (_, _) => ValueTask.CompletedTask);
}

public sealed class ConflictingModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => new("example.conflict", (_, _) => ValueTask.CompletedTask, provides: [typeof(IPluginManager)]);
}

public sealed class FailingModule : IPluginModule
{
    public PluginDefinition CreateDefinition() => new("example.failure", (_, _) => throw new InvalidOperationException("Sensitive fixture payload"));
}
