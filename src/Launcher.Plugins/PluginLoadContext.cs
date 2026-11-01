using System.Reflection;
using System.Runtime.Loader;
using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Plugins;

internal sealed class PluginLoadContext(string assemblyPath) : AssemblyLoadContext(isCollectible: true)
{
    private readonly AssemblyDependencyResolver _resolver = new(assemblyPath);
    private readonly string _directory = Path.GetDirectoryName(assemblyPath)!;

    protected override Assembly? Load(AssemblyName assemblyName)
    {
        // Runtime/Contracts and any already-loaded UI contracts retain their host type identity.
        var shared = Default.Assemblies.FirstOrDefault(assembly => AssemblyName.ReferenceMatchesDefinition(assembly.GetName(), assemblyName));
        if (shared is not null) return shared;
        var resolved = _resolver.ResolveAssemblyToPath(assemblyName);
        var path = resolved ?? PackageFiles.Within(_directory, assemblyName.Name + ".dll");
        if (!File.Exists(path)) return null;
        PackageFiles.Within(_directory, Path.GetRelativePath(_directory, path));
        return LoadFromAssemblyPath(path);
    }

    protected override nint LoadUnmanagedDll(string unmanagedDllName)
    {
        var path = _resolver.ResolveUnmanagedDllToPath(unmanagedDllName);
        if (path is null) return 0;
        PackageFiles.Within(_directory, Path.GetRelativePath(_directory, path));
        return LoadUnmanagedDllFromPath(path);
    }

    public PluginDefinition Create(PluginPackage package, bool official = false)
    {
        // Official modules share assemblies (notably Avalonia resources and internal UI contracts).
        // They are still loaded from verified files in Plugins, never from compiled-in definitions.
        var assembly = official ? Default.LoadFromAssemblyPath(assemblyPath) : LoadFromAssemblyPath(assemblyPath);
        var type = assembly.GetType(package.EntryType, throwOnError: true)!;
        if (!typeof(IPluginModule).IsAssignableFrom(type) || type.IsAbstract || type.GetConstructor(Type.EmptyTypes) is null)
            throw new PluginManagementException("插件入口必须是具有公共无参构造函数的 IPluginModule。 ");
        var module = (IPluginModule)Activator.CreateInstance(type)!;
        var definition = module.CreateDefinition();
        if (definition is null || definition.Id != package.Id)
            throw new PluginManagementException("插件入口的 ID 与 plugin.json 不一致。 ");
        return definition;
    }
}
