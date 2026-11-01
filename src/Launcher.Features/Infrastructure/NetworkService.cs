using System.Net;
using Launcher.Contracts;

namespace Launcher.Plugins.Infrastructure;

/// <summary>Shared HTTPS transport for authentication and plugin services.</summary>
public sealed class NetworkService : INetworkService, IDisposable
{
    private readonly HttpClient _http;
    private readonly HttpClient _gameDirect;
    private readonly HttpClient _gameProxy;
    public NetworkService(HttpMessageHandler? handler = null)
    {
        _http = handler is null ? new HttpClient(new HttpClientHandler { AutomaticDecompression = DecompressionMethods.All }) : new HttpClient(handler);
        _http.Timeout = TimeSpan.FromSeconds(60);
        _http.DefaultRequestHeaders.UserAgent.ParseAdd("Hydrogen/1.0");
        _gameDirect = handler is null ? GameClient(useProxy: false) : _http;
        _gameProxy = handler is null ? GameClient(useProxy: true) : _http;
    }
    public async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        if (request.RequestUri is not { IsAbsoluteUri: true, Scheme: "https" }) throw new InvalidDataException("An HTTPS endpoint is required.");
        if (request.Method != HttpMethod.Get || !request.Options.TryGetValue(NetworkRequestOptions.GameDownload, out var game) || !game)
            return await _http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, cancellationToken).ConfigureAwait(false);
        using var directTimeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        directTimeout.CancelAfter(TimeSpan.FromSeconds(8));
        try { return await _gameDirect.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, directTimeout.Token).ConfigureAwait(false); }
        catch (Exception error) when (!cancellationToken.IsCancellationRequested && !ReferenceEquals(_gameDirect, _http)
            && error is HttpRequestException or OperationCanceledException)
        {
            if (HttpClient.DefaultProxy.IsBypassed(request.RequestUri)) throw new HttpRequestException("The direct game connection failed.", error);
            using var retry = new HttpRequestMessage(HttpMethod.Get, request.RequestUri);
            foreach (var header in request.Headers) retry.Headers.TryAddWithoutValidation(header.Key, header.Value);
            return await _gameProxy.SendAsync(retry, HttpCompletionOption.ResponseHeadersRead, cancellationToken).ConfigureAwait(false);
        }
    }
    private static HttpClient GameClient(bool useProxy)
    {
        var client = new HttpClient(new SocketsHttpHandler
        {
            UseProxy = useProxy, AllowAutoRedirect = false, ConnectTimeout = TimeSpan.FromSeconds(5),
            AutomaticDecompression = DecompressionMethods.All, MaxConnectionsPerServer = 32,
        }) { Timeout = TimeSpan.FromSeconds(60) };
        client.DefaultRequestHeaders.UserAgent.ParseAdd("Hydrogen/1.0");
        return client;
    }
    public void Dispose()
    {
        if (!ReferenceEquals(_gameDirect, _http)) _gameDirect.Dispose();
        if (!ReferenceEquals(_gameProxy, _http)) _gameProxy.Dispose();
        _http.Dispose();
    }
}
