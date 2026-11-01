using System.Collections.Concurrent;
using System.Security.Cryptography;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.Plugins.Storage;

namespace Launcher.Plugins.Minecraft;

/// <summary>Verifies local content. Network acquisition is borrowed from an optional, independently owned registration.</summary>
public sealed class GameFileService(ILauncherStorage storage) : IGameFileService, IGameDownloadRegistry
{
    private readonly object _sync = new();
    private readonly ConcurrentDictionary<string, SemaphoreSlim> _files = new(OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal);
    private Registration? _registration;
    private static string CatalogPath => Path.Combine("cache", "manifests", "catalog.json");
    public bool DownloadsEnabled { get { lock (_sync) return _registration is not null; } }

    public ValueTask<IAsyncDisposable> RegisterAsync(IGameDownloadProvider provider, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        ArgumentNullException.ThrowIfNull(provider);
        lock (_sync)
        {
            if (_registration is not null) throw new InvalidOperationException("A game download provider is already registered.");
            return ValueTask.FromResult<IAsyncDisposable>(_registration = new Registration(this, provider));
        }
    }

    public async Task<JsonElement> GetCatalogAsync(CancellationToken cancellationToken)
    {
        using var usage = storage.AcquireUsage();
        using var provider = BorrowProvider();
        Exception? remoteFailure = null;
        if (provider is not null)
        {
            using var linked = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, provider.Stopping);
            try
            {
                var manifest = await provider.Provider.GetCatalogAsync(linked.Token).ConfigureAwait(false);
                ValidateCatalog(manifest);
                await storage.WriteAsync(CatalogPath, new CatalogFile(1, manifest), cancellationToken).ConfigureAwait(false);
                return manifest;
            }
            catch (Exception error) when (!cancellationToken.IsCancellationRequested && error is HttpRequestException or OperationCanceledException or InvalidDataException or JsonException)
            { remoteFailure = error; }
        }
        try
        {
            var cached = await storage.ReadAsync<CatalogFile>(CatalogPath, cancellationToken).ConfigureAwait(false);
            if (cached is not null)
            {
                ValidateCatalog(cached.Manifest);
                return cached.Manifest;
            }
        }
        catch (Exception error) when (error is InvalidDataException or JsonException)
        { throw provider is null || provider.Stopping.IsCancellationRequested ? Disabled("本地版本目录损坏或格式不兼容")
            : new GameLaunchException(LaunchFailureKind.FileVerification, "版本目录暂时无法修复，请检查网络后重试。"); }
        if (provider is null || provider.Stopping.IsCancellationRequested) throw Disabled("本地没有版本目录");
        throw remoteFailure is InvalidDataException or JsonException ? new InvalidDataException("The official version catalog is invalid.")
            : new HttpRequestException("The official version catalog is unavailable.");
    }

    public async Task EnsureFileAsync(DownloadFile file, CancellationToken cancellationToken)
    {
        using var usage = storage.AcquireUsage();
        if (file.Sha1.Length != 40 || file.Sha1.Any(c => !Uri.IsHexDigit(c)) || file.Size is < 0)
            throw new InvalidDataException("Invalid file verification metadata.");
        var gate = _files.GetOrAdd(Path.GetFullPath(file.Destination), _ => new SemaphoreSlim(1, 1));
        await gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if (await VerifyAsync(file, cancellationToken).ConfigureAwait(false)
                || await ImportLegacyAsync(file, cancellationToken).ConfigureAwait(false))
            { file.Progress?.Report(new FileInfo(file.Destination).Length); return; }
            using var provider = BorrowProvider();
            if (provider is null) throw Disabled("所需游戏文件缺失或校验失败");
            using var linked = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, provider.Stopping);
            try { await provider.Provider.DownloadAsync(file, linked.Token).ConfigureAwait(false); }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested && provider.Stopping.IsCancellationRequested)
            { throw Disabled("下载模块已停止"); }
            if (!await VerifyAsync(file, cancellationToken).ConfigureAwait(false)) throw new InvalidDataException("The game download provider did not produce a verified file.");
        }
        finally { gate.Release(); }
    }

    private BorrowedProvider? BorrowProvider()
    {
        lock (_sync)
        {
            if (_registration is null) return null;
            _registration.Users++;
            return new BorrowedProvider(_registration);
        }
    }

    private static async Task<bool> VerifyAsync(DownloadFile file, CancellationToken token)
    {
        if (!File.Exists(file.Destination)) return false;
        await using var input = new FileStream(file.Destination, FileMode.Open, FileAccess.Read, FileShare.Read | FileShare.Delete, 81920, true);
        if (file.Size is { } size && input.Length != size) return false;
        return Convert.ToHexString(await SHA1.HashDataAsync(input, token).ConfigureAwait(false)).Equals(file.Sha1, StringComparison.OrdinalIgnoreCase);
    }
    private async Task<bool> ImportLegacyAsync(DownloadFile file, CancellationToken token)
    {
        var relative = Path.GetRelativePath(storage.GameRoot, file.Destination).Replace('\\', '/');
        string? legacy = relative.StartsWith("assets/", StringComparison.Ordinal) || relative.StartsWith("libraries/", StringComparison.Ordinal)
            ? Path.Combine("cache", relative) : null;
        var parts = relative.Split('/');
        if (parts.Length == 3 && parts[0] == "versions" && ContentService.IsSupportedVersion(parts[1]))
        {
            if (parts[2] == parts[1] + ".json") legacy = Path.Combine("cache", "manifests", file.Sha1 + ".json");
            if (parts[2] == parts[1] + ".jar") legacy = Path.Combine("cache", "clients", parts[2]);
        }
        if (legacy is null) return false;
        var original = file with { Destination = storage.Resolve(legacy), Progress = null };
        if (!await VerifyAsync(original, token).ConfigureAwait(false)) return false;
        Directory.CreateDirectory(Path.GetDirectoryName(file.Destination)!);
        var temporary = file.Destination + "." + Guid.NewGuid().ToString("N") + ".import";
        try
        {
            await using (var input = new FileStream(original.Destination, FileMode.Open, FileAccess.Read, FileShare.Read, 81920, true))
            await using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None, 81920, true))
                await input.CopyToAsync(output, token).ConfigureAwait(false);
            if (!await VerifyAsync(file with { Destination = temporary }, token).ConfigureAwait(false)) return false;
            token.ThrowIfCancellationRequested();
            File.Move(temporary, file.Destination, overwrite: true);
            return true;
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
    private static void ValidateCatalog(JsonElement manifest)
    {
        if (manifest.ValueKind != JsonValueKind.Object || !manifest.TryGetProperty("versions", out var versions) || versions.ValueKind != JsonValueKind.Array || versions.GetArrayLength() == 0)
            throw new InvalidDataException("The version catalog is invalid.");
    }
    private static GameLaunchException Disabled(string reason) => new(LaunchFailureKind.DownloadsDisabled, $"{reason}，请启用或恢复“游戏下载”插件并重启后重试。");
    private sealed record CatalogFile(int SchemaVersion, JsonElement Manifest);
    private sealed class Registration(GameFileService owner, IGameDownloadProvider provider) : IAsyncDisposable
    {
        internal readonly CancellationTokenSource Stop = new();
        internal readonly TaskCompletionSource Drained = new(TaskCreationOptions.RunContinuationsAsynchronously);
        internal IGameDownloadProvider Provider => provider;
        internal int Users;
        private Task? _disposal;
        public ValueTask DisposeAsync()
        {
            lock (owner._sync)
            {
                if (_disposal is not null) return new(_disposal);
                if (ReferenceEquals(owner._registration, this)) owner._registration = null;
                if (Users == 0) Drained.TrySetResult();
                return new(_disposal = DisposeCoreAsync());
            }
        }
        private async Task DisposeCoreAsync()
        {
            await Task.Yield();
            await Stop.CancelAsync().ConfigureAwait(false);
            await Drained.Task.ConfigureAwait(false);
            Stop.Dispose();
        }
        internal void Release()
        { lock (owner._sync) if (--Users == 0 && !ReferenceEquals(owner._registration, this)) Drained.TrySetResult(); }
    }
    private sealed class BorrowedProvider(Registration registration) : IDisposable
    {
        private Registration? _registration = registration;
        internal IGameDownloadProvider Provider => _registration?.Provider ?? throw new ObjectDisposedException(nameof(BorrowedProvider));
        internal CancellationToken Stopping { get; } = registration.Stop.Token;
        public void Dispose() => Interlocked.Exchange(ref _registration, null)?.Release();
    }
}
