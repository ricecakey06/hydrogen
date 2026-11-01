using System.Collections.ObjectModel;
using Launcher.Contracts;

namespace Launcher.UI.ViewModels;

internal sealed class DownloadsViewModel : PageViewModel
{
    private readonly LauncherViewModel _root;
    public DownloadsViewModel(LauncherViewModel root) { _root = root; Title = "下载管理"; Refresh(); }
    public ObservableCollection<DownloadItemViewModel> Jobs { get; } = [];
    public bool HasJobs { get; private set => SetProperty(ref field, value); }
    public bool IsIndeterminate { get; private set => SetProperty(ref field, value); }
    public double OverallPercent { get; private set => SetProperty(ref field, value); }
    public string OverallText { get; private set => SetProperty(ref field, value); } = "";
    public string? Error { get; set => SetProperty(ref field, value); }
    public void ShowHome() => _root.ShowHome();
    public void Refresh()
    {
        var snapshots = _root.Services.Install.Jobs;
        foreach (var removed in Jobs.Where(item => !snapshots.Any(job => job.InstanceId == item.InstanceId)).ToArray()) Jobs.Remove(removed);
        foreach (var snapshot in snapshots)
        {
            var item = Jobs.FirstOrDefault(item => item.InstanceId == snapshot.InstanceId);
            if (item is null) { item = new(_root, snapshot); Jobs.Insert(0, item); }
            item.Refresh(snapshot);
        }
        HasJobs = Jobs.Count != 0;
        IsIndeterminate = snapshots.Any(job => job.IsActive && job.Progress?.Total is not > 0);
        var total = snapshots.Sum(job => job.Progress?.Total ?? 0);
        var completed = snapshots.Sum(job => Math.Min(job.Progress?.Completed ?? 0, job.Progress?.Total ?? 0));
        OverallPercent = total > 0 ? Math.Clamp(100d * completed / total, 0, 100) : 0;
        var active = snapshots.Count(job => job.IsActive);
        OverallText = active > 0 ? IsIndeterminate ? $"{active} 个任务正在准备" : $"总体进度 {OverallPercent:0}% · {active} 个任务正在下载"
            : snapshots.Count == 0 ? "没有进行中的下载" : "下载已停止，可重试未完成的任务";
    }

    internal static string Bytes(long value) => value >= 1024 * 1024 * 1024 ? $"{value / (1024d * 1024 * 1024):0.00} GB"
        : value >= 1024 * 1024 ? $"{value / (1024d * 1024):0.0} MB" : $"{value / 1024d:0.0} KB";
}

internal sealed class DownloadItemViewModel(LauncherViewModel root, InstallSnapshot snapshot) : ViewModelBase
{
    public string InstanceId { get; } = snapshot.InstanceId;
    public string GameVersion { get; } = snapshot.GameVersion;
    public string Name { get; } = snapshot.InstanceName ?? snapshot.GameVersion;
    public bool HasCustomName => Name != GameVersion;
    public string Status { get; private set => SetProperty(ref field, value); } = "";
    public string Detail { get; private set => SetProperty(ref field, value); } = "";
    public bool IsActive { get; private set => SetProperty(ref field, value); }
    public bool CanRetry { get; private set => SetProperty(ref field, value); }
    public bool IsIndeterminate { get; private set => SetProperty(ref field, value); }
    public double Percent { get; private set => SetProperty(ref field, value); }
    public string[] ActiveFiles { get; private set => SetProperty(ref field, value); } = [];
    public bool HasFiles { get; private set => SetProperty(ref field, value); }
    public string? Error { get; private set => SetProperty(ref field, value); }
    public void Cancel() => root.Services.Install.Cancel(InstanceId);
    public void Retry()
    {
        if (root.Services.Instances.Instances.FirstOrDefault(instance => instance.Id == InstanceId) is { } instance) root.StartInstall(instance);
    }
    public void Refresh(InstallSnapshot value)
    {
        IsActive = value.IsActive;
        CanRetry = value.Phase == InstallPhase.Failed;
        IsIndeterminate = IsActive && value.Progress?.Total is not > 0;
        Percent = value.Phase == InstallPhase.Completed ? 100 : value.Progress is { Total: > 0 } progress
            ? Math.Clamp(100d * progress.Completed / progress.Total.Value, 0, 100) : 0;
        Status = value.Phase switch
        {
            InstallPhase.Resolving => "正在获取版本信息",
            InstallPhase.Downloading => IsIndeterminate ? "正在获取资源索引" : $"正在下载 · {Percent:0}%",
            InstallPhase.Completed => "下载完成",
            InstallPhase.Cancelled => "已取消",
            _ => "下载失败",
        };
        Detail = value.Progress is { } state ? $"{DownloadsViewModel.Bytes(state.Completed)} / {(state.Total is { } total ? DownloadsViewModel.Bytes(total) : "待统计")}" +
            (state.TotalFiles > 0 ? $" · {state.CompletedFiles} / {state.TotalFiles} 个文件" : "") : "";
        ActiveFiles = IsActive ? value.Progress?.ActiveFiles?.ToArray()
            ?? (value.Progress?.CurrentFile is { } file ? [file] : []) : [];
        HasFiles = ActiveFiles.Length > 0;
        Error = value.Error;
    }
}
