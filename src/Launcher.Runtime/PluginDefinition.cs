using System.Collections.ObjectModel;
using System.Text.RegularExpressions;

namespace Launcher.Runtime;

/// <summary>A trusted, compiled-in plugin. Declared exports make composition validation independent of activation order.</summary>
public sealed class PluginDefinition
{
    public string Id { get; }
    public IReadOnlyList<Type> Requires { get; }
    public IReadOnlyList<Type> Provides { get; }
    public Func<Context, CancellationToken, ValueTask> ActivateAsync { get; }

    public PluginDefinition(
        string id,
        Func<Context, CancellationToken, ValueTask> activateAsync,
        IEnumerable<Type>? requires = null,
        IEnumerable<Type>? provides = null)
    {
        ArgumentNullException.ThrowIfNull(id);
        ArgumentNullException.ThrowIfNull(activateAsync);
        if (!Regex.IsMatch(id, "^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$", RegexOptions.CultureInvariant))
            throw new ArgumentException("Plugin IDs must be stable lowercase identifiers, for example 'minecraft.launch'.", nameof(id));

        Id = id;
        ActivateAsync = activateAsync;
        Requires = CopyContracts(requires, nameof(requires));
        Provides = CopyContracts(provides, nameof(provides));
    }

    private static ReadOnlyCollection<Type> CopyContracts(IEnumerable<Type>? source, string parameter)
    {
        var types = source?.ToArray() ?? [];
        if (types.Any(type => type is null || (!type.IsClass && !type.IsInterface) || type.ContainsGenericParameters))
            throw new ArgumentException("Services must be closed reference types.", parameter);
        if (types.Distinct().Count() != types.Length)
            throw new ArgumentException("A service contract may only be declared once.", parameter);
        return Array.AsReadOnly(types);
    }
}
