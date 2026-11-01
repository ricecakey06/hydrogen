using Launcher.Contracts;
using Launcher.Plugins.Infrastructure;
using Launcher.Plugins.Minecraft;
using Launcher.Plugins.Storage;

namespace Launcher.Launch.Tests;

public sealed class LaunchTests
{
    [Fact]
    public async Task Running_game_ignores_user_cancellation_keeps_locks_and_releases_resources_after_exit()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var processes = new StubProcesses();
        var launch = Create(test, processes);
        using var cancellation = new CancellationTokenSource();
        var exited = ObserveTerminal(test, instance.Id);
        await launch.LaunchAsync(instance.Id, cancellation.Token);
        Assert.Equal(LaunchPhase.Running, launch.GetSnapshot(instance.Id).Phase);
        await cancellation.CancelAsync();
        Assert.False(processes.Process!.WasTerminated);
        Assert.Single(launch.ActiveSessions);
        await Assert.ThrowsAsync<LauncherOperationException>(() => launch.LaunchAsync(instance.Id, default));
        Assert.NotEmpty(await test.Storage.ChangeDataDirectoryAsync(Path.Combine(Path.GetDirectoryName(test.Root)!, "next"), default));
        await processes.Output!(new ProcessOutput("--accessToken " + StubAccount.Session.AccessToken, false));
        processes.Process.Exit.SetResult(0);
        Assert.Equal(LaunchPhase.Exited, (await exited.WaitAsync(TimeSpan.FromSeconds(5))).Phase);
        Assert.Empty(launch.ActiveSessions);
        Assert.True(processes.Process.WasDisposed);
        Assert.Empty(await test.Instances.SaveOverridesAsync(instance.Id, new(null, 1024, null), default));
        var log = await File.ReadAllTextAsync(Assert.Single(Directory.GetFiles(Path.Combine(test.Root, "logs"))));
        Assert.DoesNotContain(StubAccount.Session.AccessToken, log);
        Assert.Contains("[redacted]", log);
        Assert.Empty(Directory.EnumerateDirectories(Path.Combine(test.Storage.GameRoot, "versions", instance.GameVersion, "natives", "windows-x64")));
        Assert.Equal(test.Instances.GetGameDirectory(instance.Id), processes.LastRequest!.WorkingDirectory);
        Assert.Empty(test.Host.GetErrors());
    }

    [Fact]
    public async Task Cancelling_preparation_leaves_verified_data_and_unlocks_the_instance()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var content = new StubContent { OnPrepare = async token => { entered.SetResult(); await Task.Delay(Timeout.Infinite, token); } };
        var processes = new StubProcesses();
        var launch = Create(test, processes, content);
        using var cancellation = new CancellationTokenSource();
        var task = launch.LaunchAsync(instance.Id, cancellation.Token);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await cancellation.CancelAsync();
        await task;
        Assert.Equal(LaunchPhase.Cancelled, launch.GetSnapshot(instance.Id).Phase);
        Assert.Null(processes.LastRequest);
        Assert.Empty(await test.Instances.SaveOverridesAsync(instance.Id, new(null, 1024, null), default));
        Assert.True(File.Exists(Path.Combine(test.Root, "instances", instance.Id, "instance.json")));
        Assert.Empty(test.Host.GetErrors());
    }

    [Fact]
    public async Task Incompatible_manual_java_prevents_process_creation_and_reports_the_required_version()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var java = new StubJava { Runtime = new(Path.Combine(test.Root, "java"), "21.0.8", "x64") };
        Assert.Empty(await test.Instances.SaveOverridesAsync(instance.Id, new(new(java.Runtime.ExecutablePath), null, null), default));
        var processes = new StubProcesses();
        var launch = Create(test, processes, java: java);
        await launch.LaunchAsync(instance.Id, default);
        Assert.Equal(LaunchFailureKind.JavaIncompatible, launch.GetSnapshot(instance.Id).Failure!.Kind);
        Assert.Contains("25", launch.GetSnapshot(instance.Id).Failure!.Detail);
        Assert.Null(processes.LastRequest);
    }

    [Fact]
    public async Task Nonzero_game_exit_exposes_a_redacted_log_and_keeps_game_files()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var processes = new StubProcesses();
        var launch = Create(test, processes);
        var terminal = ObserveTerminal(test, instance.Id);
        await launch.LaunchAsync(instance.Id, default);
        processes.Process!.Exit.SetResult(42);
        var snapshot = await terminal.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Equal(LaunchPhase.Failed, snapshot.Phase);
        Assert.Equal(LaunchFailureKind.ProcessExitedEarly, snapshot.Failure!.Kind);
        Assert.Equal(42, snapshot.ExitCode);
        Assert.True(File.Exists(snapshot.Failure.LogPath));
        Assert.True(Directory.Exists(test.Instances.GetGameDirectory(instance.Id)));
    }

    [Fact]
    public async Task Shutdown_detaches_the_game_without_terminating_it()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var processes = new StubProcesses();
        await Create(test, processes).LaunchAsync(instance.Id, default);
        await test.Host.DisposeAsync();
        Assert.False(processes.Process!.WasTerminated);
        Assert.True(processes.Process.WasDisposed);
        Assert.Empty(test.Host.GetErrors());
    }

    [Fact]
    public async Task Launch_settings_are_snapshots_and_later_changes_only_affect_the_next_launch()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var processes = new StubProcesses();
        var launch = Create(test, processes);
        Assert.Empty(await test.Storage.SaveAsync(LauncherStorage.Defaults with { MaxMemoryMiB = 1024 }, default));
        await launch.LaunchAsync(instance.Id, default);
        Assert.Empty(await test.Storage.SaveAsync(test.Storage.Settings with { MaxMemoryMiB = 1536, AfterLaunch = AfterLaunchBehavior.KeepOpen }, default));
        Assert.Contains("-Xmx1024M", processes.LastRequest!.Arguments);
        Assert.Equal(AfterLaunchBehavior.MinimizeAndRestore, launch.GetSnapshot(instance.Id).AfterLaunch);
    }

    private static GameLaunchService Create(TestScope test, StubProcesses processes, StubContent? content = null, StubJava? java = null)
        => new(test.Context, new StubAccount(), content ?? new StubContent(), java ?? new StubJava(), test.Storage, test.Instances,
            new StubPlatform(), processes, new DiagnosticsService(test.Storage));
    private static Task<LaunchSnapshot> ObserveTerminal(TestScope test, string id)
    {
        var result = new TaskCompletionSource<LaunchSnapshot>(TaskCreationOptions.RunContinuationsAsynchronously);
        test.Context.Subscribe<LaunchStateChanged>((change, _) =>
        {
            if (change.Snapshot.InstanceId == id && change.Snapshot.Phase is LaunchPhase.Exited or LaunchPhase.Failed or LaunchPhase.Cancelled) result.TrySetResult(change.Snapshot);
            return ValueTask.CompletedTask;
        });
        return result.Task;
    }
}
