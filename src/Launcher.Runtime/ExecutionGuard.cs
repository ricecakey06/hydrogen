namespace Launcher.Runtime;

// Lifecycle mutations are serialized. Reject callbacks that would otherwise wait for themselves during shutdown.
internal sealed class ExecutionGuard
{
    private readonly AsyncLocal<LifetimeScope?> _current = new();

    public IDisposable Enter(LifetimeScope scope)
    {
        var previous = _current.Value;
        _current.Value = scope;
        return new Exit(() => _current.Value = previous);
    }

    public void EnsureOutsideCallback()
    {
        if (_current.Value is not null)
            throw new InvalidOperationException("Host composition cannot be changed from a plugin callback or a managed task. Use Context.CreateScope for owned temporary resources.");
    }

    public void EnsureCanDispose(LifetimeScope target)
    {
        for (var current = _current.Value; current is not null; current = current.Parent)
            if (ReferenceEquals(current, target))
                throw new InvalidOperationException("A managed callback cannot dispose its own lifetime or an ancestor lifetime.");
    }

    private sealed class Exit(Action action) : IDisposable
    {
        public void Dispose() => action();
    }
}
