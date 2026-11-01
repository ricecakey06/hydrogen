namespace Launcher.Runtime;

internal sealed class EventHub
{
    private readonly object _sync = new();
    private readonly Dictionary<Type, List<object>> _subscriptions = [];

    public IDisposable Subscribe<T>(LifetimeScope owner, Func<T, CancellationToken, ValueTask> handler)
    {
        var subscription = new Subscription<T>(this, owner, handler);
        lock (_sync)
        {
            if (!_subscriptions.TryGetValue(typeof(T), out var handlers))
                _subscriptions.Add(typeof(T), handlers = []);
            handlers.Add(subscription);
        }
        return subscription;
    }

    public Task PublishAsync<T>(T notification, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        Subscription<T>[] handlers;
        lock (_sync)
            handlers = _subscriptions.TryGetValue(typeof(T), out var registered)
                ? registered.Cast<Subscription<T>>().ToArray() : [];
        return Task.WhenAll(handlers.Select(handler => handler.Invoke(notification, cancellationToken)));
    }

    private void Remove<T>(Subscription<T> subscription)
    {
        lock (_sync)
        {
            if (!_subscriptions.TryGetValue(typeof(T), out var handlers)) return;
            handlers.Remove(subscription);
            if (handlers.Count == 0) _subscriptions.Remove(typeof(T));
        }
    }

    private sealed class Subscription<T>(EventHub hub, LifetimeScope owner, Func<T, CancellationToken, ValueTask> handler) : IDisposable
    {
        private int _disposed;

        public Task Invoke(T notification, CancellationToken cancellationToken)
        {
            if (Volatile.Read(ref _disposed) != 0) return Task.CompletedTask;
            return owner.TryRun(token => handler(notification, token).AsTask(), cancellationToken, RuntimeErrorKind.EventHandler)
                ?? Task.CompletedTask;
        }

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _disposed, 1) == 0) hub.Remove(this);
        }
    }
}
