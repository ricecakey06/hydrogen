namespace Launcher.Runtime;

/// <summary>Owns temporary resources, typed subscriptions, and cancellable work. Child scopes have isolated event channels.</summary>
public sealed class LifetimeScope : IAsyncDisposable
{
    private readonly object _sync = new();
    private readonly CancellationTokenSource _cancellation = new();
    private readonly List<LifetimeScope> _children = [];
    private readonly List<Func<ValueTask>> _cleanup = [];
    private readonly HashSet<object> _owned = new(ReferenceEqualityComparer.Instance);
    private readonly HashSet<Task> _tasks = [];
    private readonly EventHub _events;
    private readonly ExecutionGuard _guard;
    private readonly Action<RuntimeError> _report;
    private readonly string _pluginId;
    private bool _accepting = true;
    private bool _disposed;
    private Task? _stopTask;

    public string Path { get; }
    public CancellationToken Stopping { get; }
    public bool IsStopping { get { lock (_sync) return !_accepting; } }
    internal bool IsDisposed { get { lock (_sync) return _disposed; } }
    internal LifetimeScope? Parent { get; }

    internal LifetimeScope(string path, string pluginId, EventHub events, ExecutionGuard guard,
        Action<RuntimeError> report, LifetimeScope? parent = null)
    {
        Path = path;
        _pluginId = pluginId;
        _events = events;
        _guard = guard;
        _report = report;
        Parent = parent;
        Stopping = _cancellation.Token;
    }

    public LifetimeScope CreateScope(string name)
    {
        ValidateName(name);
        lock (_sync)
        {
            EnsureAccepting();
            var path = $"{Path}/{name}";
            if (_children.Any(child => child.Path == path))
                throw new InvalidOperationException($"A live child scope already exists at '{path}'.");
            var child = new LifetimeScope(path, _pluginId, new EventHub(), _guard, _report, this);
            _children.Add(child);
            return child;
        }
    }

    /// <summary>Resources implementing both disposal interfaces use asynchronous disposal. The same object is owned only once.</summary>
    public T Own<T>(T resource) where T : class
    {
        ArgumentNullException.ThrowIfNull(resource);
        if (resource is not IDisposable && resource is not IAsyncDisposable)
            throw new ArgumentException("An owned resource must implement IDisposable or IAsyncDisposable.", nameof(resource));
        lock (_sync)
        {
            EnsureAccepting();
            if (_owned.Add(resource))
                _cleanup.Add(resource is IAsyncDisposable asyncResource
                    ? asyncResource.DisposeAsync
                    : () => { ((IDisposable)resource).Dispose(); return ValueTask.CompletedTask; });
        }
        return resource;
    }

    public void OnDispose(Action cleanup)
    {
        ArgumentNullException.ThrowIfNull(cleanup);
        OnDisposeAsync(() => { cleanup(); return ValueTask.CompletedTask; });
    }

    public void OnDisposeAsync(Func<ValueTask> cleanup)
    {
        ArgumentNullException.ThrowIfNull(cleanup);
        lock (_sync)
        {
            EnsureAccepting();
            _cleanup.Add(cleanup);
        }
    }

    /// <summary>The returned task preserves faults and cancellation. Faults are also observed and added to host diagnostics.</summary>
    public Task Run(Func<CancellationToken, Task> work)
    {
        ArgumentNullException.ThrowIfNull(work);
        return TryRun(work, CancellationToken.None, RuntimeErrorKind.BackgroundTask)
            ?? throw new ObjectDisposedException(Path);
    }

    public IDisposable Subscribe<T>(Func<T, CancellationToken, ValueTask> handler)
    {
        ArgumentNullException.ThrowIfNull(handler);
        lock (_sync)
        {
            EnsureAccepting();
            var subscription = _events.Subscribe(this, handler);
            _cleanup.Add(() => { subscription.Dispose(); return ValueTask.CompletedTask; });
            return subscription;
        }
    }

    /// <summary>Dispatches to this event channel only, awaiting all handlers even when one fails.</summary>
    public async Task PublishAsync<T>(T notification, CancellationToken cancellationToken = default)
    {
        lock (_sync) EnsureAccepting();
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(Stopping, cancellationToken);
        await _events.PublishAsync(notification, linked.Token).ConfigureAwait(false);
    }

    internal Task? TryRun(Func<CancellationToken, Task> work, CancellationToken cancellationToken, RuntimeErrorKind kind)
    {
        lock (_sync)
        {
            if (!_accepting) return null;
            var task = Task.Run(async () =>
            {
                using var execution = _guard.Enter(this);
                using var linked = CancellationTokenSource.CreateLinkedTokenSource(Stopping, cancellationToken);
                try
                {
                    linked.Token.ThrowIfCancellationRequested();
                    await work(linked.Token).ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (linked.IsCancellationRequested) { throw; }
                catch (Exception error)
                {
                    Report(kind, error);
                    throw;
                }
            });
            _tasks.Add(task);
            _ = task.ContinueWith(completed =>
            {
                // Observe faults even if a plugin intentionally does not await its background task.
                _ = completed.Exception;
                lock (_sync) _tasks.Remove(completed);
            }, CancellationToken.None, TaskContinuationOptions.ExecuteSynchronously, TaskScheduler.Default);
            return task;
        }
    }

    internal void BeginStop()
    {
        LifetimeScope[] children;
        lock (_sync)
        {
            _accepting = false;
            children = _children.ToArray();
        }
        foreach (var child in children) child.BeginStop();
    }

    public ValueTask DisposeAsync()
    {
        _guard.EnsureCanDispose(this);
        return new ValueTask(StopAsync());
    }

    internal Task StopAsync()
    {
        lock (_sync) return _stopTask ??= StopCoreAsync();
    }

    private async Task StopCoreAsync()
    {
        BeginStop();
        // Do not invoke user cleanup while holding the lock that publishes the shared stop task.
        await Task.Yield();
        using var execution = _guard.Enter(this);
        try { await _cancellation.CancelAsync().ConfigureAwait(false); }
        catch (Exception error) { Report(RuntimeErrorKind.Cleanup, error); }

        LifetimeScope[] children;
        Task[] tasks;
        Func<ValueTask>[] cleanup;
        lock (_sync)
        {
            children = _children.ToArray();
            tasks = _tasks.ToArray();
            cleanup = _cleanup.ToArray();
        }

        foreach (var child in children.Reverse())
        {
            try { await child.StopAsync().ConfigureAwait(false); }
            catch (Exception error) { Report(RuntimeErrorKind.Cleanup, error); }
        }
        try { await Task.WhenAll(tasks).ConfigureAwait(false); }
        catch (Exception) { /* Run observes and reports faults; cancellation is an expected shutdown result. */ }

        foreach (var release in cleanup.Reverse())
        {
            try { await release().ConfigureAwait(false); }
            catch (Exception error) { Report(RuntimeErrorKind.Cleanup, error); }
        }
        lock (_sync)
        {
            _children.Clear();
            _tasks.Clear();
            _cleanup.Clear();
            _owned.Clear();
            _disposed = true;
        }
        _cancellation.Dispose();
        if (Parent is not null)
            lock (Parent._sync) Parent._children.Remove(this);
    }

    internal void EnsureAccepting()
    {
        if (!_accepting) throw new ObjectDisposedException(Path);
    }

    private void Report(RuntimeErrorKind kind, Exception error) => _report(new RuntimeError(kind, Path, _pluginId, error));

    internal static void ValidateName(string name)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        if (name is "." or ".." || name.Contains('/') || name.Contains('\\'))
            throw new ArgumentException("Scope names must be a single, non-relative path segment.", nameof(name));
    }
}
