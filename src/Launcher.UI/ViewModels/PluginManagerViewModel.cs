using System.Collections.ObjectModel;
using Avalonia.Media;
using Avalonia.Platform.Storage;
using Launcher.Contracts;

namespace Launcher.UI.ViewModels;

internal sealed class PluginManagerViewModel(LauncherViewModel root) : ViewModelBase
{
    private List<PluginItemViewModel> _all = [];
    private CancellationTokenSource? _installCancellation;
    public ObservableCollection<PluginItemViewModel> BuiltIns { get; } = [];
    public ObservableCollection<PluginItemViewModel> Installed { get; } = [];
    public string SearchQuery { get; set { if (SetProperty(ref field, value)) Filter(); } } = "";
    public int FilterIndex
    {
        get;
        private set
        {
            if (field == value) return;
            IsListTransitionReversed = value < field;
            SetProperty(ref field, value);
            Filter();
        }
    }
    public bool IsListTransitionReversed { get; private set => SetProperty(ref field, value); }
    public PluginListViewModel? List { get; private set => SetProperty(ref field, value); }
    public bool IsAll => FilterIndex == 0;
    public bool IsBuiltInFilter => FilterIndex == 1;
    public bool IsInstalledFilter => FilterIndex == 2;
    public string AllLabel => $"全部  {_all.Count}";
    public string BuiltInLabel => $"内置  {_all.Count(item => item.Info.Origin == PluginOrigin.BuiltIn)}";
    public string InstalledLabel => $"已安装  {_all.Count(item => item.Info.Origin == PluginOrigin.Installed)}";
    public string Summary { get; private set => SetProperty(ref field, value); } = "正在读取插件…";
    public string? Notice { get; private set => SetProperty(ref field, value); }
    public string? ActionMessage { get; private set => SetProperty(ref field, value); }
    public bool IsError { get; private set => SetProperty(ref field, value); }
    public bool IsReadOnly { get; private set => SetProperty(ref field, value); }
    public bool HasPendingChanges { get; private set => SetProperty(ref field, value); }
    public bool IsBusy
    {
        get;
        private set
        {
            if (!SetProperty(ref field, value)) return;
            OnPropertyChanged(nameof(CanInteract));
            OnPropertyChanged(nameof(InstallActionLabel));
            foreach (var item in _all) item.NotifyAvailability();
        }
    }
    public bool CanInteract => !IsBusy && !IsReadOnly;
    public bool HasBuiltIns => BuiltIns.Count > 0;
    public bool HasInstalled => Installed.Count > 0;
    public bool IsEmpty => !HasBuiltIns && !HasInstalled && !IsBusy;
    public string EmptyTitle => IsInstalledFilter && string.IsNullOrWhiteSpace(SearchQuery) ? "还没有安装外置插件" : "没有找到匹配的插件";
    public string EmptyDescription => IsInstalledFilter && string.IsNullOrWhiteSpace(SearchQuery)
        ? "选择本地插件包，为启动器添加扩展。" : "试试其他名称、描述或插件 ID。";
    public bool ShowEmptyInstallAction => IsInstalledFilter && string.IsNullOrWhiteSpace(SearchQuery);
    public PluginItemViewModel? SelectedPlugin
    {
        get;
        private set
        {
            if (!SetProperty(ref field, value)) return;
            OnPropertyChanged(nameof(IsDetail));
            IsConfirmingRemoval = false;
        }
    }
    public bool IsDetail => SelectedPlugin is not null;
    public bool IsConfirmingRemoval { get; private set => SetProperty(ref field, value); }
    public bool IsInstallOpen { get; private set => SetProperty(ref field, value); }
    public string? PackagePath { get; private set { if (SetProperty(ref field, value)) OnPropertyChanged(nameof(PackageLabel)); } }
    public string PackageLabel => PackagePath is null ? "选择插件包…" : Path.GetFileName(PackagePath);
    public string InstallActionLabel => IsBusy ? "正在安装…" : "安装插件";

    public void ShowAll() => FilterIndex = 0;
    public void ShowBuiltIns() => FilterIndex = 1;
    public void ShowInstalled() => FilterIndex = 2;
    public void Refresh() => Run(async token => await ReadAsync(token));

    private async Task ReadAsync(CancellationToken token)
    {
        var inventory = await Task.Run(() => root.Services.Plugins.GetInventoryAsync(token), token);
        var selectedId = SelectedPlugin?.Id;
        _all = inventory.Plugins.Select(info => new PluginItemViewModel(this, info)).ToList();
        IsReadOnly = inventory.IsReadOnly;
        Notice = inventory.Notice;
        HasPendingChanges = _all.Any(item => item.Info.RequiresRestart);
        Summary = $"共 {_all.Count} 个插件 · {_all.Count(item => item.Info.State == PluginRunState.Active)} 个运行中";
        if (selectedId is not null) SelectedPlugin = _all.FirstOrDefault(item => item.Id == selectedId);
        foreach (var name in new[] { nameof(AllLabel), nameof(BuiltInLabel), nameof(InstalledLabel), nameof(CanInteract) }) OnPropertyChanged(name);
        Filter();
    }

    private void Filter()
    {
        var query = SearchQuery.Trim();
        BuiltIns.Clear();
        Installed.Clear();
        foreach (var item in _all)
        {
            if (query.Length > 0 && !new[] { item.Info.Id, item.Info.Name, item.Info.Description, item.Info.Author }
                .Any(value => value.Contains(query, StringComparison.OrdinalIgnoreCase))) continue;
            if (item.Info.Origin == PluginOrigin.BuiltIn && FilterIndex != 2) BuiltIns.Add(item);
            if (item.Info.Origin == PluginOrigin.Installed && FilterIndex != 1) Installed.Add(item);
        }
        foreach (var name in new[] { nameof(IsAll), nameof(IsBuiltInFilter), nameof(IsInstalledFilter), nameof(HasBuiltIns),
                     nameof(HasInstalled), nameof(IsEmpty), nameof(EmptyTitle), nameof(EmptyDescription), nameof(ShowEmptyInstallAction) }) OnPropertyChanged(name);
        // Each presenter keeps its own snapshot while the previous list fades out.
        List = new PluginListViewModel(this, BuiltIns.ToArray(), Installed.ToArray(), FilterIndex,
            EmptyTitle, EmptyDescription, ShowEmptyInstallAction);
    }

    public void OpenDetails(PluginItemViewModel item) => SelectedPlugin = item;
    public void CloseDetails() => SelectedPlugin = null;
    public void OpenInstall()
    {
        if (!CanInteract) return;
        ActionMessage = null;
        PackagePath = null;
        IsInstallOpen = true;
    }
    public void CloseInstall()
    {
        if (_installCancellation is not null) { _installCancellation.Cancel(); return; }
        if (!IsBusy) IsInstallOpen = false;
    }

    public void BrowsePackage() => Run(async _ =>
    {
        var files = await root.Shell.StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = "选择 Hydrogen 插件包", AllowMultiple = false,
            FileTypeFilter = [new FilePickerFileType("Hydrogen 插件包") { Patterns = ["*.hplugin", "*.zip"] }],
        });
        if (files.Count > 0)
        {
            PackagePath = files[0].TryGetLocalPath();
            if (PackagePath is null) throw new PluginManagementException("请选择保存在本机上的插件包。 ");
        }
    });

    public void Install()
    {
        if (!CanInteract) return;
        if (PackagePath is not { } path) { IsError = true; ActionMessage = "请先选择一个 .hplugin 或 .zip 插件包。"; return; }
        Run(async token =>
        {
            using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(token);
            _installCancellation = cancellation;
            try
            {
                await Task.Run(() => root.Services.Plugins.InstallAsync(path, cancellation.Token), cancellation.Token);
                IsInstallOpen = false;
                SelectedPlugin = null;
                ActionMessage = "已安装插件，重启启动器后加载。";
                FilterIndex = 2;
                await ReadAsync(token);
            }
            finally { _installCancellation = null; }
        });
    }

    internal void Toggle(PluginItemViewModel item, bool enabled)
    {
        if (!item.CanToggle) { item.NotifyEnabled(); return; }
        Run(async token =>
        {
            try
            {
                await Task.Run(() => root.Services.Plugins.SetEnabledAsync(item.Id, enabled, token), token);
                ActionMessage = enabled ? "已启用插件，重启后生效。" : "已停用插件，重启后生效。";
                await ReadAsync(token);
            }
            finally { item.NotifyEnabled(); }
        });
    }

    public void RequestUninstall() => IsConfirmingRemoval = SelectedPlugin?.CanUninstall == true;
    public void CancelUninstall() => IsConfirmingRemoval = false;
    public void ConfirmUninstall()
    {
        if (SelectedPlugin is not { CanUninstall: true } item) return;
        Run(async token =>
        {
            await Task.Run(() => root.Services.Plugins.UninstallAsync(item.Id, token), token);
            IsConfirmingRemoval = false;
            ActionMessage = "已安排卸载，重启后移除插件。你可以在重启前撤销。";
            await ReadAsync(token);
        });
    }
    public void UndoUninstall()
    {
        if (SelectedPlugin is not { } item) return;
        Run(async token =>
        {
            await Task.Run(() => root.Services.Plugins.UndoUninstallAsync(item.Id, token), token);
            ActionMessage = "已撤销卸载。";
            await ReadAsync(token);
        });
    }

    public void OpenPluginDirectory() => Run(async _ =>
    {
        var directory = root.Services.Plugins.PluginsDirectory;
        Directory.CreateDirectory(directory);
        await root.Shell.SystemLauncher.LaunchUriAsync(new Uri(Path.GetFullPath(directory) + Path.DirectorySeparatorChar));
    });

    public bool TryGoBack()
    {
        if (IsInstallOpen) { CloseInstall(); return true; }
        if (IsConfirmingRemoval) { CancelUninstall(); return true; }
        if (IsDetail) { CloseDetails(); return true; }
        return false;
    }

    private void Run(Func<CancellationToken, Task> action)
    {
        if (IsBusy) return;
        IsBusy = true;
        IsError = false;
        ActionMessage = null;
        var accepted = root.Operations.Start(async token =>
        {
            try { await action(token); }
            catch (OperationCanceledException) { ActionMessage = "操作已取消。"; }
            catch (PluginManagementException error) { IsError = true; ActionMessage = error.Message.Trim(); }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            { IsError = true; ActionMessage = "无法访问插件文件，请检查目录权限后重试。"; }
            finally { IsBusy = false; OnPropertyChanged(nameof(IsEmpty)); }
        });
        if (!accepted) IsBusy = false;
    }
}

internal sealed record PluginListViewModel(PluginManagerViewModel Actions,
    IReadOnlyList<PluginItemViewModel> BuiltIns, IReadOnlyList<PluginItemViewModel> Installed,
    int FilterIndex, string EmptyTitle, string EmptyDescription, bool ShowEmptyInstallAction)
{
    public bool HasBuiltIns => BuiltIns.Count > 0;
    public bool HasInstalled => Installed.Count > 0;
    public bool IsEmpty => !HasBuiltIns && !HasInstalled;
    public bool IsBuiltInFilter => FilterIndex == 1;
}

internal sealed class PluginItemViewModel(PluginManagerViewModel owner, ManagedPlugin info) : ViewModelBase
{
    public ManagedPlugin Info { get; } = info;
    public string Id => Info.Id;
    public string ToggleLabel => "启用 " + Info.Name;
    public bool IsEnabled { get => Info.IsEnabled; set { if (value != Info.IsEnabled) owner.Toggle(this, value); } }
    public bool CanToggle => !Info.IsRequired && !Info.IsPendingRemoval && owner.CanInteract;
    public bool CanUninstall => Info.Origin == PluginOrigin.Installed && !Info.IsPendingRemoval && owner.CanInteract;
    public bool IsBuiltIn => Info.Origin == PluginOrigin.BuiltIn;
    public bool IsInstalled => !IsBuiltIn;
    public string OriginLabel => IsBuiltIn ? "官方内置" : "外置安装";
    public string StatusLabel => Info.IsPendingRemoval ? "待卸载" : Info.RequiresRestart ? (Info.IsEnabled ? "待启用" : "待停用") : Info.State switch
    {
        PluginRunState.Active => "运行中",
        PluginRunState.Disabled => "已停用",
        PluginRunState.Waiting => "等待依赖",
        PluginRunState.Failed => "加载失败",
        _ => "尚未加载",
    };
    public IBrush StatusBrush => Brush.Parse(Info.IsPendingRemoval || Info.RequiresRestart ? "#A76C21" : Info.State switch
    {
        PluginRunState.Active => "#24855E", PluginRunState.Failed => "#B42318", PluginRunState.Waiting => "#A76C21", _ => "#858B95",
    });
    public string StateDescription => Info.Problem ?? (Info.IsPendingRemoval ? "此插件将在下次启动时移除。"
        : Info.RequiresRestart ? "配置已保存，当前运行状态保持到启动器关闭。" : Info.State == PluginRunState.Active ? "插件已成功激活。" : "此插件当前没有运行。 ");
    public string DependenciesText => Info.Dependencies.Count == 0 ? "无其他插件依赖" : string.Join("\n", Info.Dependencies);
    public string ProvidesText => Info.Provides.Count == 0 ? "未声明共享服务" : string.Join("\n", Info.Provides);
    public string IconPath => Info.Id switch
    {
        "ui.shell" or "platform.windows.window" => "M3,5 H25 V23 H3 Z M3,10 H25 M7,7.5 H8 M11,7.5 H12",
        "ui.launcher" => "M3,3 H11 V11 H3 Z M17,3 H25 V11 H17 Z M3,17 H11 V25 H3 Z M17,17 H25 V25 H17 Z",
        "account.microsoft" or "account.offline" or "account.manager" or "credentials" => "M19,8 A5,5 0 1 1 9,8 A5,5 0 1 1 19,8 M4,25 V23 C4,12 24,12 24,23 V25",
        "minecraft.launch" or "process" => "M7,3 L24,14 L7,25 Z",
        "storage" or "java.runtime" => "M3,7 H25 M3,21 H25 M9,3 V11 M19,17 V25 M3,14 H25 M17,10 V18",
        "minecraft.content" or "minecraft.instances" => "M14,2 L26,8 V21 L14,27 L2,21 V8 Z M2,8 L14,14 L26,8 M14,14 V27",
        _ => "M4,5 H11 V3 A3,3 0 0 1 17,3 V5 H24 V12 H26 A3,3 0 0 1 26,18 H24 V25 H17 V23 A3,3 0 0 0 11,23 V25 H4 V18 H6 A3,3 0 0 0 6,12 H4 Z",
    };
    public Geometry IconGeometry => Geometry.Parse(IconPath);
    public IBrush IconBackground => Brush.Parse(IsBuiltIn ? "#EDF1F7" : "#F0EDF7");
    public void OpenDetails() => owner.OpenDetails(this);
    internal void NotifyEnabled() => OnPropertyChanged(nameof(IsEnabled));
    internal void NotifyAvailability() { OnPropertyChanged(nameof(CanToggle)); OnPropertyChanged(nameof(CanUninstall)); }
}
