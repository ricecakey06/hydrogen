using Launcher.Runtime;
using static Launcher.Runtime.Tests.TestPlugins;

namespace Launcher.Runtime.Tests;

public sealed class CompositionTests
{
    [Fact]
    public async Task Manifest_order_does_not_control_activation_order()
    {
        var order = new List<string>();
        await using var host = new PluginHost([
            Define("consumer", context => { order.Add("consumer"); Assert.Equal(7, context.GetRequiredService<IFirst>().Value); }, requires: [typeof(IFirst)]),
            Define("provider", context => { order.Add("provider"); context.Provide<IFirst>(new First(7)); }, provides: [typeof(IFirst)])
        ]);
        await host.StartAsync();
        Assert.Equal(["provider", "consumer"], order);
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public async Task Missing_services_wait_and_repeated_add_remove_does_not_duplicate_listeners()
    {
        var activations = 0;
        var deliveries = 0;
        Context? providerContext = null;
        await using var host = new PluginHost([
            Define("consumer", context =>
            {
                activations++;
                context.Subscribe<int>((_, _) => { deliveries++; return ValueTask.CompletedTask; });
            }, requires: [typeof(IFirst)])
        ]);
        await host.StartAsync();
        var waiting = Assert.Single(await host.GetSnapshotAsync());
        Assert.Equal(PluginState.Waiting, waiting.State);
        Assert.Equal([typeof(IFirst)], waiting.MissingServices);
        for (var iteration = 0; iteration < 3; iteration++)
        {
            await host.Application.AddPluginAsync(Define("provider", context =>
            {
                providerContext = context;
                context.Provide<IFirst>(new First());
            }, provides: [typeof(IFirst)]));
            await providerContext!.PublishAsync(1);
            Assert.Equal(iteration + 1, deliveries);
            await host.Application.RemovePluginAsync("provider");
            Assert.Equal(PluginState.Waiting, Assert.Single(await host.GetSnapshotAsync()).State);
        }
        Assert.Equal(3, activations);
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public void Conflicts_and_duplicate_ids_are_rejected_before_activation()
    {
        var activations = 0;
        var first = Define("first", _ => activations++, provides: [typeof(IFirst)]);
        var conflict = Define("conflict", _ => activations++, provides: [typeof(IFirst)]);
        var error = Assert.Throws<PluginCompositionException>(() => new PluginHost([first, conflict]));
        Assert.Contains("IFirst", error.Message);
        Assert.Throws<PluginCompositionException>(() => new PluginHost([first, first]));
        Assert.Equal(0, activations);
    }

    [Fact]
    public void Cycles_are_reported_even_when_another_dependency_is_missing()
    {
        var error = Assert.Throws<PluginCompositionException>(() => new PluginHost([
            Define("first", _ => { }, requires: [typeof(ISecond), typeof(IThird)], provides: [typeof(IFirst)]),
            Define("second", _ => { }, requires: [typeof(IFirst)], provides: [typeof(ISecond)])
        ]));
        Assert.Contains("application/first -> application/second -> application/first", error.Message);
        Assert.Throws<PluginCompositionException>(() => new PluginHost([
            Define("self", _ => { }, requires: [typeof(IFirst)], provides: [typeof(IFirst)])
        ]));
    }

    [Fact]
    public async Task Invalid_changes_preserve_the_running_composition()
    {
        var disposed = 0;
        var consumerStarts = 0;
        await using var host = new PluginHost([
            Define("provider", context => context.Provide<IFirst>(new First(dispose: () => disposed++)), provides: [typeof(IFirst)]),
            Define("consumer", _ => consumerStarts++, requires: [typeof(IFirst)])
        ]);
        await host.StartAsync();
        await Assert.ThrowsAsync<PluginCompositionException>(() => host.Application.AddPluginAsync(
            Define("conflict", _ => { }, provides: [typeof(IFirst)])));
        await Assert.ThrowsAsync<PluginCompositionException>(() => host.Application.ReplacePluginAsync(
            Define("provider", _ => { }, requires: [typeof(IFirst)], provides: [typeof(IFirst)])));
        Assert.Equal(0, disposed);
        Assert.Equal(1, consumerStarts);
        Assert.Equal(2, (await host.GetSnapshotAsync()).Count);
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
    }

    [Fact]
    public async Task Dependency_removal_stops_transitive_consumers_before_providers()
    {
        var order = new List<string>();
        var service = new First(dispose: () => order.Add("provider"));
        await using var host = new PluginHost([
            Define("provider", context => context.Provide<IFirst>(service), provides: [typeof(IFirst)]),
            Define("middle", context =>
            {
                context.Provide<ISecond>(new Second());
                context.OnDispose(() => { Assert.False(service.IsDisposed); order.Add("middle"); });
            }, requires: [typeof(IFirst)], provides: [typeof(ISecond)]),
            Define("consumer", context => context.OnDispose(() => order.Add("consumer")), requires: [typeof(ISecond)])
        ]);
        await host.StartAsync();
        await host.Application.RemovePluginAsync("provider");
        Assert.Equal(["consumer", "middle", "provider"], order);
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Waiting, plugin.State));
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public async Task Replacement_rebinds_consumers_without_restarting_unrelated_plugins()
    {
        var values = new List<int>();
        var disposed = 0;
        var unrelatedStarts = 0;
        await using var host = new PluginHost([
            Define("provider", context => context.Provide<IFirst>(new First(1, () => disposed++)), provides: [typeof(IFirst)]),
            Define("consumer", context => values.Add(context.GetRequiredService<IFirst>().Value), requires: [typeof(IFirst)]),
            Define("unrelated", _ => unrelatedStarts++)
        ]);
        await host.StartAsync();
        await host.Application.ReplacePluginAsync(Define("provider", context => context.Provide<IFirst>(new First(2)), provides: [typeof(IFirst)]));
        Assert.Equal([1, 2], values);
        Assert.Equal(1, disposed);
        Assert.Equal(1, unrelatedStarts);
    }

    [Fact]
    public async Task Activation_failure_rolls_back_exports_tasks_children_and_subscriptions_then_can_retry()
    {
        var attempt = 0;
        var disposed = 0;
        var deliveries = 0;
        var consumerStarts = 0;
        var taskStarted = Signal();
        var taskStopped = Signal();
        Context? publisher = null;
        var failing = new PluginDefinition("failing", async (context, activationToken) =>
        {
            context.Provide<IFirst>(new First(dispose: () => disposed++));
            context.Subscribe<int>((_, _) => { deliveries++; return ValueTask.CompletedTask; });
            context.CreateScope("temporary").Own(new Release(() => disposed++));
            _ = context.Run(async token =>
            {
                taskStarted.TrySetResult();
                try { await Task.Delay(Timeout.Infinite, token); }
                finally { taskStopped.TrySetResult(); }
            });
            await Await(taskStarted.Task);
            if (++attempt == 1) throw new InvalidOperationException("failed activation");
        }, provides: [typeof(IFirst)]);
        await using var host = new PluginHost([
            failing,
            Define("consumer", _ => consumerStarts++, requires: [typeof(IFirst)]),
            Define("publisher", context => publisher = context)
        ]);
        await host.StartAsync();
        await Await(taskStopped.Task);
        Assert.Equal(2, disposed);
        Assert.Equal(0, consumerStarts);
        Assert.Equal(PluginState.Failed, Assert.Single(await host.GetSnapshotAsync(), plugin => plugin.Id == "failing").State);
        await publisher!.PublishAsync(1);
        Assert.Equal(0, deliveries);
        Assert.Single(host.GetErrors(), error => error.Kind == RuntimeErrorKind.Activation);

        await host.Application.RetryPluginAsync("failing");
        await publisher.PublishAsync(1);
        Assert.Equal(1, deliveries);
        Assert.Equal(1, consumerStarts);
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
    }

    [Fact]
    public async Task Incomplete_and_undeclared_registrations_are_isolated_activation_errors()
    {
        await using var host = new PluginHost([
            Define("incomplete", _ => { }, provides: [typeof(IFirst)]),
            Define("undeclared", context => context.Provide<ISecond>(new Second())),
            Define("undeclared.consumer", context => context.GetRequiredService<IFirst>()),
            Define("healthy", _ => { })
        ]);
        await host.StartAsync();
        Assert.Equal(3, host.GetErrors().Count);
        Assert.Equal(PluginState.Active, Assert.Single(await host.GetSnapshotAsync(), plugin => plugin.Id == "healthy").State);
        Assert.Equal(3, (await host.GetSnapshotAsync()).Count(plugin => plugin.State == PluginState.Failed));
    }

    [Fact]
    public async Task Failed_plugins_do_not_retry_until_explicitly_requested()
    {
        var attempts = 0;
        await using var host = new PluginHost([Define("failed", _ => { attempts++; throw new InvalidOperationException(); })]);
        await host.StartAsync();
        await host.StartAsync();
        await host.Application.AddPluginAsync(Define("unrelated", _ => { }));
        Assert.Equal(1, attempts);
        await host.Application.RetryPluginAsync("failed");
        Assert.Equal(2, attempts);
    }

    [Fact]
    public async Task Failed_consumers_retry_when_their_dependency_is_replaced()
    {
        var attempts = 0;
        await using var host = new PluginHost([
            Define("provider", context => context.Provide<IFirst>(new First(1)), provides: [typeof(IFirst)]),
            Define("consumer", context =>
            {
                attempts++;
                if (context.GetRequiredService<IFirst>().Value == 1) throw new InvalidOperationException();
            }, requires: [typeof(IFirst)])
        ]);
        await host.StartAsync();
        Assert.Equal(PluginState.Failed, Assert.Single(await host.GetSnapshotAsync(), plugin => plugin.Id == "consumer").State);
        await host.Application.ReplacePluginAsync(Define("provider", context => context.Provide<IFirst>(new First(2)), provides: [typeof(IFirst)]));
        Assert.Equal(2, attempts);
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
    }

    [Fact]
    public async Task Startup_cancellation_rolls_back_successful_and_in_progress_activations()
    {
        var disposed = 0;
        var entered = Signal();
        var attempt = 0;
        using var cancellation = new CancellationTokenSource();
        await using var host = new PluginHost([
            Define("first", context => context.Own(new Release(() => disposed++))),
            new PluginDefinition("slow", async (context, token) =>
            {
                context.Own(new Release(() => disposed++));
                entered.TrySetResult();
                if (++attempt == 1) await Task.Delay(Timeout.Infinite, token);
            })
        ]);
        var starting = host.StartAsync(cancellation.Token);
        await Await(entered.Task);
        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => Await(starting));
        Assert.Equal(2, disposed);
        Assert.Empty(host.GetErrors());
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Waiting, plugin.State));
        await host.StartAsync();
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
    }

    [Fact]
    public async Task Cancellation_while_waiting_for_the_composition_lock_does_not_apply_the_command()
    {
        var entered = Signal();
        var release = Signal();
        await using var host = new PluginHost([new PluginDefinition("slow", async (_, _) => { entered.SetResult(); await release.Task; })]);
        var starting = host.StartAsync();
        await Await(entered.Task);
        using var cancellation = new CancellationTokenSource();
        var adding = host.Application.AddPluginAsync(Define("unexpected", _ => { }), cancellation.Token);
        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => Await(adding));
        release.SetResult();
        await starting;
        Assert.Single(await host.GetSnapshotAsync());
    }
}
