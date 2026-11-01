using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Launcher.Contracts;

namespace Launcher.Plugins.Authentication;

public sealed record MicrosoftTokens(string AccessToken, string RefreshToken)
{
    public override string ToString() => "Microsoft tokens (redacted)";
}

public interface IMicrosoftTokenProvider
{
    Task<MicrosoftTokens> SignInAsync(CancellationToken cancellationToken);
    Task<MicrosoftTokens> RefreshAsync(string refreshToken, CancellationToken cancellationToken);
}

/// <summary>Public-client authorization code flow, S256 PKCE, state validation and a bound IPv4 loopback listener.</summary>
public sealed class MicrosoftOAuthClient(INetworkService network, IPlatformEnvironment platform, string? clientId) : IMicrosoftTokenProvider
{
    private const string Scope = "XboxLive.signin offline_access";
    private const string TokenEndpoint = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";

    public async Task<MicrosoftTokens> SignInAsync(CancellationToken cancellationToken)
    {
        ValidateClientId();
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromMinutes(5));
        var token = timeout.Token;
        var verifier = Base64Url(RandomNumberGenerator.GetBytes(48));
        var challenge = Base64Url(SHA256.HashData(Encoding.ASCII.GetBytes(verifier)));
        var state = Base64Url(RandomNumberGenerator.GetBytes(32));
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start(4);
        try
        {
            var port = ((IPEndPoint)listener.LocalEndpoint).Port;
            var redirect = $"http://localhost:{port}/";
            var parameters = new Dictionary<string, string>
            {
                ["client_id"] = clientId!, ["response_type"] = "code", ["redirect_uri"] = redirect,
                ["response_mode"] = "query", ["scope"] = Scope, ["state"] = state,
                ["code_challenge"] = challenge, ["code_challenge_method"] = "S256", ["prompt"] = "select_account",
            };
            var query = string.Join('&', parameters.Select(pair => Uri.EscapeDataString(pair.Key) + "=" + Uri.EscapeDataString(pair.Value)));
            await platform.OpenBrowserAsync(new Uri("https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize?" + query), token).ConfigureAwait(false);
            while (true)
            {
                using var client = await listener.AcceptTcpClientAsync(token).ConfigureAwait(false);
                using var readTimeout = CancellationTokenSource.CreateLinkedTokenSource(token);
                readTimeout.CancelAfter(TimeSpan.FromSeconds(5));
                var result = await ReadCallbackAsync(client, state, port, readTimeout.Token).ConfigureAwait(false);
                if (result is null) continue;
                if (result.Error is not null) throw new AccountAuthenticationException(result.Error == "access_denied" ? AccountFailureKind.Cancelled : AccountFailureKind.Configuration);
                return await ExchangeAsync(new Dictionary<string, string>
                {
                    ["client_id"] = clientId!, ["grant_type"] = "authorization_code", ["code"] = result.Code!,
                    ["redirect_uri"] = redirect, ["code_verifier"] = verifier, ["scope"] = Scope,
                }, null, token).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        { throw new AccountAuthenticationException(AccountFailureKind.Cancelled); }
        finally { listener.Stop(); }
    }

    public Task<MicrosoftTokens> RefreshAsync(string refreshToken, CancellationToken cancellationToken)
    {
        ValidateClientId();
        return ExchangeAsync(new Dictionary<string, string>
        {
            ["client_id"] = clientId!, ["grant_type"] = "refresh_token", ["refresh_token"] = refreshToken, ["scope"] = Scope,
        }, refreshToken, cancellationToken);
    }

    private async Task<MicrosoftTokens> ExchangeAsync(Dictionary<string, string> fields, string? previousRefreshToken, CancellationToken token)
    {
        using var request = new HttpRequestMessage(HttpMethod.Post, TokenEndpoint) { Content = new FormUrlEncodedContent(fields) };
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token);
        timeout.CancelAfter(TimeSpan.FromSeconds(60));
        using var response = await network.SendAsync(request, timeout.Token).ConfigureAwait(false);
        await response.Content.LoadIntoBufferAsync(1024 * 1024, timeout.Token).ConfigureAwait(false);
        using var document = JsonDocument.Parse(await response.Content.ReadAsByteArrayAsync(timeout.Token).ConfigureAwait(false));
        if (!response.IsSuccessStatusCode)
        {
            var error = document.RootElement.TryGetProperty("error", out var kind) ? kind.GetString() : null;
            throw new AccountAuthenticationException(error is "invalid_client" or "unauthorized_client" or "invalid_scope" ? AccountFailureKind.Configuration : AccountFailureKind.Unexpected);
        }
        var root = document.RootElement;
        return new(root.GetProperty("access_token").GetString()!, root.TryGetProperty("refresh_token", out var refresh) ? refresh.GetString()! : previousRefreshToken ?? throw new AccountAuthenticationException(AccountFailureKind.Configuration));
    }

    private void ValidateClientId()
    { if (!Guid.TryParse(clientId, out var id) || id == Guid.Empty) throw new AccountAuthenticationException(AccountFailureKind.Configuration); }
    private static string Base64Url(byte[] bytes) => Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');

    public static IReadOnlyDictionary<string, string> ParseQuery(string query)
    {
        var result = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var pair in query.TrimStart('?').Split('&', StringSplitOptions.RemoveEmptyEntries))
        {
            var parts = pair.Split('=', 2);
            var key = Uri.UnescapeDataString(parts[0].Replace('+', ' '));
            if (!result.TryAdd(key, parts.Length == 2 ? Uri.UnescapeDataString(parts[1].Replace('+', ' ')) : "")) throw new FormatException("Duplicate callback parameter.");
        }
        return result;
    }

    private static async Task<Callback?> ReadCallbackAsync(TcpClient client, string expectedState, int port, CancellationToken token)
    {
        try
        {
            var stream = client.GetStream();
            var buffer = new byte[8192];
            var length = 0;
            while (length < buffer.Length)
            {
                var read = await stream.ReadAsync(buffer.AsMemory(length), token).ConfigureAwait(false);
                if (read == 0) return null;
                length += read;
                if (Encoding.ASCII.GetString(buffer, 0, length).Contains("\r\n\r\n", StringComparison.Ordinal)) break;
            }
            var headers = Encoding.ASCII.GetString(buffer, 0, length);
            var firstLine = headers.Split("\r\n", 2)[0].Split(' ');
            if (firstLine.Length != 3 || firstLine[0] != "GET" || !firstLine[1].StartsWith("/?", StringComparison.Ordinal)
                || !headers.Split("\r\n").Any(line => line.Equals($"Host: localhost:{port}", StringComparison.OrdinalIgnoreCase))) return null;
            var fields = ParseQuery(firstLine[1][1..]);
            var state = fields.GetValueOrDefault("state") ?? "";
            var valid = CryptographicOperations.FixedTimeEquals(Encoding.UTF8.GetBytes(state), Encoding.UTF8.GetBytes(expectedState));
            var code = fields.GetValueOrDefault("code");
            var error = fields.GetValueOrDefault("error");
            valid &= !string.IsNullOrEmpty(code) ^ !string.IsNullOrEmpty(error);
            const string body = "<!doctype html><meta charset=utf-8><title>Hydrogen</title><p>You can close this window and return to Hydrogen.</p>";
            var bytes = Encoding.UTF8.GetBytes(body);
            var response = Encoding.ASCII.GetBytes($"HTTP/1.1 {(valid ? "200 OK" : "400 Bad Request")}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {bytes.Length}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n");
            await stream.WriteAsync(response, token).ConfigureAwait(false);
            await stream.WriteAsync(bytes, token).ConfigureAwait(false);
            return valid ? new Callback(code, error) : null;
        }
        catch (Exception failure) when (failure is IOException or FormatException or OperationCanceledException) { return null; }
    }
    private sealed record Callback(string? Code, string? Error);
}
