using Avalonia.Media;
using Launcher.Contracts;

namespace Launcher.UI.ViewModels;

/// <summary>
/// The home page (DESIGN.md 9.3). Signed-out, missing-version, launch-progress and failure states reuse the account,
/// version and launch areas instead of adding new regions.
/// </summary>
internal sealed class HomeViewModel : ViewModelBase, IDisposable
{
    private readonly LauncherViewModel _root;
    private CancellationTokenSource? _signIn;
    private CancellationTokenSource? _launch;
    private SkinFace? _face;
    private byte[]? _faceSource;
    private string? _instanceId;
    private string? _accountMessage;
    private string? _launchMessage;
    private string? _downloadMessage;
    private LaunchFailure? _failure;
    private string? _modeError;

    public HomeViewModel(LauncherViewModel root)
    {
        _root = root;
        RefreshAccount();
        RefreshInstance();
    }

    public bool IsSignedIn { get; private set => SetProperty(ref field, value); }
    public bool IsSigningIn { get; private set => SetProperty(ref field, value); }
    public bool CanSignIn { get; private set => SetProperty(ref field, value); }
    public string SignInLabel { get; private set => SetProperty(ref field, value); } = "登录微软账号";
    public string PlayerName { get; private set => SetProperty(ref field, value); } = "";
    public IImage? Face { get; private set => SetProperty(ref field, value); }
    public IImage? FaceHat { get; private set => SetProperty(ref field, value); }
    public bool HasFace { get; private set => SetProperty(ref field, value); }
    public bool HasAccountModes { get; private set => SetProperty(ref field, value); }
    public bool IsMicrosoftMode { get; private set => SetProperty(ref field, value); }
    public bool IsOfflineMode { get; private set => SetProperty(ref field, value); }
    public bool CanSwitchAccountMode { get; private set => SetProperty(ref field, value); }
    public bool IsSwitchingAccountMode { get; private set => SetProperty(ref field, value); }

    public bool HasInstance { get; private set => SetProperty(ref field, value); }
    public string VersionText { get; private set => SetProperty(ref field, value); } = "";

    public bool IsLaunchBusy { get; private set => SetProperty(ref field, value); }
    public bool IsLaunchEnabled { get; private set => SetProperty(ref field, value); }
    public string LaunchActionName { get; private set => SetProperty(ref field, value); } = "启动游戏";
    public bool IsProgressIndeterminate { get; private set => SetProperty(ref field, value); }
    public bool IsProgressDeterminate { get; private set => SetProperty(ref field, value); }
    public double ProgressSweep { get; private set => SetProperty(ref field, value); }

    public string? StatusText { get; private set => SetProperty(ref field, value); }
    public bool HasStatus { get; private set => SetProperty(ref field, value); }
    public bool IsStatusError { get; private set => SetProperty(ref field, value); }
    public bool CanOpenLog { get; private set => SetProperty(ref field, value); }
    public bool HasDownloads { get; private set => SetProperty(ref field, value); }
    public bool IsDownloadIndeterminate { get; private set => SetProperty(ref field, value); }
    public bool IsDownloadDeterminate { get; private set => SetProperty(ref field, value); }
    public double DownloadSweep { get; private set => SetProperty(ref field, value); }
    public string DownloadActionName { get; private set => SetProperty(ref field, value); } = "下载管理";
    public void ShowDownloads() => _root.ShowDownloads();

    public void RefreshDownloads()
    {
        HasDownloads = _root.Services.Install.Jobs.Count > 0;
        IsDownloadIndeterminate = _root.Downloads.IsIndeterminate;
        IsDownloadDeterminate = HasDownloads && !IsDownloadIndeterminate;
        DownloadSweep = _root.Downloads.OverallPercent * 3.6;
        DownloadActionName = "查看下载进度 · " + _root.Downloads.OverallText;
        var selected = _instanceId is null ? null : _root.Services.Install.GetSnapshot(_instanceId);
        _downloadMessage = selected?.IsActive == true ? "游戏文件正在下载，可点击右上角查看进度" : null;
        UpdateLaunchAvailability();
        UpdateStatus();
    }

    public void ShowVersions() => _root.ShowVersions();
    public void ShowInstanceSettings() => _root.ShowInstanceSettings();
    public void ShowLauncherSettings() => _root.ShowLauncherSettings();

    public void SignIn()
    {
        if (!CanSignIn || _signIn is not null) return;
        _modeError = null;
        _signIn = StartCancellable(async token =>
        {
            try { await _root.Services.Account.SignInAsync(token); }
            catch (LauncherOperationException error) { _modeError = error.Message; UpdateStatus(); }
        }, () => _signIn = null);
    }

    public void SelectMicrosoft() => SelectAccountMode(AccountLoginMode.Microsoft);
    public void SelectOffline() => SelectAccountMode(AccountLoginMode.Offline);

    private void SelectAccountMode(AccountLoginMode mode)
    {
        if (!CanChangeAccountMode()) return;
        _modeError = null;
        IsSwitchingAccountMode = true;
        UpdateModeAvailability();
        UpdateLaunchAvailability();
        var accepted = _root.Operations.Start(async token =>
        {
            try { await _root.Services.AccountModes.SelectAsync(mode, token); }
            catch (LauncherOperationException error) { _modeError = error.Message; }
            finally { IsSwitchingAccountMode = false; _root.RefreshAccount(); }
        });
        if (!accepted) { IsSwitchingAccountMode = false; UpdateModeAvailability(); }
    }

    private bool CanChangeAccountMode() => !IsSwitchingAccountMode && _root.Services.Account.State.Status != AccountStatus.SigningIn
        && !_root.Services.Launch.ActiveSessions.Any(session => session.Phase is LaunchPhase.CheckingEnvironment or LaunchPhase.PreparingFiles or LaunchPhase.Starting);
    private void UpdateModeAvailability() => CanSwitchAccountMode = HasAccountModes && CanChangeAccountMode();

    public void CancelSignIn() => _signIn?.Cancel();

    /// <summary>The round button launches, cancels preparation while busy, and retries after a failure.</summary>
    public void LaunchOrCancel()
    {
        if (IsLaunchBusy)
        {
            _launch?.Cancel();
            return;
        }
        if (!IsLaunchEnabled || _instanceId is not { } instanceId || _launch is not null) return;
        _launch = StartCancellable(token => _root.Services.Launch.LaunchAsync(instanceId, token), () => _launch = null);
    }

    public void OpenLog()
    {
        if (_failure?.LogPath is not { } path) return;
        _root.Operations.Start(async _ => await _root.Shell.SystemLauncher.LaunchUriAsync(new Uri(Path.GetFullPath(path))));
    }

    public void RefreshAccount()
    {
        var state = _root.Services.Account.State;
        var modes = _root.Services.AccountModes.Snapshot;
        HasAccountModes = modes.OfflineAvailable;
        IsMicrosoftMode = modes.Mode == AccountLoginMode.Microsoft;
        IsOfflineMode = modes.Mode == AccountLoginMode.Offline;
        IsSignedIn = state.Status == AccountStatus.SignedIn;
        IsSigningIn = state.Status == AccountStatus.SigningIn;
        CanSignIn = state.Status is AccountStatus.SignedOut or AccountStatus.Expired;
        SignInLabel = IsOfflineMode ? "离线登录" : state.Status == AccountStatus.Expired ? "重新登录" : "登录微软账号";
        PlayerName = state.Profile?.PlayerName ?? "";
        UpdateFace(IsMicrosoftMode && IsSignedIn ? state.Profile?.SkinPng : null);
        _accountMessage = IsOfflineMode ? state.Detail ?? (state.Status == AccountStatus.SignedOut ? "离线账号未登录" : null) : state.Status switch
        {
            AccountStatus.Expired => state.LastFailure switch
            {
                AccountFailureKind.Network => "无法刷新登录，请检查网络后重试",
                AccountFailureKind.AccountRestricted => "该账号受到限制，无法进入游戏",
                AccountFailureKind.NoGameOwnership => "该账号没有有效的 Minecraft: Java 版资格",
                AccountFailureKind.Configuration => "微软登录配置已更改，请重新配置后登录",
                AccountFailureKind.AppNotAuthorized => "此启动器的 Minecraft 登录接入尚未获授权",
                _ => "登录已失效，请重新登录",
            },
            AccountStatus.SignedOut => state.LastFailure switch
            {
                AccountFailureKind.Cancelled => "已取消登录",
                AccountFailureKind.Network => "网络连接失败，请稍后重试",
                AccountFailureKind.AccountRestricted => "该账号受到限制，无法进入游戏",
                AccountFailureKind.NoGameOwnership => "该账号未拥有 Minecraft: Java 版",
                AccountFailureKind.Unexpected => "登录失败，请重试",
                AccountFailureKind.Configuration => "微软登录尚未配置，请检查账号服务配置",
                AccountFailureKind.AppNotAuthorized => "此启动器的 Minecraft 登录接入尚未获授权",
                _ => null,
            },
            AccountStatus.SignedIn => state.Detail,
            _ => null,
        };
        if (HasAccountModes && modes.Problem is not null) _modeError = modes.Problem;
        UpdateModeAvailability();
        UpdateLaunchAvailability();
        UpdateStatus();
    }

    public void RefreshInstance()
    {
        var services = _root.Services.Instances;
        var selectedId = services.SelectedInstanceId;
        var instance = services.Instances.FirstOrDefault(candidate => candidate.Id == selectedId && candidate.IsInstalled);
        _instanceId = instance?.Id;
        HasInstance = instance is not null;
        VersionText = instance is null ? "未选择版本" : instance.Name == instance.GameVersion
            ? instance.GameVersion : $"{instance.Name} · {instance.GameVersion}";
        RefreshLaunch();
        RefreshDownloads();
    }

    public void RefreshLaunch()
    {
        var snapshot = _instanceId is null ? null : _root.Services.Launch.GetSnapshot(_instanceId);
        var phase = snapshot?.Phase ?? LaunchPhase.Idle;
        IsLaunchBusy = phase is LaunchPhase.CheckingEnvironment or LaunchPhase.PreparingFiles or LaunchPhase.Starting;
        _failure = phase == LaunchPhase.Failed ? snapshot!.Failure : null;
        LaunchActionName = IsLaunchBusy ? "取消启动" : phase == LaunchPhase.Failed ? "重试启动" : "启动游戏";

        // Show a percentage only when the amount of work is known (DESIGN.md 8.2).
        var fraction = phase == LaunchPhase.PreparingFiles && snapshot!.Progress is { Total: > 0 } progress
            ? Math.Clamp((double)progress.Completed / progress.Total.Value, 0, 1)
            : (double?)null;
        IsProgressDeterminate = IsLaunchBusy && fraction is not null;
        IsProgressIndeterminate = IsLaunchBusy && fraction is null;
        ProgressSweep = (fraction ?? 0) * 360;

        _launchMessage = phase switch
        {
            LaunchPhase.CheckingEnvironment => "正在检查环境",
            LaunchPhase.PreparingFiles => fraction is { } value ? $"正在准备游戏文件 {value:P0}" : "正在准备游戏文件",
            LaunchPhase.Starting => "正在启动",
            LaunchPhase.Running => "游戏进程运行中",
            LaunchPhase.Exited => snapshot!.ExitCode is null or 0 ? "游戏已退出" : $"游戏已退出（退出码 {snapshot.ExitCode}）",
            LaunchPhase.Cancelled => "已取消",
            LaunchPhase.Failed => DescribeFailure(_failure),
            _ => null,
        };
        CanOpenLog = _failure?.LogPath is not null;
        UpdateModeAvailability();
        UpdateLaunchAvailability(phase);
        UpdateStatus();
    }

    public void Dispose()
    {
        _face?.Dispose();
        _face = null;
    }

    // Returns the source that cancels the operation, or null when the plugin no longer accepts work.
    private CancellationTokenSource? StartCancellable(Func<CancellationToken, Task> operation, Action completed)
    {
        var user = new CancellationTokenSource();
        var accepted = _root.Operations.Start(async stopping =>
        {
            using var linked = CancellationTokenSource.CreateLinkedTokenSource(user.Token, stopping);
            try { await operation(linked.Token); }
            catch (OperationCanceledException) when (linked.IsCancellationRequested) { }
            finally
            {
                completed();
                user.Dispose();
            }
        });
        if (accepted) return user;
        user.Dispose();
        return null;
    }

    private void UpdateLaunchAvailability(LaunchPhase? phase = null)
    {
        phase ??= _instanceId is null ? LaunchPhase.Idle : _root.Services.Launch.GetSnapshot(_instanceId).Phase;
        // Busy launches stay clickable so they can be cancelled; a running game is never launched twice.
        var installing = _instanceId is not null && _root.Services.Install.GetSnapshot(_instanceId)?.IsActive == true;
        IsLaunchEnabled = IsLaunchBusy || (!installing && !IsSwitchingAccountMode && IsSignedIn && HasInstance && phase != LaunchPhase.Running);
    }

    private void UpdateStatus()
    {
        StatusText = _modeError ?? _downloadMessage ?? _launchMessage ?? _accountMessage;
        HasStatus = StatusText is not null;
        IsStatusError = _modeError is not null || _failure is not null
            || (!IsSignedIn && _downloadMessage is null && _launchMessage is null && _accountMessage is not null);
    }

    private void UpdateFace(byte[]? skinPng)
    {
        if (ReferenceEquals(skinPng, _faceSource)) return;
        _face?.Dispose();
        _faceSource = skinPng;
        _face = skinPng is null ? null : SkinFace.TryCreate(skinPng);
        Face = _face?.Face;
        FaceHat = _face?.Hat;
        HasFace = _face is not null;
    }

    private static string DescribeFailure(LaunchFailure? failure)
    {
        var message = failure?.Kind switch
        {
            LaunchFailureKind.SignInRequired => "登录已失效，请重新登录",
            LaunchFailureKind.JavaMissing => "未找到兼容的 Java",
            LaunchFailureKind.JavaIncompatible => "所选 Java 与此版本不兼容",
            LaunchFailureKind.Network => "网络连接中断",
            LaunchFailureKind.FileVerification => "游戏文件校验失败",
            LaunchFailureKind.DirectoryNotWritable => "数据目录不可写，请在启动器设置中更换位置",
            LaunchFailureKind.PlatformUnsupported => "此版本不支持当前系统或架构",
            LaunchFailureKind.ProcessExitedEarly => "游戏进程提前退出",
            LaunchFailureKind.InvalidSettings => "启动设置不可用",
            LaunchFailureKind.InstanceInUse => "此版本正在使用中",
            LaunchFailureKind.InsufficientSpace => "数据目录剩余空间不足",
            LaunchFailureKind.DownloadsDisabled => "本地游戏文件不完整，需要下载插件补全",
            _ => "启动失败",
        };
        return failure?.Detail is { Length: > 0 } detail ? $"{message}（{detail}）" : message;
    }
}
