using System.Collections.Concurrent;

namespace Launcher.Runtime;

/// <summary>Serializes explicit composition changes and manages dependency-driven plugin activations.</summary>
public sealed class PluginHost : IAsyncDisposable
{
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly ExecutionGuard _guard = new();
    private readonly ConcurrentQueue<RuntimeError> _errors = new();
    private bool _started;
    private bool _disposed;

    public PluginScope Application { get; }

    public PluginHost(IEnumerable<PluginDefinition> manifest)
    {
        ArgumentNullException.ThrowIfNull(manifest);
        Application = new PluginScope(this, "application", null, manifest);
        _ = new Composition(Application);
    }

    /// <summary>Starts available plugins. Missing dependencies wait; activation failures are isolated and diagnosed.</summary>
    public Task StartAsync(CancellationToken cancellationToken = default) => ExecuteAsync(async () =>
    {
        _started = true;
        try { await ReconcileAsync([], cancellationToken).ConfigureAwait(false); }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            await StopNodesAsync(AllNodes()).ConfigureAwait(false);
            _started = false;
            throw;
        }
    }, cancellationToken);

    public Task<IReadOnlyList<PluginSnapshot>> GetSnapshotAsync(CancellationToken cancellationToken = default)
        => ExecuteAsync<IReadOnlyList<PluginSnapshot>>(() =>
        {
            var composition = new Composition(Application);
            IReadOnlyList<PluginSnapshot> snapshot = Array.AsReadOnly(composition.Ordered.Select(node => new PluginSnapshot(
                node.Scope.Path, node.Definition.Id, node.State,
                Array.AsReadOnly(composition.Dependencies[node].Where(pair => pair.Value?.Context is null).Select(pair => pair.Key).ToArray()),
                node.ActivationError)).ToArray());
            return Task.FromResult(snapshot);
        }, cancellationToken);

    /// <summary>Returns diagnostic exceptions without formatting or persisting potentially sensitive plugin data.</summary>
    public IReadOnlyList<RuntimeError> GetErrors() => Array.AsReadOnly(_errors.ToArray());

    internal Task<PluginScope> CreateChildAsync(PluginScope parent, string name, PluginDefinition[] plugins, CancellationToken token)
        => ExecuteAsync(async () =>
        {
            EnsureScope(parent);
            LifetimeScope.ValidateName(name);
            if (parent.Children.Any(child => child.Name == name))
                throw new PluginCompositionException($"Child scope '{name}' already exists in '{parent.Path}'.");
            var child = new PluginScope(this, name, parent, plugins);
            parent.Children.Add(child);
            try { _ = new Composition(Application); }
            catch { parent.Children.Remove(child); throw; }
            if (_started) await ReconcileAsync([]).ConfigureAwait(false);
            return child;
        }, token);

    internal Task AddPluginAsync(PluginScope scope, PluginDefinition definition, CancellationToken token)
        => ExecuteAsync(async () =>
        {
            EnsureScope(scope);
            ArgumentNullException.ThrowIfNull(definition);
            var node = new PluginNode(scope, definition);
            scope.Plugins.Add(node);
            try { _ = new Composition(Application); }
            catch { scope.Plugins.Remove(node); throw; }
            if (_started) await ReconcileAsync([]).ConfigureAwait(false);
        }, token);

    internal Task RemovePluginAsync(PluginScope scope, string id, CancellationToken token)
        => ExecuteAsync(async () =>
        {
            EnsureScope(scope);
            var node = Find(scope, id);
            scope.Plugins.Remove(node);
            if (_started) await ReconcileAsync([node]).ConfigureAwait(false);
        }, token);

    internal Task ReplacePluginAsync(PluginScope scope, PluginDefinition definition, CancellationToken token)
        => ExecuteAsync(async () =>
        {
            EnsureScope(scope);
            ArgumentNullException.ThrowIfNull(definition);
            var previous = Find(scope, definition.Id);
            var index = scope.Plugins.IndexOf(previous);
            scope.Plugins[index] = new PluginNode(scope, definition);
            try { _ = new Composition(Application); }
            catch { scope.Plugins[index] = previous; throw; }
            if (_started) await ReconcileAsync([previous]).ConfigureAwait(false);
        }, token);

    internal Task RetryPluginAsync(PluginScope scope, string id, CancellationToken token)
        => ExecuteAsync(async () =>
        {
            EnsureScope(scope);
            var node = Find(scope, id);
            if (node.State == PluginState.Failed)
            {
                node.State = PluginState.Waiting;
                node.ActivationError = null;
                node.FailedBindings = null;
            }
            if (_started) await ReconcileAsync([]).ConfigureAwait(false);
        }, token);

    internal async Task DisposeScopeAsync(PluginScope scope)
    {
        _guard.EnsureOutsideCallback();
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            if (scope.IsDisposed) return;
            var scopes = Composition.EnumerateScopes(scope).ToArray();
            var retired = scopes.SelectMany(child => child.Plugins).ToArray();
            foreach (var child in scopes) child.IsDisposed = true;
            scope.Parent!.Children.Remove(scope);
            if (_started) await ReconcileAsync(retired).ConfigureAwait(false);
        }
        finally { _gate.Release(); }
    }

    private async Task ReconcileAsync(IEnumerable<PluginNode> retired, CancellationToken activationCancellation = default)
    {
        var composition = new Composition(Application);
        var stopping = retired.Where(node => node.Context is not null).ToHashSet();
        foreach (var node in composition.Ordered.Where(node => node.Context is not null))
            if (composition.Dependencies[node].Any(pair => pair.Value?.Context is not { } current
                || !ReferenceEquals(node.Context!.Providers[pair.Key], current)))
                stopping.Add(node);

        bool changed;
        do
        {
            changed = false;
            var contexts = stopping.Select(node => node.Context!).ToHashSet();
            foreach (var node in composition.Ordered.Where(node => node.Context is not null))
                if (node.Context!.Providers.Values.Any(contexts.Contains) && stopping.Add(node)) changed = true;
        } while (changed);
        await StopNodesAsync(stopping).ConfigureAwait(false);

        foreach (var node in composition.Ordered)
        {
            activationCancellation.ThrowIfCancellationRequested();
            if (node.Context is not null) continue;
            var dependencies = composition.Dependencies[node];
            if (node.State == PluginState.Failed && node.FailedBindings is { } failed
                && dependencies.Any(pair => !ReferenceEquals(pair.Value?.Context, failed[pair.Key])))
            {
                node.State = PluginState.Waiting;
                node.ActivationError = null;
                node.FailedBindings = null;
            }
            if (node.State == PluginState.Failed) continue;
            if (dependencies.Values.Any(provider => provider?.Context is null)) continue;
            var context = new Context(node.Definition, node.Scope.Path,
                dependencies.ToDictionary(pair => pair.Key, pair => pair.Value!.Context!), node.Scope.Events, _guard, Report);
            try
            {
                using var execution = _guard.Enter(context.Lifetime);
                using var activation = CancellationTokenSource.CreateLinkedTokenSource(context.Stopping, activationCancellation);
                await node.Definition.ActivateAsync(context, activation.Token).ConfigureAwait(false);
                activation.Token.ThrowIfCancellationRequested();
                context.Commit();
                node.Context = context;
                node.State = PluginState.Active;
                node.ActivationError = null;
                node.FailedBindings = null;
            }
            catch (Exception error)
            {
                context.Lifetime.BeginStop();
                await context.StopAsync().ConfigureAwait(false);
                if (error is OperationCanceledException && activationCancellation.IsCancellationRequested) throw;
                node.State = PluginState.Failed;
                node.ActivationError = error;
                node.FailedBindings = context.Providers;
                Report(new RuntimeError(RuntimeErrorKind.Activation, node.Path, node.Definition.Id, error));
            }
        }
    }

    private static async Task StopNodesAsync(IEnumerable<PluginNode> nodes)
    {
        var active = nodes.Where(node => node.Context is not null).Distinct().ToDictionary(node => node.Context!, node => node);
        var ordered = new List<Context>();
        var visited = new HashSet<Context>();
        void Visit(Context context)
        {
            if (!visited.Add(context)) return;
            foreach (var provider in context.Providers.Values.Where(active.ContainsKey)) Visit(provider);
            ordered.Add(context);
        }
        foreach (var context in active.Keys) Visit(context);
        // Freeze the entire affected graph before cancellation can run user callbacks.
        foreach (var context in ordered) context.Lifetime.BeginStop();
        foreach (var context in ordered.AsEnumerable().Reverse())
        {
            await context.StopAsync().ConfigureAwait(false);
            var node = active[context];
            node.Context = null;
            node.State = PluginState.Waiting;
        }
    }

    public async ValueTask DisposeAsync()
    {
        _guard.EnsureOutsideCallback();
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            if (_disposed) return;
            _disposed = true;
            foreach (var scope in Composition.EnumerateScopes(Application)) scope.IsDisposed = true;
            await StopNodesAsync(AllNodes()).ConfigureAwait(false);
        }
        finally { _gate.Release(); }
    }

    private IEnumerable<PluginNode> AllNodes() => Composition.EnumerateScopes(Application).SelectMany(scope => scope.Plugins);

    private static PluginNode Find(PluginScope scope, string id)
        => scope.Plugins.SingleOrDefault(node => node.Definition.Id == id)
            ?? throw new KeyNotFoundException($"Plugin '{id}' is not registered in '{scope.Path}'.");

    private static void EnsureScope(PluginScope scope) => ObjectDisposedException.ThrowIf(scope.IsDisposed, scope);

    private async Task ExecuteAsync(Func<Task> action, CancellationToken token)
    {
        await ExecuteAsync(async () => { await action().ConfigureAwait(false); return true; }, token).ConfigureAwait(false);
    }

    private async Task<T> ExecuteAsync<T>(Func<Task<T>> action, CancellationToken token)
    {
        _guard.EnsureOutsideCallback();
        await _gate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            token.ThrowIfCancellationRequested();
            return await action().ConfigureAwait(false);
        }
        finally { _gate.Release(); }
    }

    private void Report(RuntimeError error) => _errors.Enqueue(error);
}
