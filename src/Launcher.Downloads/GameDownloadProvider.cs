using System.Net;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text.Json;
using Launcher.Contracts;

namespace Launcher.Downloads;

/// <summary>Downloads into temporary files and commits only verified content. Connections are shared across files.</summary>
public sealed class GameDownloadProvider(INetworkService network, ILauncherSettingsService? settings = null) : IGameDownloadProvider
{
    public static readonly Uri ManifestUri = new("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json");
    public const long ParallelDownloadThreshold = 8L * 1024 * 1024;
    private const int SegmentCount = 16;
    private readonly SemaphoreSlim _connections = new(32, 32);
    private readonly object _sourceSync = new();
    private DateTimeOffset _mirrorRetryAfter;

    private Uri ResolveSource(Uri original, GameDownloadSource source)
    {
        lock (_sourceSync)
            return source == GameDownloadSource.Bmclapi && original.Scheme == "https" && DateTimeOffset.UtcNow < _mirrorRetryAfter
                ? original : GameDownloadSources.Resolve(original, source);
    }

    public async Task<JsonElement> GetCatalogAsync(CancellationToken cancellationToken)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(60));
        var source = settings?.Settings.DownloadSource ?? GameDownloadSource.Official;
        for (var attempt = 0; ; attempt++)
        {
            try
            {
                using var response = await SendAsync(ResolveSource(ManifestUri, source), null, timeout.Token).ConfigureAwait(false);
                response.EnsureSuccessStatusCode();
                await response.Content.LoadIntoBufferAsync(16 * 1024 * 1024, timeout.Token).ConfigureAwait(false);
                using var document = JsonDocument.Parse(await response.Content.ReadAsByteArrayAsync(timeout.Token).ConfigureAwait(false));
                return document.RootElement.Clone();
            }
            catch (HttpRequestException error) when (attempt < 2 && !timeout.IsCancellationRequested
                && (error.StatusCode is null or HttpStatusCode.RequestTimeout or HttpStatusCode.TooManyRequests || error.StatusCode >= HttpStatusCode.InternalServerError))
            { await Task.Delay(TimeSpan.FromMilliseconds(300 * (attempt + 1)), timeout.Token).ConfigureAwait(false); }
        }
    }

    public async Task DownloadAsync(DownloadFile file, CancellationToken cancellationToken)
    {
        var source = settings?.Settings.DownloadSource ?? GameDownloadSource.Official;
        var uri = ResolveSource(file.Url, source);
        if (uri.Scheme != "https" || file.Sha1.Length != 40 || file.Sha1.Any(c => !Uri.IsHexDigit(c)) || file.Size is < 0)
            throw new InvalidDataException("Invalid download metadata.");
        try { await DownloadFromAsync(file, uri, cancellationToken).ConfigureAwait(false); }
        catch (HttpRequestException error) when (!cancellationToken.IsCancellationRequested && source == GameDownloadSource.Bmclapi
            && uri != file.Url && file.Url.Scheme == "https"
            && error.StatusCode is null or HttpStatusCode.NotFound or HttpStatusCode.Gone or HttpStatusCode.TooManyRequests)
        { await DownloadFromAsync(file, file.Url, cancellationToken).ConfigureAwait(false); }
    }

    private async Task DownloadFromAsync(DownloadFile file, Uri uri, CancellationToken cancellationToken)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(file.Destination)!);
        var parallel = file.Size >= ParallelDownloadThreshold;
        for (var attempt = 0; ; attempt++)
        {
            var temporary = file.Destination + "." + Guid.NewGuid().ToString("N") + ".part";
            try
            {
                using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                var seconds = file.Size is { } length ? Math.Clamp(length / 131072d + 10, 20, 600) : 60;
                timeout.CancelAfter(TimeSpan.FromSeconds(seconds));
                file.Progress?.Report(0);
                if (parallel)
                {
                    parallel = await DownloadSegmentsAsync(file, uri, temporary, timeout.Token).ConfigureAwait(false);
                    if (!parallel) { File.Delete(temporary); file.Progress?.Report(0); }
                }
                if (!parallel) await DownloadSingleAsync(file, uri, temporary, timeout.Token).ConfigureAwait(false);
                await using (var input = new FileStream(temporary, FileMode.Open, FileAccess.Read, FileShare.Read, 81920, true))
                {
                    if ((file.Size is { } size && input.Length != size)
                        || !Convert.ToHexString(await SHA1.HashDataAsync(input, timeout.Token).ConfigureAwait(false)).Equals(file.Sha1, StringComparison.OrdinalIgnoreCase))
                        throw new InvalidDataException("Download verification failed.");
                }
                cancellationToken.ThrowIfCancellationRequested();
                File.Move(temporary, file.Destination, overwrite: true);
                return;
            }
            catch (Exception error) when (attempt < 2 && !cancellationToken.IsCancellationRequested
                && (error is InvalidDataException or OperationCanceledException
                    || error is HttpRequestException http && (http.StatusCode is null or HttpStatusCode.RequestTimeout || http.StatusCode >= HttpStatusCode.InternalServerError)))
            { await Task.Delay(TimeSpan.FromMilliseconds(300 * (attempt + 1)), cancellationToken).ConfigureAwait(false); }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            { throw new HttpRequestException("Download timed out."); }
            finally { if (File.Exists(temporary)) File.Delete(temporary); }
        }
    }

    private async Task DownloadSingleAsync(DownloadFile file, Uri uri, string destination, CancellationToken token)
    {
        using var connection = await ConnectAsync(token).ConfigureAwait(false);
        using var response = await SendAsync(uri, null, token).ConfigureAwait(false);
        response.EnsureSuccessStatusCode();
        await using var input = await response.Content.ReadAsStreamAsync(token).ConfigureAwait(false);
        await using var output = new FileStream(destination, FileMode.CreateNew, FileAccess.Write, FileShare.None, 81920, true);
        var buffer = new byte[81920];
        long length = 0;
        int read;
        while ((read = await ReadAsync(input, buffer, token).ConfigureAwait(false)) > 0)
        {
            length += read;
            if (file.Size is { } size && length > size) throw new InvalidDataException("Download size does not match.");
            await output.WriteAsync(buffer.AsMemory(0, read), token).ConfigureAwait(false);
            file.Progress?.Report(length);
        }
        await output.FlushAsync(token).ConfigureAwait(false);
    }

    private async Task<bool> DownloadSegmentsAsync(DownloadFile file, Uri uri, string destination, CancellationToken token)
    {
        var size = file.Size!.Value;
        using var output = File.OpenHandle(destination, FileMode.CreateNew, FileAccess.Write, FileShare.None, FileOptions.Asynchronous | FileOptions.RandomAccess);
        RandomAccess.SetLength(output, size);
        var unsupported = 0;
        long completed = 0;
        var progressSync = new object();
        try
        {
            using (await ConnectAsync(token).ConfigureAwait(false))
            {
                using var probe = await SendAsync(uri, new RangeHeaderValue(0, 0), token).ConfigureAwait(false);
                if (probe.StatusCode is HttpStatusCode.OK or HttpStatusCode.RequestedRangeNotSatisfiable) return false;
                probe.EnsureSuccessStatusCode();
                var range = probe.Content.Headers.ContentRange;
                if (probe.StatusCode != HttpStatusCode.PartialContent || range is null || range.Unit != "bytes" || range.From != 0 || range.To != 0 || range.Length != size)
                    throw new InvalidDataException("Invalid range probe response.");
                uri = probe.RequestMessage?.RequestUri ?? uri;
            }
            await Parallel.ForEachAsync(Enumerable.Range(0, SegmentCount), new ParallelOptions { MaxDegreeOfParallelism = SegmentCount, CancellationToken = token }, async (segment, stopping) =>
            {
                var start = size * segment / SegmentCount;
                var end = size * (segment + 1) / SegmentCount - 1;
                using var connection = await ConnectAsync(stopping).ConfigureAwait(false);
                using var response = await SendAsync(uri, new RangeHeaderValue(start, end), stopping).ConfigureAwait(false);
                if (response.StatusCode is HttpStatusCode.OK or HttpStatusCode.RequestedRangeNotSatisfiable)
                {
                    Interlocked.Exchange(ref unsupported, 1);
                    throw new RangeUnsupportedException();
                }
                response.EnsureSuccessStatusCode();
                var range = response.Content.Headers.ContentRange;
                if (response.StatusCode != HttpStatusCode.PartialContent || range is null || !range.Unit.Equals("bytes", StringComparison.OrdinalIgnoreCase)
                    || range.From != start || range.To != end || range.Length != size
                    || (response.Content.Headers.ContentLength is { } length && length != end - start + 1)
                    || response.Content.Headers.ContentEncoding.Any(value => !value.Equals("identity", StringComparison.OrdinalIgnoreCase)))
                    throw new InvalidDataException("Invalid partial content response.");
                await using var input = await response.Content.ReadAsStreamAsync(stopping).ConfigureAwait(false);
                var buffer = new byte[81920];
                var position = start;
                int read;
                while ((read = await ReadAsync(input, buffer, stopping).ConfigureAwait(false)) > 0)
                {
                    if (position + read > end + 1) throw new InvalidDataException("Partial download is too long.");
                    await RandomAccess.WriteAsync(output, buffer.AsMemory(0, read), position, stopping).ConfigureAwait(false);
                    position += read;
                    lock (progressSync) file.Progress?.Report(completed += read);
                }
                if (position != end + 1) throw new InvalidDataException("Partial download is incomplete.");
            }).ConfigureAwait(false);
            return true;
        }
        catch (Exception) when (Volatile.Read(ref unsupported) != 0 && !token.IsCancellationRequested) { return false; }
    }

    private async Task<HttpResponseMessage> SendAsync(Uri uri, RangeHeaderValue? range, CancellationToken token)
    {
        for (var redirects = 0; ; redirects++)
        {
            uri = GameDownloadSources.ObjectEndpoint(uri);
            lock (_sourceSync)
                if (uri.Host == GameDownloadSources.BmclapiBaseUri.Host && DateTimeOffset.UtcNow < _mirrorRetryAfter)
                    throw new HttpRequestException("The mirror is observing Retry-After.", null, HttpStatusCode.TooManyRequests);
            using var request = new HttpRequestMessage(HttpMethod.Get, uri);
            request.Options.Set(NetworkRequestOptions.GameDownload, true);
            if (range is not null) { request.Headers.Range = range; request.Headers.AcceptEncoding.ParseAdd("identity"); }
            var response = await network.SendAsync(request, token).ConfigureAwait(false);
            response.RequestMessage ??= request;
            if (response.StatusCode == HttpStatusCode.TooManyRequests && uri.Host == GameDownloadSources.BmclapiBaseUri.Host)
            {
                var after = response.Headers.RetryAfter?.Date ?? DateTimeOffset.UtcNow.Add(response.Headers.RetryAfter?.Delta ?? TimeSpan.FromMinutes(1));
                lock (_sourceSync) if (after > _mirrorRetryAfter) _mirrorRetryAfter = after;
            }
            if (response.StatusCode is not (HttpStatusCode.MovedPermanently or HttpStatusCode.Redirect or HttpStatusCode.SeeOther or HttpStatusCode.TemporaryRedirect or HttpStatusCode.PermanentRedirect))
                return response;
            using (response)
            {
                if (redirects >= 8 || response.Headers.Location is not { } location)
                    throw new HttpRequestException("Invalid or repeated game download redirect.");
                var next = new Uri(response.RequestMessage.RequestUri!, location);
                if (next.Scheme != "https" || next.UserInfo.Length != 0) throw new InvalidDataException("An HTTPS game download redirect is required.");
                uri = next;
            }
        }
    }

    private async ValueTask<IDisposable> ConnectAsync(CancellationToken token)
    {
        await _connections.WaitAsync(token).ConfigureAwait(false);
        return new Connection(_connections);
    }
    private static async ValueTask<int> ReadAsync(Stream stream, byte[] buffer, CancellationToken token)
    {
        try { return await stream.ReadAsync(buffer, token).ConfigureAwait(false); }
        catch (IOException error) { throw new HttpRequestException("The download connection was interrupted.", error); }
    }
    private sealed class Connection(SemaphoreSlim gate) : IDisposable { public void Dispose() => gate.Release(); }
    private sealed class RangeUnsupportedException : Exception;
}
