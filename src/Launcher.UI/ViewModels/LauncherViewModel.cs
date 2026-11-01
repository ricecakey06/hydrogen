using Launcher.Contracts;
using Launcher.UI.Shell;

namespace Launcher.UI.ViewModels;

/// <summary>The root of <c>ui.launcher</c>: page navigation and window behaviour around game sessions.</summary>
internal sealed class LauncherViewModel : ViewModelBase, IDisposable
{
    private PageViewModel? _page;
    private bool _gameRunning;
    private bool _minimizedForGame;
    private bool _quitWhenIdle;

    public LauncherViewModel(LauncherServices services, IShell shell, UiOperations operations)
    {
        Services = services;
        Shell = shell;
        Operations = operations;
        Downloads = new DownloadsViewModel(this);
        Home = new HomeViewModel(this);
        _gameRunning = IsAnyGameRunning();
    }

    public LauncherServices Services { get; }
    public IShell Shell { get; }
    public UiOperations Operations { get; }
    public HomeViewModel Home { get; }
    public DownloadsViewModel Downloads { get; }

    public PageViewModel? Page
    {
        get => _page;
        private set
        {
            if (!SetProperty(ref _page, value)) return;
            OnPropertyChanged(nameof(IsHomeVisible));
            OnPropertyChanged(nameof(CurrentPage));
        }
    }

    public bool IsHomeVisible => _page is null;
    public ViewModelBase CurrentPage => (ViewModelBase?)_page ?? Home;

    public void ShowHome() => Page = null;
    public void ShowVersions() => Page = new VersionsViewModel(this);
    public void ShowLauncherSettings() => Page = new LauncherSettingsViewModel(this);
    public void ShowDownloads() { Downloads.Refresh(); Page = Downloads; }

    public void StartInstall(InstanceInfo instance)
    {
        Downloads.Error = null;
        ShowDownloads();
        Operations.Start(async token =>
        {
            try { await Services.Install.InstallAsync(instance.Id, token); }
            catch (LauncherOperationException error) { Downloads.Error = error.Message; }
            catch (GameLaunchException error) { Downloads.Error = error.Message; }
            catch (IOException) { Downloads.Error = "该版本正在使用，请等待游戏退出后重试。"; }
            finally { RefreshDownloads(); }
        });
    }

    public void RefreshDownloads()
    {
        Downloads.Refresh();
        Home.RefreshDownloads();
    }

    public void ShowInstanceSettings()
    {
        var selected = Services.Instances.SelectedInstanceId;
        if (Services.Instances.Instances.FirstOrDefault(instance => instance.Id == selected) is { } instance)
            ShowInstanceSettings(instance);
    }

    public void ShowInstanceSettings(InstanceInfo instance) => Page = new InstanceSettingsViewModel(this, instance);

    public void GoBack()
    {
        if (_page?.TryGoBack() != true) ShowHome();
    }

    public void RefreshAccount()
    {
        Home.RefreshAccount();
        (_page as LauncherSettingsViewModel)?.RefreshAccount();
    }

    public void RefreshInstances()
    {
        Home.RefreshInstance();
        (_page as VersionsViewModel)?.RefreshInstances();
    }

    public void RefreshLaunch()
    {
        Home.RefreshLaunch();
        (_page as LauncherSettingsViewModel)?.RefreshLaunch();
        var running = IsAnyGameRunning();
        if (running && !_gameRunning && !_quitWhenIdle
            && (Services.Launch.ActiveSessions.FirstOrDefault(session => session.Phase == LaunchPhase.Running)?.AfterLaunch
                ?? Services.Settings.Settings.AfterLaunch) == AfterLaunchBehavior.MinimizeAndRestore)
        {
            Shell.Minimize();
            _minimizedForGame = true;
        }
        else if (!running && _gameRunning && _minimizedForGame)
        {
            _minimizedForGame = false;
            if (!_quitWhenIdle) Shell.Restore();
        }
        _gameRunning = running;
        if (_quitWhenIdle && Services.Launch.ActiveSessions.Count == 0) Shell.Quit();
    }

    /// <summary>Closing the window during a session hides it and keeps watching the game; the launcher quits afterwards.</summary>
    public bool HandleCloseRequest()
    {
        if (Services.Launch.ActiveSessions.Count == 0) return false;
        _quitWhenIdle = true;
        Shell.Hide();
        return true;
    }

    public void Dispose() => Home.Dispose();

    private bool IsAnyGameRunning() => Services.Launch.ActiveSessions.Any(session => session.Phase == LaunchPhase.Running);
}
