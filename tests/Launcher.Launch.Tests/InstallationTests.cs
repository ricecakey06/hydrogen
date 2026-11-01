using Launcher.Contracts;
using Launcher.Plugins.Minecraft;

namespace Launcher.Launch.Tests;

public sealed class InstallationTests
{
    [Fact]
    public async Task Installation_requires_no_account_or_java_and_holds_the_version_and_data_root_until_complete()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("1.21.1", default);
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var finish = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var content = new StubContent { OnPrepare = async token => { entered.SetResult(); await finish.Task.WaitAsync(token); } };
        var service = new GameInstallService(test.Context, content, test.Instances, new StubPlatform());
        var download = service.InstallAsync(instance.Id, default);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.True(service.GetSnapshot(instance.Id)!.IsActive);
        Assert.False(Assert.Single(test.Instances.Instances).IsInstalled);
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.SelectAsync(instance.Id, default));
        await using (var restarted = await TestScope.CreateAsync(test.Root))
        {
            Assert.False(Assert.Single(restarted.Instances.Instances).IsInstalled);
            Assert.Null(restarted.Instances.SelectedInstanceId);
        }
        Assert.Throws<LauncherOperationException>(() => test.Instances.AcquireLaunch(instance.Id));
        await Assert.ThrowsAsync<LauncherOperationException>(() => test.Instances.DeleteAsync(instance.Id, default));
        Assert.NotEmpty(await test.Storage.ChangeDataDirectoryAsync(Path.Combine(test.Root, "other-root"), default));
        finish.SetResult();
        await download;
        Assert.Equal(InstallPhase.Completed, service.GetSnapshot(instance.Id)!.Phase);
        Assert.Equal(10, service.GetSnapshot(instance.Id)!.Progress!.Completed);
        content.LastProgress!.Report(new(1, 10));
        Assert.Equal(InstallPhase.Completed, service.GetSnapshot(instance.Id)!.Phase);
        Assert.Empty(service.Jobs);
        Assert.True(Assert.Single(test.Instances.Instances).IsInstalled);
        Assert.Equal(instance.Id, test.Instances.SelectedInstanceId);
        using var unlocked = test.Instances.AcquireLaunch(instance.Id);
        Assert.Empty(test.Host.GetErrors());
    }

    [Fact]
    public async Task Cancellation_and_retry_reuse_the_same_instance_and_publish_terminal_state_after_unlocking()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var content = new StubContent { OnPrepare = async token => { entered.SetResult(); await Task.Delay(Timeout.Infinite, token); } };
        var service = new GameInstallService(test.Context, content, test.Instances, new StubPlatform());
        var download = service.InstallAsync(instance.Id, default);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        service.Cancel(instance.Id);
        await download;
        Assert.Equal(InstallPhase.Cancelled, service.GetSnapshot(instance.Id)!.Phase);
        content.LastProgress!.Report(new(1, 10));
        Assert.Equal(InstallPhase.Cancelled, service.GetSnapshot(instance.Id)!.Phase);
        Assert.Empty(service.Jobs);
        Assert.False(Assert.Single(test.Instances.Instances).IsInstalled);
        Assert.Equal(instance.Id, (await test.Instances.CreateAsync("26.1.2", default)).Id);
        content.OnPrepare = _ => Task.FromException(new GameLaunchException(LaunchFailureKind.DownloadsDisabled, "downloads disabled"));
        await service.InstallAsync(instance.Id, default);
        Assert.Equal(InstallPhase.Failed, service.GetSnapshot(instance.Id)!.Phase);
        Assert.Equal("downloads disabled", service.GetSnapshot(instance.Id)!.Error);
        Assert.Equal(InstallPhase.Failed, Assert.Single(service.Jobs).Phase);
        Assert.False(Assert.Single(test.Instances.Instances).IsInstalled);
        content.OnPrepare = null;
        await service.InstallAsync(instance.Id, default);
        Assert.Equal(InstallPhase.Completed, service.GetSnapshot(instance.Id)!.Phase);
        Assert.Empty(service.Jobs);
        Assert.True(Assert.Single(test.Instances.Instances).IsInstalled);
        using var unlocked = test.Instances.AcquireLaunch(instance.Id);
    }
}
