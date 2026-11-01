namespace Launcher.Runtime;

/// <summary>A single activation's service bindings and owned effects. A fresh context is used on every reactivation.</summary>
public sealed class Context
{
    private readonly PluginDefinition _definition;
    private readonly Dictionary<Type, object> _services = [];
    private readonly object _sync = new();
    private bool _sealed;

    public string PluginId => _definition.Id;
    public string ScopePath { get; }
    public CancellationToken Stopping => Lifetime.Stopping;
    internal LifetimeScope Lifetime { get; }
    internal IReadOnlyDictionary<Type, Context> Providers { get; }

    internal Context(PluginDefinition definition, string scopePath, Dictionary<Type, Context> providers,
        EventHub events, ExecutionGuard guard, Action<RuntimeError> report)
    {
        _definition = definition;
        ScopePath = scopePath;
        Providers = providers;
        Lifetime = new LifetimeScope($"{scopePath}/{definition.Id}", definition.Id, events, guard, report);
    }

    public T GetRequiredService<T>() where T : class
    {
        ObjectDisposedException.ThrowIf(Lifetime.IsDisposed, this);
        if (!Providers.TryGetValue(typeof(T), out var provider))
            throw new InvalidOperationException($"Plugin '{PluginId}' did not declare required service '{typeof(T)}'.");
        return (T)provider.GetExport(typeof(T));
    }

    /// <summary>Only declared services may be provided, and only during activation. Exports become visible together on success.</summary>
    public void Provide<T>(T service, bool ownsService = true) where T : class
    {
        ArgumentNullException.ThrowIfNull(service);
        lock (_sync)
        {
            if (_sealed || Lifetime.IsStopping) throw new InvalidOperationException("Service registration is closed for this activation.");
            if (!_definition.Provides.Contains(typeof(T)))
                throw new InvalidOperationException($"Plugin '{PluginId}' did not declare export '{typeof(T)}'.");
            if (!_services.TryAdd(typeof(T), service))
                throw new PluginCompositionException($"Plugin '{PluginId}' already registered '{typeof(T)}'.");
            if (ownsService && service is IDisposable or IAsyncDisposable) Lifetime.Own(service);
        }
    }

    public LifetimeScope CreateScope(string name) => Lifetime.CreateScope(name);
    public T Own<T>(T resource) where T : class => Lifetime.Own(resource);
    public void OnDispose(Action cleanup) => Lifetime.OnDispose(cleanup);
    public void OnDisposeAsync(Func<ValueTask> cleanup) => Lifetime.OnDisposeAsync(cleanup);
    public Task Run(Func<CancellationToken, Task> work) => Lifetime.Run(work);
    public IDisposable Subscribe<T>(Func<T, CancellationToken, ValueTask> handler) => Lifetime.Subscribe(handler);
    public Task PublishAsync<T>(T notification, CancellationToken cancellationToken = default) => Lifetime.PublishAsync(notification, cancellationToken);

    internal object GetExport(Type service)
    {
        ObjectDisposedException.ThrowIf(Lifetime.IsDisposed, this);
        lock (_sync) return _services[service];
    }

    internal void Commit()
    {
        lock (_sync)
        {
            var missing = _definition.Provides.Where(type => !_services.ContainsKey(type)).ToArray();
            if (missing.Length != 0)
                throw new InvalidOperationException($"Plugin '{PluginId}' did not register declared exports: {string.Join(", ", missing.Select(type => type.Name))}.");
            _sealed = true;
        }
    }

    internal async Task StopAsync()
    {
        await Lifetime.StopAsync().ConfigureAwait(false);
        lock (_sync)
        {
            _sealed = true;
            _services.Clear();
        }
    }
}
