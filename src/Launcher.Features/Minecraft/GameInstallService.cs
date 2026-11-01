using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Plugins.Minecraft;

public sealed class GameInstallService(Context context, IGameContentService content, IInstanceRepository instances,
    IPlatformEnvironment platform) : IGameInstallService
{
    private readonly object _sync = new();
    private readonly Dictionary<string, InstallSnapshot> _jobs = new(StringComparer.Ordinal);
    private readonly Dictionary<string, CancellationTokenSource> _cancellations = new(StringComparer.Ordinal);
    public IReadOnlyList<InstallSnapshot> Jobs { get { lock (_sync) return _jobs.Values
        .Where(job => job.Phase is not (InstallPhase.Completed or InstallPhase.Cancelled)).ToArray(); } }
    public InstallSnapshot? GetSnapshot(string instanceId) { lock (_sync) return _jobs.GetValueOrDefault(instanceId); }
    public void Clear()
    {
        lock (_sync) { if (_cancellations.Count != 0) return; _jobs.Clear(); }
        _ = context.NotifyAsync(new InstallStateChanged(""));
    }
    public void Cancel(string instanceId) { lock (_sync) if (_cancellations.TryGetValue(instanceId, out var source)) source.Cancel(); }
    public void Forget(string instanceId)
    {
        lock (_sync) { if (_cancellations.ContainsKey(instanceId)) return; _jobs.Remove(instanceId); }
        _ = context.NotifyAsync(new InstallStateChanged(instanceId));
    }

    public async Task InstallAsync(string instanceId, CancellationToken cancellationToken)
    {
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, context.Stopping);
        using var lease = instances.AcquireLaunch(instanceId);
        lock (_sync)
        {
            if (_cancellations.ContainsKey(instanceId)) throw new LauncherOperationException("此版本正在下载。");
            _cancellations.Add(instanceId, linked);
            _jobs[instanceId] = new(instanceId, lease.Instance.GameVersion, InstallPhase.Resolving) { InstanceName = lease.Instance.Name };
        }
        InstallSnapshot terminal;
        try
        {
            await context.NotifyAsync(new InstallStateChanged(instanceId)).ConfigureAwait(false);
            var metadata = await content.ResolveAsync(lease.Instance.GameVersion, linked.Token).ConfigureAwait(false);
            Set(new(instanceId, metadata.Id, InstallPhase.Downloading, new(0, null)), linked);
            await content.PrepareAsync(metadata, platform.Target, lease.DataRoot,
                new ProgressSink(value => Set(new(instanceId, metadata.Id, InstallPhase.Downloading, value), linked)), linked.Token).ConfigureAwait(false);
            linked.Token.ThrowIfCancellationRequested();
            await lease.MarkInstalledAsync(linked.Token).ConfigureAwait(false);
            terminal = new(instanceId, metadata.Id, InstallPhase.Completed, GetSnapshot(instanceId)?.Progress);
        }
        catch (OperationCanceledException) when (linked.IsCancellationRequested)
        { terminal = new(instanceId, lease.Instance.GameVersion, InstallPhase.Cancelled, GetSnapshot(instanceId)?.Progress); }
        catch (Exception error)
        {
            var message = error switch
            {
                GameLaunchException or LauncherOperationException => error.Message,
                HttpRequestException => "下载连接中断，请检查网络或切换下载源后重试。",
                IOException or UnauthorizedAccessException => "文件无法保存，请检查目录权限和剩余空间。",
                _ => "游戏文件准备失败，请重试。",
            };
            terminal = new(instanceId, lease.Instance.GameVersion, InstallPhase.Failed, GetSnapshot(instanceId)?.Progress, message);
        }
        // Release the instance and data-directory leases before publishing readiness to the UI.
        // Keep this atomic with Clear so a concurrent directory switch cannot restore an old job.
        lock (_sync)
        {
            lease.Dispose();
            _cancellations.Remove(instanceId);
            _jobs[instanceId] = terminal with { InstanceName = lease.Instance.Name };
        }
        _ = context.NotifyAsync(new InstallStateChanged(instanceId));
    }

    private void Set(InstallSnapshot snapshot, CancellationTokenSource attempt)
    {
        lock (_sync)
        {
            if (!_cancellations.TryGetValue(snapshot.InstanceId, out var current) || !ReferenceEquals(current, attempt)
                || attempt.IsCancellationRequested) return;
            _jobs[snapshot.InstanceId] = snapshot with { InstanceName = _jobs.GetValueOrDefault(snapshot.InstanceId)?.InstanceName };
        }
        _ = context.NotifyAsync(new InstallStateChanged(snapshot.InstanceId));
    }
    private sealed class ProgressSink(Action<LaunchProgress> report) : IProgress<LaunchProgress>
    { public void Report(LaunchProgress value) => report(value); }
}
