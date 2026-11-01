namespace Launcher.Contracts;

/// <summary>The currently selected account, provided by <c>account.manager</c>.</summary>
public interface IAccountService
{
    AccountState State { get; }

    /// <summary>Signs in through the selected provider. Microsoft uses the system browser.</summary>
    Task SignInAsync(CancellationToken cancellationToken);

    /// <summary>Signs out the current provider. Microsoft removes local credentials; game data is never touched.</summary>
    Task SignOutAsync(CancellationToken cancellationToken);
}

public enum AccountStatus { SignedOut, SigningIn, SignedIn, Expired }

public enum AccountFailureKind { Cancelled, Network, AccountRestricted, NoGameOwnership, Unexpected, Configuration, AppNotAuthorized }

/// <param name="SkinPng">The encoded skin texture, if available. The UI decides how to render it.</param>
public sealed record AccountProfile(string PlayerName, byte[]? SkinPng);

public sealed record AccountState(AccountStatus Status, AccountProfile? Profile, AccountFailureKind? LastFailure = null, string? Detail = null);

/// <summary>The selected game identity. Microsoft credentials are ownership-checked; local test identities are explicitly marked offline.</summary>
public interface IGameAccountService
{
    Task<GameAccountSession> GetSessionAsync(CancellationToken cancellationToken);
}

public sealed record GameAccountSession(string PlayerName, string Uuid, string AccessToken, string Xuid, string ClientId, DateTimeOffset ExpiresAt,
    AccountLoginMode Mode = AccountLoginMode.Microsoft)
{
    public string UserType => Mode == AccountLoginMode.Offline ? "legacy" : "msa";
    public override string ToString() => "Game account session (credentials redacted)";
}

public enum AccountLoginMode { Microsoft, Offline }

/// <summary>The Microsoft provider remains separate from the current-account router.</summary>
public interface IMicrosoftAccountService : IAccountService, IGameAccountService;

public interface IOfflineAccountService : IAccountService, IGameAccountService
{
    string PlayerName { get; }
    Task SetPlayerNameAsync(string playerName, CancellationToken cancellationToken);
}

public sealed record AccountModeSnapshot(bool OfflineAvailable, AccountLoginMode Mode, string OfflinePlayerName, string? Problem = null);

/// <summary>Always available; an optional provider registration makes the offline UI visible.</summary>
public interface IAccountModeService
{
    AccountModeSnapshot Snapshot { get; }
    Task SelectAsync(AccountLoginMode mode, CancellationToken cancellationToken);
    Task SetOfflinePlayerNameAsync(string playerName, CancellationToken cancellationToken);
}

/// <summary>The optional offline plugin registers a borrowed provider and owns the returned registration.</summary>
public interface IAccountProviderRegistry
{
    ValueTask<IAsyncDisposable> RegisterOfflineAsync(IOfflineAccountService provider, CancellationToken cancellationToken);
}

public sealed record AccountModesChanged(AccountModeSnapshot Snapshot);

/// <summary>Provider-local notifications; only the selected provider is forwarded as AccountStateChanged.</summary>
public sealed record AccountProviderStateChanged(AccountLoginMode Mode, AccountState State);

/// <summary>Published whenever <see cref="IAccountService.State"/> changes.</summary>
public sealed record AccountStateChanged(AccountState State);
