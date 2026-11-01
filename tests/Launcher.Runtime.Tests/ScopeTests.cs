using Launcher.Runtime;
using static Launcher.Runtime.Tests.TestPlugins;

namespace Launcher.Runtime.Tests;

public sealed class ScopeTests
{
    [Fact]
    public async Task Child_services_shadow_ancestors_without_leaking_to_siblings()
    {
        var childValues = new List<int>();
        var siblingValues = new List<int>();
        var parentStarts = 0;
        await using var host = new PluginHost([
            Define("provider", context => { parentStarts++; context.Provide<IFirst>(new First(1)); }, provides: [typeof(IFirst)])
        ]);
        var child = await host.Application.CreateChildAsync("child", [
            Define("consumer", context => childValues.Add(context.GetRequiredService<IFirst>().Value), requires: [typeof(IFirst)])
        ]);
        await host.Application.CreateChildAsync("sibling", [
            Define("consumer", context => siblingValues.Add(context.GetRequiredService<IFirst>().Value), requires: [typeof(IFirst)])
        ]);
        Assert.Empty(childValues);
        await host.StartAsync();
        await child.AddPluginAsync(Define("local", context => context.Provide<IFirst>(new First(2)), provides: [typeof(IFirst)]));
        await child.RemovePluginAsync("local");
        Assert.Equal([1, 2, 1], childValues);
        Assert.Equal([1], siblingValues);
        Assert.Equal(1, parentStarts);
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public async Task Parent_changes_propagate_through_nested_scope_dependencies()
    {
        var stopped = new List<string>();
        var values = new List<int>();
        await using var host = new PluginHost([
            Define("provider", context =>
            {
                context.Provide<IFirst>(new First(1));
                context.OnDispose(() => stopped.Add("provider"));
            }, provides: [typeof(IFirst)])
        ]);
        var child = await host.Application.CreateChildAsync("instance", [
            Define("middle", context =>
            {
                values.Add(context.GetRequiredService<IFirst>().Value);
                context.Provide<ISecond>(new Second());
                context.OnDispose(() => stopped.Add("middle"));
            }, requires: [typeof(IFirst)], provides: [typeof(ISecond)])
        ]);
        await child.CreateChildAsync("launch", [
            Define("consumer", context => context.OnDispose(() => stopped.Add("consumer")), requires: [typeof(ISecond)])
        ]);
        await host.StartAsync();
        await host.Application.RemovePluginAsync("provider");
        Assert.Equal(["consumer", "middle", "provider"], stopped);
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Waiting, plugin.State));
        await host.Application.AddPluginAsync(Define("provider", context => context.Provide<IFirst>(new First(2)), provides: [typeof(IFirst)]));
        Assert.Equal([1, 2], values);
        Assert.All(await host.GetSnapshotAsync(), plugin => Assert.Equal(PluginState.Active, plugin.State));
    }

    [Fact]
    public async Task Disposing_a_scope_only_releases_its_subtree_and_is_idempotent()
    {
        var stopped = new List<string>();
        await using var host = new PluginHost([Define("root", context => context.OnDispose(() => stopped.Add("root")))]);
        var child = await host.Application.CreateChildAsync("child", [Define("child", context => context.OnDispose(() => stopped.Add("child")))]);
        await child.CreateChildAsync("nested", [Define("nested", context => context.OnDispose(() => stopped.Add("nested")))]);
        var sibling = await host.Application.CreateChildAsync("sibling", [Define("sibling", context => context.OnDispose(() => stopped.Add("sibling")))]);
        await host.StartAsync();
        await Task.WhenAll(child.DisposeAsync().AsTask(), child.DisposeAsync().AsTask());
        Assert.Equal(2, stopped.Count);
        Assert.Contains("child", stopped);
        Assert.Contains("nested", stopped);
        Assert.Equal(2, (await host.GetSnapshotAsync()).Count);
        await Assert.ThrowsAsync<ObjectDisposedException>(() => child.AddPluginAsync(Define("late", _ => { })));
        await host.DisposeAsync();
        await sibling.DisposeAsync();
        Assert.Equal(4, stopped.Count);
        Assert.Empty(host.GetErrors());
    }

    [Fact]
    public async Task Invalid_child_composition_rolls_back_the_scope()
    {
        await using var host = new PluginHost([]);
        await host.StartAsync();
        await Assert.ThrowsAsync<PluginCompositionException>(() => host.Application.CreateChildAsync("instance", [
            Define("one", _ => { }, provides: [typeof(IFirst)]),
            Define("two", _ => { }, provides: [typeof(IFirst)])
        ]));
        var valid = await host.Application.CreateChildAsync("instance", [Define("healthy", _ => { })]);
        Assert.Single(await host.GetSnapshotAsync());
        await Assert.ThrowsAsync<PluginCompositionException>(() => host.Application.CreateChildAsync("instance"));
        await valid.DisposeAsync();
        await host.Application.CreateChildAsync("instance");
    }

    [Fact]
    public async Task Scoped_event_channels_do_not_cross_composition_scope_boundaries()
    {
        Context? parentPublisher = null;
        Context? childPublisher = null;
        var parentMessages = 0;
        var childMessages = 0;
        await using var host = new PluginHost([
            Define("publisher", context => parentPublisher = context),
            Define("listener", context => context.Subscribe<int>((_, _) => { parentMessages++; return ValueTask.CompletedTask; }))
        ]);
        await host.Application.CreateChildAsync("child", [
            Define("publisher", context => childPublisher = context),
            Define("listener", context => context.Subscribe<int>((_, _) => { childMessages++; return ValueTask.CompletedTask; }))
        ]);
        await host.StartAsync();
        await parentPublisher!.PublishAsync(1);
        Assert.Equal(1, parentMessages);
        Assert.Equal(0, childMessages);
        await childPublisher!.PublishAsync(1);
        Assert.Equal(1, parentMessages);
        Assert.Equal(1, childMessages);
    }

    [Fact]
    public async Task Independent_composition_commands_are_serialized()
    {
        var activations = 0;
        var cleanups = 0;
        await using var host = new PluginHost([]);
        await host.StartAsync();
        await Task.WhenAll(Enumerable.Range(0, 20).Select(index => host.Application.AddPluginAsync(
            Define($"plugin-{index}", context => { activations++; context.OnDispose(() => cleanups++); }))));
        Assert.Equal(20, activations);
        Assert.Equal(20, (await host.GetSnapshotAsync()).Count);
        await Task.WhenAll(Enumerable.Range(0, 20).Select(index => host.Application.RemovePluginAsync($"plugin-{index}")));
        Assert.Equal(20, cleanups);
        Assert.Empty(await host.GetSnapshotAsync());
    }
}
