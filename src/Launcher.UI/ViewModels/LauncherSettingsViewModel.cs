using System.Collections.ObjectModel;
using Avalonia.Platform.Storage;
using Launcher.Contracts;

namespace Launcher.UI.ViewModels;

/// <summary>Launcher-wide defaults, the account and the data directory (DESIGN.md 5).</summary>
internal sealed class LauncherSettingsViewModel : PageViewModel
{
    private static readonly GameWindowMode[] WindowModes = [GameWindowMode.GameDefault, GameWindowMode.Custom, GameWindowMode.Fullscreen];
    private static readonly AfterLaunchBehavior[] AfterLaunchBehaviors = [AfterLaunchBehavior.MinimizeAndRestore, AfterLaunchBehavior.KeepOpen];
    private static readonly GameDownloadSource[] DownloadSources = [GameDownloadSource.Official, GameDownloadSource.Bmclapi];
    private readonly LauncherViewModel _root;
    private string? _offlineNameSource;

    public LauncherSettingsViewModel(LauncherViewModel root)
    {
        _root = root;
        Plugins = new PluginManagerViewModel(root);
        Title = "启动器设置";
        var settings = root.Services.Settings.Settings;
        Java = new JavaSelector(root, inheritable: false, settings.Java);
        Memory = new MemoryEditor(root.Services.Settings.MemoryAdvice, settings.MaxMemoryMiB);
        WindowModeIndex = Math.Max(0, Array.IndexOf(WindowModes, settings.Window.Mode));
        WindowWidth = settings.Window.Width;
        WindowHeight = settings.Window.Height;
        AfterLaunchIndex = Math.Max(0, Array.IndexOf(AfterLaunchBehaviors, settings.AfterLaunch));
        DownloadSourceIndex = Math.Max(0, Array.IndexOf(DownloadSources, settings.DownloadSource));
        JvmArguments = settings.JvmArguments;
        DataDirectory = root.Services.Settings.DataDirectory;
        DataDirectoryIssue = root.Services.Settings.Notice;
        RefreshAccount();
    }

    public string AccountText { get; private set => SetProperty(ref field, value); } = "";
    public bool IsSignedIn { get; private set => SetProperty(ref field, value); }
    public bool CanSignIn { get; private set => SetProperty(ref field, value); }
    public bool IsOfflineMode { get; private set => SetProperty(ref field, value); }
    public string AccountHint { get; private set => SetProperty(ref field, value); } = "";
    public string OfflinePlayerName { get; set => SetProperty(ref field, value); } = "Player";
    public string? OfflineNameIssue { get; private set => SetProperty(ref field, value); }
    public bool IsSavingOfflineName
    {
        get;
        private set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(CanEditOfflineName)); }
    }
    public bool CanEditOfflineName => !IsSavingOfflineName && !_root.Services.Launch.ActiveSessions.Any(session =>
        session.Phase is LaunchPhase.CheckingEnvironment or LaunchPhase.PreparingFiles or LaunchPhase.Starting);
    public void RefreshLaunch() => OnPropertyChanged(nameof(CanEditOfflineName));

    public PluginManagerViewModel Plugins { get; }
    public bool IsPlugins { get; private set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(IsGeneral)); } }
    public bool IsGeneral => !IsPlugins;
    public void ShowGeneral() => IsPlugins = false;
    public void ShowPlugins()
    {
        if (IsPlugins) return;
        IsPlugins = true;
        Plugins.Refresh();
    }
    public override bool TryGoBack() => IsPlugins && Plugins.TryGoBack();

    public JavaSelector Java { get; private set => SetProperty(ref field, value); }
    public MemoryEditor Memory { get; private set => SetProperty(ref field, value); }

    public int WindowModeIndex
    {
        get;
        set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(IsCustomWindow)); }
    }

    public bool IsCustomWindow => WindowModeIndex == Array.IndexOf(WindowModes, GameWindowMode.Custom);
    public decimal? WindowWidth { get; set => SetProperty(ref field, value); }
    public decimal? WindowHeight { get; set => SetProperty(ref field, value); }
    public int AfterLaunchIndex { get; set => SetProperty(ref field, value); }
    public bool HasGameDownloads => _root.Services.GameFiles.DownloadsEnabled;
    public int DownloadSourceIndex
    {
        get;
        set
        {
            if (!SetProperty(ref field, value)) return;
            OnPropertyChanged(nameof(IsBmclapiSource));
            OnPropertyChanged(nameof(DownloadSourceHint));
        }
    }
    public bool IsBmclapiSource => DownloadSourceIndex == Array.IndexOf(DownloadSources, GameDownloadSource.Bmclapi);
    public string DownloadSourceHint => IsBmclapiSource ? "镜像服务由 BMCLAPI（bangbang93）提供。文件缺失或镜像限流时回退官方源，保留原始校验。"
        : "从 Mojang 官方服务下载游戏文件。保存后用于新的下载请求。";
    public void OpenBmclapiDocs() => _root.Operations.Start(async _ => await _root.Shell.SystemLauncher.LaunchUriAsync(new Uri("https://bmclapidoc.bangbang93.com/")));
    public string JvmArguments { get; set => SetProperty(ref field, value); } = "";

    public string DataDirectory { get; private set => SetProperty(ref field, value); } = "";
    public string GameDirectory => _root.Services.Settings.GameDirectory;
    public string? DataDirectoryIssue { get; private set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(HasDataDirectoryIssue)); } }
    public bool HasDataDirectoryIssue => DataDirectoryIssue is not null;

    public ObservableCollection<string> Issues { get; } = [];
    public string? SavedMessage { get; private set => SetProperty(ref field, value); }
    public bool IsSaving { get; private set => SetProperty(ref field, value); }

    public void RefreshAccount()
    {
        var state = _root.Services.Account.State;
        var modes = _root.Services.AccountModes.Snapshot;
        IsOfflineMode = modes.Mode == AccountLoginMode.Offline;
        if (_offlineNameSource != modes.OfflinePlayerName) { _offlineNameSource = modes.OfflinePlayerName; OfflinePlayerName = modes.OfflinePlayerName; }
        AccountHint = IsOfflineMode ? "当前使用离线模式，切回正版可继续使用原有微软账号。"
            : "更换账号需先退出再重新登录。退出登录只删除本机凭据，不删除游戏数据。";
        IsSignedIn = state.Status == AccountStatus.SignedIn;
        CanSignIn = state.Status is AccountStatus.SignedOut or AccountStatus.Expired;
        AccountText = state.Status switch
        {
            AccountStatus.SignedIn => state.Profile?.PlayerName ?? "",
            AccountStatus.SigningIn => "正在登录",
            AccountStatus.Expired => "登录已失效",
            _ => "未登录",
        };
        if (IsOfflineMode && IsSignedIn) AccountText = "离线账号 · " + AccountText;
    }

    public void SaveOfflineName()
    {
        if (!CanEditOfflineName) return;
        var name = OfflinePlayerName;
        IsSavingOfflineName = true;
        OfflineNameIssue = null;
        SavedMessage = null;
        var accepted = _root.Operations.Start(async token =>
        {
            try
            {
                await _root.Services.AccountModes.SetOfflinePlayerNameAsync(name, token);
                OfflinePlayerName = _root.Services.AccountModes.Snapshot.OfflinePlayerName;
                SavedMessage = "离线名称已保存，下次启动游戏时使用。";
            }
            catch (LauncherOperationException error) { OfflineNameIssue = error.Message; }
            finally { IsSavingOfflineName = false; }
        });
        if (!accepted) IsSavingOfflineName = false;
    }

    // Switching accounts is signing out and signing in again; the home page shows the browser sign-in progress.
    public void SignIn()
    {
        _root.ShowHome();
        _root.Home.SignIn();
    }

    public void SignOut() => _root.Operations.Start(async token =>
    {
        try { await _root.Services.Account.SignOutAsync(token); }
        catch (LauncherOperationException error) { Issues.Clear(); Issues.Add(error.Message); }
    });

    public void ChangeDataDirectory() => _root.Operations.Start(async token =>
    {
        var folders = await _root.Shell.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = "选择数据目录",
            AllowMultiple = false,
        });
        if (folders.Count == 0 || folders[0].TryGetLocalPath() is not { } path) return;
        var issues = await _root.Services.Settings.ChangeDataDirectoryAsync(path, token);
        DataDirectoryIssue = issues.Count == 0 ? null : string.Join(" ", issues.Select(issue => issue.Message));
        DataDirectory = _root.Services.Settings.DataDirectory;
        OnPropertyChanged(nameof(GameDirectory));
        if (issues.Count == 0)
        {
            var settings = _root.Services.Settings.Settings;
            Java = new JavaSelector(_root, inheritable: false, settings.Java);
            Memory = new MemoryEditor(_root.Services.Settings.MemoryAdvice, settings.MaxMemoryMiB);
            WindowModeIndex = Math.Max(0, Array.IndexOf(WindowModes, settings.Window.Mode));
            WindowWidth = settings.Window.Width;
            WindowHeight = settings.Window.Height;
            AfterLaunchIndex = Math.Max(0, Array.IndexOf(AfterLaunchBehaviors, settings.AfterLaunch));
            DownloadSourceIndex = Math.Max(0, Array.IndexOf(DownloadSources, settings.DownloadSource));
            JvmArguments = settings.JvmArguments;
            Issues.Clear();
            SavedMessage = "已切换数据目录，已有数据保留在原位置。";
        }
    });

    public void Save()
    {
        if (IsSaving) return;
        var current = _root.Services.Settings.Settings;
        var settings = new LauncherSettings(
            Java.Choice ?? JavaChoice.Automatic,
            Memory.Value,
            new GameWindowSettings(
                WindowModes[Math.Clamp(WindowModeIndex, 0, WindowModes.Length - 1)],
                (int)(WindowWidth ?? current.Window.Width),
                (int)(WindowHeight ?? current.Window.Height)),
            AfterLaunchBehaviors[Math.Clamp(AfterLaunchIndex, 0, AfterLaunchBehaviors.Length - 1)],
            JvmArguments.Trim(),
            DownloadSources[Math.Clamp(DownloadSourceIndex, 0, DownloadSources.Length - 1)]);
        IsSaving = true;
        Issues.Clear();
        SavedMessage = null;
        var accepted = _root.Operations.Start(async token =>
        {
            try
            {
                var issues = await _root.Services.Settings.SaveAsync(settings, token);
                foreach (var issue in issues) Issues.Add(issue.Message);
                if (issues.Count == 0) SavedMessage = settings.DownloadSource != current.DownloadSource
                    ? "已保存，下载源对新请求生效。" : "已保存，下次启动时生效。";
            }
            finally { IsSaving = false; }
        });
        if (!accepted) IsSaving = false;
    }
}
