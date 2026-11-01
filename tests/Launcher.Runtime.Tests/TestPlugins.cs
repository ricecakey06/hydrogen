using Launcher.Runtime;

namespace Launcher.Runtime.Tests;

internal static class TestPlugins
{
    public static PluginDefinition Define(string id, Action<Context> activate, Type[]? requires = null, Type[]? provides = null)
        => new(id, (context, _) => { activate(context); return ValueTask.CompletedTask; }, requires, provides);

    public static TaskCompletionSource Signal() => new(TaskCreationOptions.RunContinuationsAsynchronously);
    public static Task Await(Task task) => task.WaitAsync(TimeSpan.FromSeconds(10));
}

internal interface IFirst { int Value { get; } }
internal interface ISecond;
internal interface IThird;
internal sealed class Second : ISecond;
internal sealed class Third : IThird;
internal sealed class First(int value = 1, Action? dispose = null) : IFirst, IDisposable
{
    public int Value { get; } = value;
    public bool IsDisposed { get; private set; }
    public void Dispose() { IsDisposed = true; dispose?.Invoke(); }
}

internal sealed class Release(Action release) : IDisposable
{
    public void Dispose() => release();
}
