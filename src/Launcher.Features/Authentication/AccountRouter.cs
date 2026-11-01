using Launcher.Contracts;
using Launcher.Plugins.Storage;
using Launcher.Runtime;

namespace Launcher.Plugins.Authentication;

/// <summary>Routes the current account without replacing or signing out the Microsoft provider.</summary>
public sealed class AccountRouter(Context context, IMicrosoftAccountService microsoft, ILauncherStorage storage)
    : IAccountService, IGameAccountService, IAccountModeService, IAccountProviderRegistry
{
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly object _sync = new();
    private IOfflineAccountService? _offline;
    private AccountLoginMode _mode;
    private AccountLoginMode _preferred;
    private string? _problem;
    private static string SettingsPath => Path.Combine("accounts", "mode.json");

    public AccountState State { get { lock (_sync) return CurrentAccount.State; } }
    public AccountModeSnapshot Snapshot
    { get { lock (_sync) return new(_offline is not null, _mode, _offline?.PlayerName ?? "Player", _problem); } }
    private IAccountService CurrentAccount => _mode == AccountLoginMode.Offline && _offline is not null ? _offline : microsoft;
    private IGameAccountService CurrentGameAccount => _mode == AccountLoginMode.Offline && _offline is not null ? _offline : microsoft;

    public async Task LoadPreferencesAsync(CancellationToken token)
    {
        await _gate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            AccountLoginMode preferred = AccountLoginMode.Microsoft;
            string? problem = null;
            try
            {
                var saved = await storage.ReadAsync<ModeSettings>(SettingsPath, token).ConfigureAwait(false);
                if (saved is not null)
                {
                    if (!Enum.IsDefined(saved.Mode)) throw new InvalidDataException();
                    preferred = saved.Mode;
                }
            }
            catch (Exception error) when (LauncherStorage.IsStorageError(error))
            { problem = "登录方式配置无法读取或格式不兼容，请检查 accounts/mode.json。"; }
            lock (_sync)
            {
                _preferred = preferred;
                _mode = preferred == AccountLoginMode.Offline && _offline is not null ? preferred : AccountLoginMode.Microsoft;
                _problem = problem;
            }
        }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    public async ValueTask<IAsyncDisposable> RegisterOfflineAsync(IOfflineAccountService provider, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(provider);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            lock (_sync)
            {
                if (_offline is not null) throw new InvalidOperationException("An offline account provider is already registered.");
                _offline = provider;
                if (_preferred == AccountLoginMode.Offline) _mode = AccountLoginMode.Offline;
            }
        }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
        return new Registration(this, provider);
    }

    public async Task SelectAsync(AccountLoginMode mode, CancellationToken cancellationToken)
    {
        if (!Enum.IsDefined(mode)) throw new LauncherOperationException("无法识别此登录方式。");
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            IOfflineAccountService? offline;
            lock (_sync)
            {
                if (_problem is not null) throw new LauncherOperationException(_problem);
                offline = _offline;
                if (mode == AccountLoginMode.Offline && offline is null) throw new LauncherOperationException("离线登录插件尚未启用，请先启用并重启启动器。");
            }
            if (mode == AccountLoginMode.Offline && offline!.State.Status != AccountStatus.SignedIn)
                await offline.SignInAsync(cancellationToken).ConfigureAwait(false);
            await storage.WriteAsync(SettingsPath, new ModeSettings(1, mode), cancellationToken).ConfigureAwait(false);
            lock (_sync) { _mode = mode; _preferred = mode; }
        }
        catch (Exception error) when (LauncherStorage.IsStorageError(error))
        { throw new LauncherOperationException("登录方式未保存，请检查数据目录的权限和配置格式。"); }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    public async Task SetOfflinePlayerNameAsync(string playerName, CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            IOfflineAccountService offline;
            lock (_sync) offline = _offline ?? throw new LauncherOperationException("离线登录插件尚未启用。");
            await offline.SetPlayerNameAsync(playerName, cancellationToken).ConfigureAwait(false);
        }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    public async Task SignInAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try { IAccountService provider; lock (_sync) provider = CurrentAccount; await provider.SignInAsync(cancellationToken).ConfigureAwait(false); }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    public async Task SignOutAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try { IAccountService provider; lock (_sync) provider = CurrentAccount; await provider.SignOutAsync(cancellationToken).ConfigureAwait(false); }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    public async Task<GameAccountSession> GetSessionAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            IGameAccountService provider;
            lock (_sync) provider = CurrentGameAccount;
            return await provider.GetSessionAsync(cancellationToken).ConfigureAwait(false);
        }
        finally { _gate.Release(); }
    }

    public void OnProviderStateChanged(AccountProviderStateChanged change)
    {
        lock (_sync) if (change.Mode != _mode) return;
        // Provider notifications may arrive inside an operation; forwarding must not make it wait on its own gate.
        try { _ = context.Run(_ => context.NotifyAsync(new AccountStateChanged(State))); }
        catch (ObjectDisposedException) { }
    }

    private async ValueTask UnregisterAsync(IOfflineAccountService provider)
    {
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            lock (_sync)
            {
                if (!ReferenceEquals(_offline, provider)) return;
                _offline = null;
                _mode = AccountLoginMode.Microsoft;
            }
        }
        finally { _gate.Release(); }
        await NotifyAsync().ConfigureAwait(false);
    }

    private async Task NotifyAsync()
    {
        await context.NotifyAsync(new AccountModesChanged(Snapshot)).ConfigureAwait(false);
        await context.NotifyAsync(new AccountStateChanged(State)).ConfigureAwait(false);
    }

    private sealed record ModeSettings(int SchemaVersion, AccountLoginMode Mode);
    private sealed class Registration(AccountRouter router, IOfflineAccountService provider) : IAsyncDisposable
    {
        private int _disposed;
        public ValueTask DisposeAsync() => Interlocked.Exchange(ref _disposed, 1) == 0 ? router.UnregisterAsync(provider) : ValueTask.CompletedTask;
    }
}
