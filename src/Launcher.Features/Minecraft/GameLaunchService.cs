using System.ComponentModel;
using System.Diagnostics;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.Plugins.Java;
using Launcher.Plugins.Storage;
using Launcher.Runtime;

namespace Launcher.Plugins.Minecraft;

public sealed class GameLaunchService(Context context, IGameAccountService account, IGameContentService content,
    IJavaRuntimeService java, ILauncherSettingsService settings, IInstanceRepository instances, IPlatformEnvironment platform,
    IProcessService processes, IDiagnosticsService diagnostics) : IGameLaunchService
{
    private readonly object _sync = new();
    private readonly Dictionary<string, LaunchSnapshot> _snapshots = new(StringComparer.Ordinal);

    public LaunchSnapshot GetSnapshot(string instanceId)
    { lock (_sync) return _snapshots.GetValueOrDefault(instanceId) ?? new(instanceId, LaunchPhase.Idle); }
    public IReadOnlyList<LaunchSnapshot> ActiveSessions
    { get { lock (_sync) return _snapshots.Values.Where(snapshot => snapshot.IsActive).ToArray(); } }

    public async Task LaunchAsync(string instanceId, CancellationToken cancellationToken)
    {
        lock (_sync)
        {
            if (GetSnapshot(instanceId).IsActive) throw new LauncherOperationException("此版本已经在启动或运行。");
            _snapshots[instanceId] = new(instanceId, LaunchPhase.CheckingEnvironment);
        }
        await context.NotifyAsync(new LaunchStateChanged(GetSnapshot(instanceId))).ConfigureAwait(false);
        LifetimeScope? scope = null;
        ILauncherLog? log = null;
        var handedOff = false;
        LaunchSnapshot? terminal = null;
        try
        {
            scope = context.CreateScope("launch-" + Guid.NewGuid().ToString("N"));
            using var linked = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, context.Stopping, scope.Stopping);
            var token = linked.Token;
            token.ThrowIfCancellationRequested();
            IInstanceLease lease;
            try { lease = scope.Own(instances.AcquireLaunch(instanceId)); }
            catch (IOException) { throw new GameLaunchException(LaunchFailureKind.InstanceInUse, "实例正在由另一个启动器使用，或目录无法访问。"); }
            var defaults = settings.Settings;
            var overrides = lease.Overrides;
            var effective = defaults with
            {
                Java = overrides.Java ?? defaults.Java,
                MaxMemoryMiB = overrides.MaxMemoryMiB ?? defaults.MaxMemoryMiB,
                JvmArguments = overrides.JvmArguments ?? defaults.JvmArguments,
            };
            var issues = SettingsValidation.Check(effective.MaxMemoryMiB, effective.JvmArguments, effective.Java);
            if (issues.Count > 0) throw new GameLaunchException(LaunchFailureKind.InvalidSettings, issues[0].Message);
            var gameDirectory = instances.GetGameDirectory(instanceId);
            Directory.CreateDirectory(gameDirectory);
            var probe = JsonFiles.Within(gameDirectory, ".write-check-" + Guid.NewGuid().ToString("N"));
            using (var writable = new FileStream(probe, FileMode.CreateNew, FileAccess.Write, FileShare.None, 1, FileOptions.DeleteOnClose)) writable.Flush();
            log = scope.Own(await diagnostics.CreateLogAsync("game", token).ConfigureAwait(false));
            await log.WriteAsync($"Preparing official Minecraft {lease.Instance.GameVersion}.", token).ConfigureAwait(false);
            var identity = await account.GetSessionAsync(token).ConfigureAwait(false);
            if (identity.Mode == AccountLoginMode.Microsoft) log.AddSecrets(identity.AccessToken, identity.Xuid);
            else await log.WriteAsync("Using offline mode.", token).ConfigureAwait(false);
            var metadata = await content.ResolveAsync(lease.Instance.GameVersion, token).ConfigureAwait(false);
            var runtime = await SelectJavaAsync(effective.Java, metadata.JavaMajorVersion, token).ConfigureAwait(false);
            if (runtime.Architecture == "x86" && effective.MaxMemoryMiB > 1400)
                throw new GameLaunchException(LaunchFailureKind.JavaIncompatible, "32 位 Java 的堆内存不能超过 1400 MB，请使用 64 位 Java。");
            var target = platform.Target with { Architecture = runtime.Architecture };
            await log.WriteAsync($"Java {runtime.Version}; {target.OperatingSystem}/{target.Architecture}; heap {effective.MaxMemoryMiB} MiB.", token).ConfigureAwait(false);
            await SetAsync(new(instanceId, LaunchPhase.PreparingFiles, new(0, null))).ConfigureAwait(false);
            var progress = new ProgressSink(value => ReportProgress(instanceId, value));
            var prepared = await content.PrepareAsync(metadata, target, lease.DataRoot, progress, token).ConfigureAwait(false);
            var relative = Path.Combine("versions", lease.Instance.GameVersion, "natives", target.OperatingSystem + "-" + target.Architecture, Guid.NewGuid().ToString("N"));
            var sessionDirectory = JsonFiles.Within(lease.DataRoot, relative);
            var processCreated = false;
            scope.OnDispose(() =>
            {
                // A deliberate host exit detaches the child; it must retain native files until that child exits.
                if (processCreated && context.Stopping.IsCancellationRequested) return;
                var owned = JsonFiles.Within(lease.DataRoot, relative);
                if (Directory.Exists(owned)) Directory.Delete(owned, recursive: true);
            });
            await LaunchPlanBuilder.ExtractNativesAsync(prepared.Natives, sessionDirectory, token).ConfigureAwait(false);
            var plan = LaunchPlanBuilder.Build(prepared, runtime, identity, effective, target, gameDirectory, sessionDirectory);
            await SetAsync(new(instanceId, LaunchPhase.Starting)).ConfigureAwait(false);
            token.ThrowIfCancellationRequested();
            var process = scope.Own(processes.Start(plan, output => log.WriteAsync((output.IsError ? "[stderr] " : "[stdout] ") + output.Text)));
            processCreated = true;
            try { lease.MarkRunning(process.Id); }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException) { await TryLogAsync(log, "Could not persist the process marker.").ConfigureAwait(false); }
            // From process creation onward, user cancellation never terminates the game.
            await TryLogAsync(log, $"Game process created (PID {process.Id}).").ConfigureAwait(false);
            await SetAsync(new(instanceId, LaunchPhase.Running, AfterLaunch: effective.AfterLaunch)).ConfigureAwait(false);
            var ownedScope = scope;
            _ = context.Run(stopping => MonitorAsync(instanceId, process, ownedScope, log, stopping));
            handedOff = true;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested || context.Stopping.IsCancellationRequested || scope?.Stopping.IsCancellationRequested == true)
        { terminal = new(instanceId, LaunchPhase.Cancelled); }
        catch (Exception error)
        {
            var failure = Describe(error, log?.Path);
            if (log is not null) await TryLogAsync(log, $"Launch failed: {failure.Kind} ({error.GetType().Name}).").ConfigureAwait(false);
            terminal = new(instanceId, LaunchPhase.Failed, Failure: failure);
        }
        finally
        {
            if (!handedOff && scope is not null) await scope.DisposeAsync().ConfigureAwait(false);
        }
        if (terminal is not null) await SetAsync(terminal).ConfigureAwait(false);
    }

    private async Task<JavaRuntimeInfo> SelectJavaAsync(JavaChoice choice, int required, CancellationToken token)
    {
        JavaRuntimeInfo? selected;
        if (choice.ExecutablePath is { } path)
        {
            selected = await java.InspectAsync(path, token).ConfigureAwait(false);
            if (selected is null) throw new GameLaunchException(LaunchFailureKind.JavaMissing, $"需要 Java {required}，请重新选择可执行文件。");
        }
        else
        {
            var candidates = java.KnownRuntimes;
            if (!candidates.Any(candidate => JavaRuntimeService.MajorVersion(candidate.Version) == required)) candidates = await java.ScanAsync(token).ConfigureAwait(false);
            var match = candidates.Where(candidate => JavaRuntimeService.MajorVersion(candidate.Version) == required)
                .OrderByDescending(candidate => candidate.Architecture == platform.Target.Architecture).ThenByDescending(candidate => candidate.Architecture != "x86").FirstOrDefault();
            selected = match is null ? null : await java.InspectAsync(match.ExecutablePath, token).ConfigureAwait(false);
            if (selected is null) throw new GameLaunchException(LaunchFailureKind.JavaMissing, $"需要 Java {required}，请在设置中扫描或手动选择。");
        }
        if (JavaRuntimeService.MajorVersion(selected.Version) != required)
            throw new GameLaunchException(LaunchFailureKind.JavaIncompatible, $"此版本需要 Java {required}，当前为 Java {JavaRuntimeService.MajorVersion(selected.Version)}。");
        return selected;
    }

    private async Task MonitorAsync(string instanceId, IRunningProcess process, LifetimeScope scope, ILauncherLog log, CancellationToken stopping)
    {
        LaunchSnapshot? terminal = null;
        try
        {
            var code = await process.WaitForExitAsync(stopping).ConfigureAwait(false);
            await TryLogAsync(log, $"Game process exited with code {code}.").ConfigureAwait(false);
            terminal = code == 0 ? new(instanceId, LaunchPhase.Exited, ExitCode: code)
                : new(instanceId, LaunchPhase.Failed, Failure: new(LaunchFailureKind.ProcessExitedEarly, $"退出码 {code}", log.Path), ExitCode: code);
        }
        catch (OperationCanceledException) when (stopping.IsCancellationRequested) { }
        catch (Exception error)
        {
            await TryLogAsync(log, $"Process monitoring failed ({error.GetType().Name}).").ConfigureAwait(false);
            terminal = new(instanceId, LaunchPhase.Failed, Failure: new(LaunchFailureKind.Unexpected, LogPath: log.Path));
        }
        finally { await scope.DisposeAsync().ConfigureAwait(false); }
        if (terminal is not null) await SetAsync(terminal).ConfigureAwait(false);
    }

    private void ReportProgress(string id, LaunchProgress progress)
    {
        LaunchSnapshot snapshot;
        lock (_sync)
        {
            var previous = GetSnapshot(id);
            if (previous.Phase != LaunchPhase.PreparingFiles) return;
            progress = progress with { Completed = Math.Max(previous.Progress?.Completed ?? 0, progress.Completed) };
            snapshot = previous with { Progress = progress };
            _snapshots[id] = snapshot;
        }
        try { _ = context.Run(_ => context.NotifyAsync(new LaunchStateChanged(snapshot))); }
        catch (ObjectDisposedException) { }
    }
    private Task SetAsync(LaunchSnapshot snapshot)
    { lock (_sync) _snapshots[snapshot.InstanceId] = snapshot; return context.NotifyAsync(new LaunchStateChanged(snapshot)); }
    private static async Task TryLogAsync(ILauncherLog log, string message)
    { try { await log.WriteAsync(message).ConfigureAwait(false); } catch (Exception error) when (error is IOException or UnauthorizedAccessException or ObjectDisposedException) { } }
    private static LaunchFailure Describe(Exception error, string? log) => error switch
    {
        GameLaunchException failure => new(failure.Kind, failure.Message, log),
        AccountAuthenticationException auth => new(auth.Kind == AccountFailureKind.Network ? LaunchFailureKind.Network : LaunchFailureKind.SignInRequired, LogPath: log),
        HttpRequestException or OperationCanceledException => new(LaunchFailureKind.Network, LogPath: log),
        InvalidDataException or JsonException => new(LaunchFailureKind.FileVerification, LogPath: log),
        IOException or UnauthorizedAccessException => new(LaunchFailureKind.DirectoryNotWritable, LogPath: log),
        Win32Exception => new(LaunchFailureKind.JavaIncompatible, "Java 进程无法创建，请重新验证运行时。", log),
        LauncherOperationException problem => new(LaunchFailureKind.InvalidSettings, problem.Message, log),
        _ => new(LaunchFailureKind.Unexpected, LogPath: log),
    };
    private sealed class ProgressSink(Action<LaunchProgress> report) : IProgress<LaunchProgress>
    {
        private readonly object _sync = new();
        private long _last;
        public void Report(LaunchProgress value)
        {
            lock (_sync)
            {
                var now = Stopwatch.GetTimestamp();
                if (_last != 0 && value.Completed != value.Total && Stopwatch.GetElapsedTime(_last, now) < TimeSpan.FromMilliseconds(100)) return;
                _last = now;
                report(value);
            }
        }
    }
}
