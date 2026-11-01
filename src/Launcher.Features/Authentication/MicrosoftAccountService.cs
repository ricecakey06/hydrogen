using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Security.Cryptography;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.Runtime;

namespace Launcher.Plugins.Authentication;

public sealed class MicrosoftAccountService(Context context, INetworkService network, ICredentialStore credentials,
    ILauncherStorage storage, IMicrosoftTokenProvider microsoft, string? clientId) : IMicrosoftAccountService
{
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly object _sync = new();
    private AccountState _state = new(AccountStatus.SignedOut, null);
    private GameAccountSession? _session;
    private string? _refreshToken;
    private bool _saved = true;
    public AccountState State { get { lock (_sync) return _state; } }

    public async Task ResetForDataDirectoryAsync(CancellationToken token)
    {
        await _gate.WaitAsync(token).ConfigureAwait(false);
        try
        {
            _session = null;
            _refreshToken = null;
            await SetStateAsync(new(AccountStatus.SignedOut, null)).ConfigureAwait(false);
        }
        finally { _gate.Release(); }
        _ = context.Run(RestoreAsync);
    }

    public async Task RestoreAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            _session = null;
            _refreshToken = null;
            if (!Guid.TryParse(clientId, out var configured) || configured == Guid.Empty)
            {
                await SetStateAsync(new(AccountStatus.SignedOut, null, AccountFailureKind.Configuration)).ConfigureAwait(false);
                return;
            }
            var bytes = await credentials.ReadAsync(cancellationToken).ConfigureAwait(false);
            if (bytes is null)
            { await SetStateAsync(new(AccountStatus.SignedOut, null)).ConfigureAwait(false); return; }
            RefreshCredential? saved;
            try { saved = JsonSerializer.Deserialize<RefreshCredential>(bytes); }
            finally { CryptographicOperations.ZeroMemory(bytes); }
            if (saved is null || saved.SchemaVersion != 1 || saved.ClientId != clientId || string.IsNullOrWhiteSpace(saved.RefreshToken))
                throw new AccountAuthenticationException(AccountFailureKind.Configuration);
            _refreshToken = saved.RefreshToken;
            await SetStateAsync(new(AccountStatus.SigningIn, null)).ConfigureAwait(false);
            await CompleteAsync(await microsoft.RefreshAsync(_refreshToken, cancellationToken).ConfigureAwait(false), cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { throw; }
        catch (Exception error) when (IsExpected(error))
        { await SetStateAsync(new(AccountStatus.Expired, null, FailureKind(error))).ConfigureAwait(false); }
        finally { _gate.Release(); }
    }

    public async Task SignInAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            _session = null;
            await SetStateAsync(new(AccountStatus.SigningIn, null)).ConfigureAwait(false);
            await CompleteAsync(await microsoft.SignInAsync(cancellationToken).ConfigureAwait(false), cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            await SetStateAsync(new(AccountStatus.SignedOut, null, AccountFailureKind.Cancelled)).ConfigureAwait(false);
            throw;
        }
        catch (Exception error) when (IsExpected(error))
        { await SetStateAsync(new(AccountStatus.SignedOut, null, FailureKind(error))).ConfigureAwait(false); }
        finally { _gate.Release(); }
    }

    public async Task SignOutAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            try { await credentials.DeleteAsync(cancellationToken).ConfigureAwait(false); }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            { throw new LauncherOperationException("无法删除本地登录凭据，请检查数据目录权限后重试。"); }
            _session = null;
            _refreshToken = null;
            await SetStateAsync(new(AccountStatus.SignedOut, null)).ConfigureAwait(false);
        }
        finally { _gate.Release(); }
    }

    public async Task<GameAccountSession> GetSessionAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            using var usage = storage.AcquireUsage();
            if (_session is { } session && session.ExpiresAt > DateTimeOffset.UtcNow.AddMinutes(2)) return session;
            if (_refreshToken is null) throw new AccountAuthenticationException(AccountFailureKind.Unexpected);
            try
            {
                await CompleteAsync(await microsoft.RefreshAsync(_refreshToken, cancellationToken).ConfigureAwait(false), cancellationToken).ConfigureAwait(false);
                return _session!;
            }
            catch (Exception error) when (IsExpected(error) && error is not OperationCanceledException)
            {
                _session = null;
                var kind = FailureKind(error);
                await SetStateAsync(new(AccountStatus.Expired, null, kind)).ConfigureAwait(false);
                throw new AccountAuthenticationException(kind);
            }
        }
        finally { _gate.Release(); }
    }

    private async Task CompleteAsync(MicrosoftTokens tokens, CancellationToken token)
    {
        _refreshToken = tokens.RefreshToken;
        _saved = await SaveRefreshAsync(token).ConfigureAwait(false);
        var xbox = await PostAsync("https://user.auth.xboxlive.com/user/authenticate", new
        {
            Properties = new { AuthMethod = "RPS", SiteName = "user.auth.xboxlive.com", RpsTicket = "d=" + tokens.AccessToken },
            RelyingParty = "http://auth.xboxlive.com", TokenType = "JWT",
        }, AccountFailureKind.AccountRestricted, token).ConfigureAwait(false);
        var xsts = await PostAsync("https://xsts.auth.xboxlive.com/xsts/authorize", new
        {
            Properties = new { SandboxId = "RETAIL", UserTokens = new[] { xbox.GetProperty("Token").GetString()! } },
            RelyingParty = "rp://api.minecraftservices.com/", TokenType = "JWT",
        }, AccountFailureKind.AccountRestricted, token).ConfigureAwait(false);
        var claim = xsts.GetProperty("DisplayClaims").GetProperty("xui")[0];
        var userHash = claim.GetProperty("uhs").GetString()!;
        var xuid = claim.TryGetProperty("xid", out var xid) ? xid.GetString() ?? "" : "";
        var game = await PostAsync("https://api.minecraftservices.com/authentication/login_with_xbox",
            new { identityToken = $"XBL3.0 x={userHash};{xsts.GetProperty("Token").GetString()}" }, AccountFailureKind.AppNotAuthorized, token).ConfigureAwait(false);
        var accessToken = game.GetProperty("access_token").GetString()!;
        var expiresIn = game.GetProperty("expires_in").GetInt32();
        if (expiresIn <= 0 || string.IsNullOrWhiteSpace(accessToken)) throw new AccountAuthenticationException(AccountFailureKind.Unexpected);
        var ownership = await GetAuthenticatedAsync("https://api.minecraftservices.com/entitlements/mcstore", accessToken, token).ConfigureAwait(false);
        if (!ownership.TryGetProperty("items", out var items) || !items.EnumerateArray().Any(item =>
                item.TryGetProperty("name", out var name) && name.GetString() is "game_minecraft" or "product_minecraft"))
            throw new AccountAuthenticationException(AccountFailureKind.NoGameOwnership);
        var profile = await GetAuthenticatedAsync("https://api.minecraftservices.com/minecraft/profile", accessToken, token).ConfigureAwait(false);
        var playerName = profile.GetProperty("name").GetString()!;
        var uuid = profile.GetProperty("id").GetString()!;
        if (!Guid.TryParseExact(uuid, "N", out _) || string.IsNullOrWhiteSpace(playerName)) throw new AccountAuthenticationException(AccountFailureKind.Unexpected);
        var skin = await TrySkinAsync(profile, token).ConfigureAwait(false);
        _session = new(playerName, uuid, accessToken, xuid, clientId!, DateTimeOffset.UtcNow.AddSeconds(Math.Min(expiresIn, 86400)));
        await SetStateAsync(new(AccountStatus.SignedIn, new(playerName, skin), Detail: !_saved || !credentials.IsPersistent ? "本次登录仅在当前会话有效。" : null)).ConfigureAwait(false);
    }

    private async Task<bool> SaveRefreshAsync(CancellationToken token)
    {
        var bytes = JsonSerializer.SerializeToUtf8Bytes(new RefreshCredential(1, clientId!, _refreshToken!));
        try
        {
            await credentials.SaveAsync(bytes, token).ConfigureAwait(false);
            return true;
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or CryptographicException) { return false; }
        finally { CryptographicOperations.ZeroMemory(bytes); }
    }

    private async Task<JsonElement> PostAsync(string uri, object body, AccountFailureKind failure, CancellationToken token)
    {
        using var request = new HttpRequestMessage(HttpMethod.Post, uri) { Content = JsonContent.Create(body, options: new JsonSerializerOptions()) };
        return await ReadResponseAsync(request, failure, token).ConfigureAwait(false);
    }

    private async Task<JsonElement> GetAuthenticatedAsync(string uri, string accessToken, CancellationToken token)
    {
        using var request = new HttpRequestMessage(HttpMethod.Get, uri);
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", accessToken);
        return await ReadResponseAsync(request, AccountFailureKind.NoGameOwnership, token).ConfigureAwait(false);
    }

    private async Task<JsonElement> ReadResponseAsync(HttpRequestMessage request, AccountFailureKind failure, CancellationToken token)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token);
        timeout.CancelAfter(TimeSpan.FromSeconds(60));
        using var response = await network.SendAsync(request, timeout.Token).ConfigureAwait(false);
        if ((int)response.StatusCode >= 500 || response.StatusCode == HttpStatusCode.TooManyRequests) throw new HttpRequestException("Account service unavailable.");
        if (!response.IsSuccessStatusCode) throw new AccountAuthenticationException(failure);
        await response.Content.LoadIntoBufferAsync(1024 * 1024, timeout.Token).ConfigureAwait(false);
        using var document = JsonDocument.Parse(await response.Content.ReadAsByteArrayAsync(timeout.Token).ConfigureAwait(false));
        return document.RootElement.Clone();
    }

    private async Task<byte[]?> TrySkinAsync(JsonElement profile, CancellationToken token)
    {
        if (!profile.TryGetProperty("skins", out var skins) || skins.GetArrayLength() == 0) return null;
        var source = skins.EnumerateArray().FirstOrDefault(skin => skin.TryGetProperty("state", out var state) && state.GetString() == "ACTIVE");
        if (source.ValueKind == JsonValueKind.Undefined || !Uri.TryCreate(source.GetProperty("url").GetString(), UriKind.Absolute, out var uri) || uri.Host != "textures.minecraft.net") return null;
        try
        {
            uri = new UriBuilder(uri) { Scheme = "https", Port = -1 }.Uri;
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token);
            timeout.CancelAfter(TimeSpan.FromSeconds(10));
            using var request = new HttpRequestMessage(HttpMethod.Get, uri);
            using var response = await network.SendAsync(request, timeout.Token).ConfigureAwait(false);
            if (!response.IsSuccessStatusCode) return null;
            await response.Content.LoadIntoBufferAsync(1024 * 1024, timeout.Token).ConfigureAwait(false);
            return await response.Content.ReadAsByteArrayAsync(timeout.Token).ConfigureAwait(false);
        }
        catch (Exception error) when (!token.IsCancellationRequested && error is HttpRequestException or OperationCanceledException) { return null; }
    }

    private Task SetStateAsync(AccountState state)
    { lock (_sync) _state = state; return context.NotifyAsync(new AccountProviderStateChanged(AccountLoginMode.Microsoft, state)); }
    private static bool IsExpected(Exception error) => error is AccountAuthenticationException or HttpRequestException or IOException or UnauthorizedAccessException
        or JsonException or CryptographicException or OperationCanceledException or System.ComponentModel.Win32Exception or System.Net.Sockets.SocketException or KeyNotFoundException or InvalidOperationException;
    private static AccountFailureKind FailureKind(Exception error) => error switch
    { AccountAuthenticationException auth => auth.Kind, HttpRequestException or OperationCanceledException => AccountFailureKind.Network, _ => AccountFailureKind.Unexpected };
    private sealed record RefreshCredential(int SchemaVersion, string ClientId, string RefreshToken)
    { public override string ToString() => "Refresh credential (redacted)"; }
}
