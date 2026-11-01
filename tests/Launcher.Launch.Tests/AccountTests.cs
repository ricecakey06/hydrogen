using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Launcher.Contracts;
using Launcher.Platform.Windows;
using Launcher.Plugins.Authentication;
using Launcher.Plugins.Infrastructure;

namespace Launcher.Launch.Tests;

public sealed class AccountTests
{
    private const string ClientId = "11111111-2222-3333-4444-555555555555";

    [Fact]
    public async Task Browser_login_uses_pkce_checks_state_and_exchanges_the_matching_code()
    {
        var platform = new StubPlatform();
        IReadOnlyDictionary<string, string>? authorize = null;
        Task? callbacks = null;
        using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        using var browser = new HttpClient(new HttpClientHandler { UseProxy = false });
        platform.OnBrowser = (uri, token) =>
        {
            authorize = MicrosoftOAuthClient.ParseQuery(uri.Query);
            callbacks = Task.Run(async () =>
            {
                using var wrong = await browser.GetAsync(authorize["redirect_uri"] + "?code=wrong&state=forged", token);
                Assert.Equal(HttpStatusCode.BadRequest, wrong.StatusCode);
                using var good = await browser.GetAsync(authorize["redirect_uri"] + "?code=correct-code&state=" + authorize["state"], token);
                Assert.Equal(HttpStatusCode.OK, good.StatusCode);
            }, token);
            return Task.CompletedTask;
        };
        using var network = new NetworkService(new ScriptedHandler(async (request, token) =>
        {
            var form = MicrosoftOAuthClient.ParseQuery(await request.Content!.ReadAsStringAsync(token));
            Assert.Equal("correct-code", form["code"]);
            Assert.NotNull(authorize);
            var challenge = Convert.ToBase64String(SHA256.HashData(Encoding.ASCII.GetBytes(form["code_verifier"]))).TrimEnd('=').Replace('+', '-').Replace('/', '_');
            Assert.Equal(challenge, authorize["code_challenge"]);
            Assert.Equal("S256", authorize["code_challenge_method"]);
            Assert.Equal(authorize["redirect_uri"], form["redirect_uri"]);
            Assert.Equal(ClientId, form["client_id"]);
            Assert.DoesNotContain("client_secret", form.Keys);
            return ScriptedHandler.Json("""{"access_token":"ms-access","refresh_token":"ms-refresh"}""");
        }));
        var tokens = await new MicrosoftOAuthClient(network, platform, ClientId).SignInAsync(cancellation.Token);
        await callbacks!;
        Assert.Equal("ms-access", tokens.AccessToken);
        Assert.Equal("ms-refresh", tokens.RefreshToken);
        Assert.DoesNotContain(tokens.AccessToken, tokens.ToString());
    }

    [Fact]
    public async Task Missing_client_id_fails_before_opening_a_browser()
    {
        var opened = false;
        var platform = new StubPlatform { OnBrowser = (_, _) => { opened = true; return Task.CompletedTask; } };
        using var network = new NetworkService(new ScriptedHandler((_, _) => throw new InvalidOperationException("No request expected.")));
        var error = await Assert.ThrowsAsync<AccountAuthenticationException>(() => new MicrosoftOAuthClient(network, platform, null).SignInAsync(default));
        Assert.Equal(AccountFailureKind.Configuration, error.Kind);
        Assert.False(opened);
    }

    [Fact]
    public async Task Complete_account_chain_checks_ownership_restores_login_and_signs_out_without_touching_instances()
    {
        await using var test = await TestScope.CreateAsync();
        var instance = await test.Instances.CreateAsync("26.1.2", default);
        using var credentials = new SessionCredentialStore();
        using var network = AccountNetwork();
        var microsoft = new StubMicrosoft();
        var account = new MicrosoftAccountService(test.Context, network, credentials, test.Storage, microsoft, ClientId);
        await account.SignInAsync(default);
        Assert.Equal(AccountStatus.SignedIn, account.State.Status);
        Assert.Equal("TestPlayer", account.State.Profile!.PlayerName);
        var session = await account.GetSessionAsync(default);
        Assert.Equal("game-access-secret", session.AccessToken);
        Assert.DoesNotContain(session.AccessToken, session.ToString());
        var restored = new MicrosoftAccountService(test.Context, network, credentials, test.Storage, microsoft, ClientId);
        await restored.RestoreAsync(default);
        Assert.Equal(1, microsoft.RefreshCalls);
        Assert.Equal(AccountStatus.SignedIn, restored.State.Status);
        await restored.SignOutAsync(default);
        Assert.Null(await credentials.ReadAsync(default));
        Assert.Equal(AccountStatus.SignedOut, restored.State.Status);
        Assert.True(Directory.Exists(test.Instances.GetGameDirectory(instance.Id)));
    }

    [Theory]
    [InlineData(true, false, AccountFailureKind.AppNotAuthorized)]
    [InlineData(false, true, AccountFailureKind.NoGameOwnership)]
    public async Task Missing_app_access_and_missing_game_ownership_have_distinct_failures(bool denied, bool noOwnership, AccountFailureKind expected)
    {
        await using var test = await TestScope.CreateAsync();
        using var network = AccountNetwork(denied, noOwnership);
        using var credentials = new SessionCredentialStore();
        var account = new MicrosoftAccountService(test.Context, network, credentials, test.Storage, new StubMicrosoft(), ClientId);
        await account.SignInAsync(default);
        Assert.Equal(AccountStatus.SignedOut, account.State.Status);
        Assert.Equal(expected, account.State.LastFailure);
        await Assert.ThrowsAsync<AccountAuthenticationException>(() => account.GetSessionAsync(default));
    }

    [Fact]
    public async Task Windows_credentials_are_encrypted_for_the_current_user_and_can_be_deleted()
    {
        if (!OperatingSystem.IsWindows()) return;
        await using var test = await TestScope.CreateAsync();
        var store = new WindowsCredentialStore(test.Storage);
        var secret = Encoding.UTF8.GetBytes("refresh-token-secret-123456");
        await store.SaveAsync(secret, default);
        Assert.Equal(secret, await store.ReadAsync(default));
        var raw = await File.ReadAllBytesAsync(Path.Combine(test.Root, "credentials.bin"));
        Assert.DoesNotContain("refresh-token-secret-123456", Encoding.UTF8.GetString(raw));
        Assert.True(store.IsPersistent);
        await store.DeleteAsync(default);
        Assert.Null(await store.ReadAsync(default));
    }

    [Fact]
    public void Ambiguous_oauth_callbacks_are_rejected()
        => Assert.Throws<FormatException>(() => MicrosoftOAuthClient.ParseQuery("?code=a&state=one&state=two"));

    private static NetworkService AccountNetwork(bool denied = false, bool noOwnership = false) => new(new ScriptedHandler(async (request, token) =>
    {
        switch (request.RequestUri!.Host + request.RequestUri.AbsolutePath)
        {
            case "user.auth.xboxlive.com/user/authenticate":
                using (var body = JsonDocument.Parse(await request.Content!.ReadAsStringAsync(token)))
                    Assert.Equal("d=ms-access-secret", body.RootElement.GetProperty("Properties").GetProperty("RpsTicket").GetString());
                return ScriptedHandler.Json("""{"Token":"xbox-token","DisplayClaims":{"xui":[{"uhs":"user-hash"}]}}""");
            case "xsts.auth.xboxlive.com/xsts/authorize":
                return ScriptedHandler.Json("""{"Token":"xsts-token","DisplayClaims":{"xui":[{"uhs":"user-hash","xid":"xbox-user-id"}]}}""");
            case "api.minecraftservices.com/authentication/login_with_xbox":
                if (denied) return ScriptedHandler.Json("{}", HttpStatusCode.Forbidden);
                using (var body = JsonDocument.Parse(await request.Content!.ReadAsStringAsync(token)))
                    Assert.Equal("XBL3.0 x=user-hash;xsts-token", body.RootElement.GetProperty("identityToken").GetString());
                return ScriptedHandler.Json("""{"access_token":"game-access-secret","expires_in":3600}""");
            case "api.minecraftservices.com/entitlements/mcstore":
                Assert.Equal("game-access-secret", request.Headers.Authorization?.Parameter);
                return ScriptedHandler.Json(noOwnership ? "{\"items\":[]}" : "{\"items\":[{\"name\":\"game_minecraft\"}]}");
            case "api.minecraftservices.com/minecraft/profile":
                Assert.Equal("game-access-secret", request.Headers.Authorization?.Parameter);
                return ScriptedHandler.Json("""{"id":"123456781234123412341234567890ab","name":"TestPlayer","skins":[]}""");
            default: throw new InvalidOperationException("Unexpected test request.");
        }
    }));
    private sealed class StubMicrosoft : IMicrosoftTokenProvider
    {
        public int RefreshCalls { get; private set; }
        public Task<MicrosoftTokens> SignInAsync(CancellationToken cancellationToken) => Task.FromResult(new MicrosoftTokens("ms-access-secret", "ms-refresh-secret"));
        public Task<MicrosoftTokens> RefreshAsync(string refreshToken, CancellationToken cancellationToken)
        { RefreshCalls++; Assert.Equal("ms-refresh-secret", refreshToken); return SignInAsync(cancellationToken); }
    }
}
