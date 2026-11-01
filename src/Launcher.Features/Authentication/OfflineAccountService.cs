using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using Launcher.Contracts;
using Launcher.Plugins.Storage;
using Launcher.Runtime;

namespace Launcher.Plugins.Authentication;

/// <summary>A local test identity. It never calls Microsoft, reads credentials, or represents an online entitlement.</summary>
public sealed class OfflineAccountService(Context context, ILauncherStorage storage) : IOfflineAccountService
{
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly object _sync = new();
    private string _name = "Player";
    private string? _problem;
    private AccountState _state = new(AccountStatus.SignedOut, null);
    private static string SettingsPath => Path.Combine("accounts", "offline.json");
    public string PlayerName { get { lock (_sync) return _name; } }
    public AccountState State { get { lock (_sync) return _state; } }

    public async Task LoadAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            var name = "Player";
            string? problem = null;
            try
            {
                var settings = await storage.ReadAsync<OfflineSettings>(SettingsPath, cancellationToken).ConfigureAwait(false);
                if (settings is not null)
                {
                    if (!IsValidName(settings.PlayerName)) throw new InvalidDataException();
                    name = settings.PlayerName;
                }
            }
            catch (Exception error) when (LauncherStorage.IsStorageError(error))
            { problem = "离线账号配置无法读取或格式不兼容，请检查 accounts/offline.json。"; }
            lock (_sync)
            {
                _name = name;
                _problem = problem;
                _state = problem is null ? SignedIn(name) : new(AccountStatus.SignedOut, null, AccountFailureKind.Configuration, problem);
            }
        }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    public async Task SetPlayerNameAsync(string playerName, CancellationToken cancellationToken)
    {
        playerName = playerName.Trim();
        if (!IsValidName(playerName)) throw new LauncherOperationException("离线名称应为 3–16 位英文字母、数字或下划线。");
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            lock (_sync) if (_problem is not null) throw new LauncherOperationException(_problem);
            await storage.WriteAsync(SettingsPath, new OfflineSettings(1, playerName), cancellationToken).ConfigureAwait(false);
            lock (_sync)
            {
                _name = playerName;
                if (_state.Status == AccountStatus.SignedIn) _state = SignedIn(playerName);
            }
        }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { throw new LauncherOperationException("离线名称未保存，请检查数据目录的权限和配置格式。"); }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    public Task SignInAsync(CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        context.Stopping.ThrowIfCancellationRequested();
        lock (_sync)
        {
            if (_problem is not null) throw new LauncherOperationException(_problem);
            _state = SignedIn(_name);
        }
        return NotifyAsync();
    }

    public Task SignOutAsync(CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        lock (_sync) _state = new(AccountStatus.SignedOut, null);
        return NotifyAsync();
    }

    public Task<GameAccountSession> GetSessionAsync(CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        context.Stopping.ThrowIfCancellationRequested();
        lock (_sync)
        {
            if (_state.Status != AccountStatus.SignedIn) throw new AccountAuthenticationException(AccountFailureKind.Unexpected);
            return Task.FromResult(new GameAccountSession(_name, CreateUuid(_name), "0", "", "", DateTimeOffset.MaxValue, AccountLoginMode.Offline));
        }
    }

    public static string CreateUuid(string playerName)
    {
        // Java UUID.nameUUIDFromBytes("OfflinePlayer:" + name), including UUID version/variant and network byte order.
        var hash = MD5.HashData(Encoding.UTF8.GetBytes("OfflinePlayer:" + playerName));
        hash[6] = (byte)((hash[6] & 0x0f) | 0x30);
        hash[8] = (byte)((hash[8] & 0x3f) | 0x80);
        return new Guid(hash, bigEndian: true).ToString("N");
    }

    private static bool IsValidName(string name) => !string.IsNullOrEmpty(name) && Regex.IsMatch(name, "^[A-Za-z0-9_]{3,16}$", RegexOptions.CultureInvariant);
    private static AccountState SignedIn(string name) => new(AccountStatus.SignedIn, new(name, null), Detail: "离线模式");
    private Task NotifyAsync() => context.NotifyAsync(new AccountProviderStateChanged(AccountLoginMode.Offline, State));
    private sealed record OfflineSettings(int SchemaVersion, string PlayerName);
}
