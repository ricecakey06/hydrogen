using System.Collections.ObjectModel;
using Launcher.Contracts;

namespace Launcher.UI.ViewModels;

/// <summary>Choose a version, configure its instance, then explicitly start installation.</summary>
internal sealed class VersionsViewModel : PageViewModel
{
    private readonly LauncherViewModel _root;
    private IReadOnlyList<GameVersionInfo>? _catalog;
    private GameVersionInfo? _version;

    public VersionsViewModel(LauncherViewModel root)
    {
        _root = root;
        Title = "版本选择";
        RefreshInstances();
    }

    public ObservableCollection<InstanceItem> Instances { get; } = [];
    public ObservableCollection<VersionItem> Available { get; } = [];
    public bool HasInstances { get; private set => SetProperty(ref field, value); }
    public bool IsAdding
    {
        get;
        private set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(IsChoosingVersion)); }
    }
    public bool IsConfiguring
    {
        get;
        private set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(IsChoosingVersion)); }
    }
    public bool IsChoosingVersion => IsAdding && !IsConfiguring;
    public bool IsBusy { get; private set => SetProperty(ref field, value); }
    public string? SelectedVersion => _version?.Id;
    public bool IsLoading { get; private set => SetProperty(ref field, value); }
    public string? LoadError { get; private set => SetProperty(ref field, value); }
    public string? ActionError { get; private set => SetProperty(ref field, value); }
    public string NewInstanceName { get; set => SetProperty(ref field, value); } = "";

    public void ShowAdd()
    {
        IsAdding = true;
        IsConfiguring = false;
        _version = null;
        Title = "添加版本";
        ActionError = null;
        NewInstanceName = "";
        if (_catalog is null && !IsLoading) LoadCatalog();
    }

    public void LoadCatalog()
    {
        IsLoading = true;
        LoadError = null;
        var accepted = _root.Operations.Start(async token =>
        {
            try
            {
                _catalog = await _root.Services.Versions.GetVersionsAsync(token);
                RebuildAvailable();
            }
            catch (GameLaunchException error) when (error.Kind == LaunchFailureKind.DownloadsDisabled)
            { LoadError = error.Message; }
            catch (Exception) when (!token.IsCancellationRequested)
            {
                LoadError = "无法获取官方版本列表，请检查网络后重试。";
            }
            finally { IsLoading = false; }
        });
        if (!accepted) IsLoading = false;
    }

    public override bool TryGoBack()
    {
        if (IsBusy) return true;
        if (IsConfiguring)
        {
            IsConfiguring = false;
            _version = null;
            Title = "添加版本";
            ActionError = null;
            return true;
        }
        if (!IsAdding) return false;
        IsAdding = false;
        Title = "版本选择";
        ActionError = null;
        return true;
    }

    public void RefreshInstances()
    {
        var service = _root.Services.Instances;
        ActionError = service.Notice;
        var selected = service.SelectedInstanceId;
        Instances.Clear();
        foreach (var instance in service.Instances.Where(instance => instance.IsInstalled))
            Instances.Add(new InstanceItem(this, instance, instance.Id == selected));
        HasInstances = Instances.Count > 0;
        RebuildAvailable();
    }

    internal void Select(InstanceInfo instance) => Run(async token =>
    {
        await _root.Services.Instances.SelectAsync(instance.Id, token);
        _root.ShowHome();
    });

    internal void Configure(GameVersionInfo version)
    {
        _version = version;
        OnPropertyChanged(nameof(SelectedVersion));
        NewInstanceName = version.Id;
        ActionError = null;
        IsConfiguring = true;
        Title = "安装实例";
    }

    internal void ShowSettings(InstanceInfo instance) => _root.ShowInstanceSettings(instance);

    public void StartInstallation()
    {
        if (_version is not { } version || !IsConfiguring) return;
        var name = NewInstanceName;
        Run(async token =>
        {
            var created = await _root.Services.Instances.CreateAsync(version.Id, name, token);
            _root.StartInstall(created);
        });
    }

    private void Run(Func<CancellationToken, Task> work)
    {
        if (IsBusy) return;
        IsBusy = true;
        ActionError = null;
        var accepted = _root.Operations.Start(async token =>
        {
            try { await work(token); }
            catch (LauncherOperationException error) { ActionError = error.Message; }
            catch (GameLaunchException error) { ActionError = error.Message; }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException) { ActionError = "操作未完成，请检查数据目录的权限和剩余空间。"; }
            finally { IsBusy = false; }
        });
        if (!accepted) IsBusy = false;
    }

    private void RebuildAvailable()
    {
        Available.Clear();
        if (_catalog is null) return;
        foreach (var version in _catalog)
            Available.Add(new VersionItem(this, version));
    }
}

internal sealed class InstanceItem(VersionsViewModel owner, InstanceInfo instance, bool isSelected)
{
    public string Name => instance.Name;
    public string GameVersion => instance.GameVersion;
    public bool IsSelected => isSelected;
    public void Select() => owner.Select(instance);
    public void ShowSettings() => owner.ShowSettings(instance);
}

internal sealed class VersionItem(VersionsViewModel owner, GameVersionInfo version)
{
    public string Id => version.Id;
    public void Configure() => owner.Configure(version);
}
