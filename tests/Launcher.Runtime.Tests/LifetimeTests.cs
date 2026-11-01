using Launcher.Runtime;
using static Launcher.Runtime.Tests.TestPlugins;

namespace Launcher.Runtime.Tests;

public sealed class LifetimeTests
{
    [Fact]
    public async Task Shutdown_waits_for_tasks_before_releasing_services_and_rejects_new_work()
    {
        var entered = Signal();
        var canceled = Signal();
        var allowExit = Signal();
        var released = false;
        Context? active = null;
        await using var host = new PluginHost([
            Define("worker", context =>
            {
                active = context;
                context.Own(new Release(() => released = true));
                _ = context.Run(async token =>
                {
                    entered.SetResult();
                    try { await Task.Delay(Timeout.Infinite, token); }
                    finally { canceled.SetResult(); await allowExit.Task; }
                });
            })
        ]);
        await host.StartAsync();
        await Await(entered.Task);
        var stopping = host.DisposeAsync().AsTask();
        await Await(canceled.Task);
        Assert.False(stopping.IsCompleted);
        Assert.False(released);
        await Assert.ThrowsAsync<ObjectDisposedException>(() => active!.Run(_ => Task.CompletedTask));
        Assert.Throws<ObjectDisposedException>(() => active!.CreateScope("late"));
        Assert.Throws<ObjectDisposedException>(() => active!.Subscribe<int>((_, _) => ValueTask.CompletedTask));
        Assert.Throws<ObjectDisposedException>(() => active!.Own(new Release(() => { })));
        allowExit.SetResult();
        await Await(stopping);
        Assert.True(released);
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public async Task Child_lifetimes_are_isolated_cancel_independently_and_are_owned_by_the_parent()
    {
        LifetimeScope? first = null;
        LifetimeScope? second = null;
        Context? parent = null;
        var messages = new int[3];
        var disposed = new List<string>();
        await using var host = new PluginHost([
            Define("owner", context =>
            {
                parent = context;
                first = context.CreateScope("first");
                second = context.CreateScope("second");
                first.Own(new Release(() => disposed.Add("first")));
                second.Own(new Release(() => disposed.Add("second")));
                context.Own(new Release(() => disposed.Add("parent")));
                context.Subscribe<int>((_, _) => { messages[0]++; return ValueTask.CompletedTask; });
                first.Subscribe<int>((_, _) => { messages[1]++; return ValueTask.CompletedTask; });
                second.Subscribe<int>((_, _) => { messages[2]++; return ValueTask.CompletedTask; });
            })
        ]);
        await host.StartAsync();
        await first!.PublishAsync(1);
        Assert.Equal([0, 1, 0], messages);
        await first.DisposeAsync();
        await first.DisposeAsync();
        Assert.True(first.Stopping.IsCancellationRequested);
        Assert.False(second!.Stopping.IsCancellationRequested);
        await second.PublishAsync(1);
        await parent!.PublishAsync(1);
        Assert.Equal([1, 1, 1], messages);
        var reused = parent.CreateScope("first");
        Assert.False(reused.Stopping.IsCancellationRequested);
        await host.DisposeAsync();
        Assert.True(second.Stopping.IsCancellationRequested);
        Assert.True(reused.Stopping.IsCancellationRequested);
        Assert.Equal(["first", "second", "parent"], disposed);
    }

    [Fact]
    public async Task Cleanup_failure_does_not_skip_other_resources_children_or_cancellation_callbacks()
    {
        var order = new List<string>();
        var callbackEntered = Signal();
        await using var host = new PluginHost([
            Define("owner", context =>
            {
                context.OnDispose(() => order.Add("first"));
                context.OnDispose(() => throw new InvalidOperationException("cleanup failed"));
                context.OnDispose(() => order.Add("last"));
                context.CreateScope("child").OnDispose(() => throw new InvalidOperationException("child failed"));
                _ = context.Stopping.Register(() => { callbackEntered.SetResult(); throw new InvalidOperationException("callback failed"); });
            })
        ]);
        await host.StartAsync();
        await host.DisposeAsync();
        await Await(callbackEntered.Task);
        Assert.Equal(["last", "first"], order);
        Assert.Equal(3, host.GetErrors().Count);
        Assert.All(host.GetErrors(), error => Assert.Equal(RuntimeErrorKind.Cleanup, error.Kind));
    }

    [Fact]
    public async Task Owned_objects_are_released_once_with_async_disposal_preferred()
    {
        var resource = new DualResource();
        var borrowed = new First();
        await using var host = new PluginHost([
            Define("owner", context =>
            {
                context.Own(resource);
                context.Own(resource);
                context.Provide<IFirst>(borrowed, ownsService: false);
            }, provides: [typeof(IFirst)])
        ]);
        await host.StartAsync();
        await host.DisposeAsync();
        await host.DisposeAsync();
        Assert.Equal(1, resource.AsyncDisposals);
        Assert.Equal(0, resource.SyncDisposals);
        Assert.False(borrowed.IsDisposed);
    }

    [Fact]
    public async Task Background_faults_are_observed_once_and_do_not_prevent_cleanup()
    {
        Task? work = null;
        var released = false;
        await using var host = new PluginHost([
            Define("worker", context =>
            {
                context.OnDispose(() => released = true);
                work = context.Run(_ => throw new InvalidOperationException("task failed"));
            })
        ]);
        await host.StartAsync();
        await Assert.ThrowsAsync<InvalidOperationException>(() => work!);
        await host.DisposeAsync();
        Assert.True(released);
        Assert.Equal(RuntimeErrorKind.BackgroundTask, Assert.Single(host.GetErrors()).Kind);
    }

    [Fact]
    public async Task Event_dispatch_waits_for_all_handlers_and_records_faults()
    {
        Context? publisher = null;
        var deliveries = 0;
        await using var host = new PluginHost([
            Define("publisher", context => publisher = context),
            Define("bad", context => context.Subscribe<int>((_, _) => throw new InvalidOperationException("handler failed"))),
            Define("good", context => context.Subscribe<int>((_, _) => { Interlocked.Increment(ref deliveries); return ValueTask.CompletedTask; }))
        ]);
        await host.StartAsync();
        await Assert.ThrowsAsync<InvalidOperationException>(() => publisher!.PublishAsync(1));
        Assert.Equal(1, deliveries);
        Assert.Equal(RuntimeErrorKind.EventHandler, Assert.Single(host.GetErrors()).Kind);
    }

    [Fact]
    public async Task Inflight_event_handlers_are_canceled_and_joined_before_listener_resources_are_released()
    {
        Context? publisher = null;
        var entered = Signal();
        var canceled = Signal();
        var release = Signal();
        var disposed = false;
        await using var host = new PluginHost([
            Define("publisher", context => publisher = context),
            Define("listener", context =>
            {
                context.OnDispose(() => disposed = true);
                context.Subscribe<int>(async (_, token) =>
                {
                    entered.SetResult();
                    try { await Task.Delay(Timeout.Infinite, token); }
                    finally { canceled.SetResult(); await release.Task; }
                });
            })
        ]);
        await host.StartAsync();
        var publishing = publisher!.PublishAsync(1);
        await Await(entered.Task);
        var removing = host.Application.RemovePluginAsync("listener");
        await Await(canceled.Task);
        Assert.False(removing.IsCompleted);
        Assert.False(disposed);
        release.SetResult();
        await Await(removing);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => publishing);
        Assert.True(disposed);
        await publisher.PublishAsync(2);
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public async Task Explicit_unsubscribe_is_idempotent()
    {
        Context? publisher = null;
        IDisposable? subscription = null;
        var calls = 0;
        await using var host = new PluginHost([
            Define("owner", context =>
            {
                publisher = context;
                subscription = context.Subscribe<int>((_, _) => { calls++; return ValueTask.CompletedTask; });
            })
        ]);
        await host.StartAsync();
        await publisher!.PublishAsync(1);
        subscription!.Dispose();
        subscription.Dispose();
        await publisher.PublishAsync(2);
        Assert.Equal(1, calls);
    }

    [Fact]
    public async Task Stopping_a_publisher_cancels_its_dispatch_without_stopping_the_listener()
    {
        Task? publishing = null;
        var entered = Signal();
        var exited = Signal();
        var listenerDisposed = false;
        await using var host = new PluginHost([
            Define("listener", context =>
            {
                context.OnDispose(() => listenerDisposed = true);
                context.Subscribe<int>(async (_, token) =>
                {
                    entered.SetResult();
                    try { await Task.Delay(Timeout.Infinite, token); }
                    finally { exited.SetResult(); }
                });
            }),
            Define("publisher", context => publishing = context.Run(_ => context.PublishAsync(1)))
        ]);
        await host.StartAsync();
        await Await(entered.Task);
        await Await(host.Application.RemovePluginAsync("publisher"));
        await Await(exited.Task);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => publishing!);
        Assert.False(listenerDisposed);
        Assert.Equal(PluginState.Active, Assert.Single(await host.GetSnapshotAsync()).State);
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public async Task Lifecycle_reentrancy_and_self_disposal_fail_promptly_instead_of_deadlocking()
    {
        PluginHost? host = null;
        Task? task = null;
        host = new PluginHost([
            Define("owner", context =>
            {
                task = context.Run(async token =>
                {
                    await Assert.ThrowsAsync<InvalidOperationException>(() => host!.Application.RemovePluginAsync("owner", token));
                    var child = context.CreateScope("child");
                    await child.Run(async _ =>
                    {
                        await Assert.ThrowsAsync<InvalidOperationException>(() => child.DisposeAsync().AsTask());
                    });
                    await child.DisposeAsync();
                });
            })
        ]);
        await using (host)
        {
            await host.StartAsync();
            await Await(task!);
            Assert.Empty(host.GetErrors());
        }
    }

    [Fact]
    public async Task Services_cannot_be_registered_after_activation()
    {
        Context? context = null;
        await using var host = new PluginHost([Define("owner", active => context = active)]);
        await host.StartAsync();
        Assert.Throws<InvalidOperationException>(() => context!.Provide<IFirst>(new First()));
    }

    [Fact]
    public async Task Consumer_cleanup_can_access_dependencies_but_stale_contexts_cannot()
    {
        Context? consumer = null;
        var service = new First();
        var cleanupValue = 0;
        await using var host = new PluginHost([
            Define("provider", context => context.Provide<IFirst>(service), provides: [typeof(IFirst)]),
            Define("consumer", context =>
            {
                consumer = context;
                context.OnDispose(() => cleanupValue = context.GetRequiredService<IFirst>().Value);
            }, requires: [typeof(IFirst)])
        ]);
        await host.StartAsync();
        await host.DisposeAsync();
        Assert.Equal(1, cleanupValue);
        Assert.True(service.IsDisposed);
        Assert.Throws<ObjectDisposedException>(() => consumer!.GetRequiredService<IFirst>());
        Assert.Empty(host.GetErrors());
    }

    private sealed class DualResource : IDisposable, IAsyncDisposable
    {
        public int AsyncDisposals { get; private set; }
        public int SyncDisposals { get; private set; }
        public void Dispose() => SyncDisposals++;
        public ValueTask DisposeAsync() { AsyncDisposals++; return ValueTask.CompletedTask; }
    }
}
