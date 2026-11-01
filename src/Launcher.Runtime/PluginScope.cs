namespace Launcher.Runtime;

/// <summary>A composition scope. Services resolve locally first, then through ancestor scopes; sibling scopes are isolated.</summary>
public sealed class PluginScope : IAsyncDisposable
{
    private readonly PluginHost _host;
    public string Name { get; }
    public string Path { get; }
    internal PluginScope? Parent { get; }
    internal List<PluginScope> Children { get; } = [];
    internal List<PluginNode> Plugins { get; } = [];
    internal EventHub Events { get; } = new();
    internal bool IsDisposed { get; set; }

    internal PluginScope(PluginHost host, string name, PluginScope? parent, IEnumerable<PluginDefinition> plugins)
    {
        _host = host;
        Name = name;
        Parent = parent;
        Path = parent is null ? name : $"{parent.Path}/{name}";
        Plugins.AddRange(plugins.Select(definition => new PluginNode(this, definition)));
    }

    public Task<PluginScope> CreateChildAsync(string name, IEnumerable<PluginDefinition>? plugins = null, CancellationToken cancellationToken = default)
        => _host.CreateChildAsync(this, name, plugins?.ToArray() ?? [], cancellationToken);

    public Task AddPluginAsync(PluginDefinition plugin, CancellationToken cancellationToken = default)
        => _host.AddPluginAsync(this, plugin, cancellationToken);

    public Task RemovePluginAsync(string id, CancellationToken cancellationToken = default)
        => _host.RemovePluginAsync(this, id, cancellationToken);

    public Task ReplacePluginAsync(PluginDefinition replacement, CancellationToken cancellationToken = default)
        => _host.ReplacePluginAsync(this, replacement, cancellationToken);

    public Task RetryPluginAsync(string id, CancellationToken cancellationToken = default)
        => _host.RetryPluginAsync(this, id, cancellationToken);

    public ValueTask DisposeAsync() => Parent is null ? _host.DisposeAsync() : new ValueTask(_host.DisposeScopeAsync(this));
}

internal sealed class PluginNode(PluginScope scope, PluginDefinition definition)
{
    public PluginScope Scope { get; } = scope;
    public PluginDefinition Definition { get; } = definition ?? throw new ArgumentNullException(nameof(definition));
    public Context? Context { get; set; }
    public PluginState State { get; set; } = PluginState.Waiting;
    public Exception? ActivationError { get; set; }
    public IReadOnlyDictionary<Type, Context>? FailedBindings { get; set; }
    public string Path => $"{Scope.Path}/{Definition.Id}";
}
