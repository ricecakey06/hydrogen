using System.Text;
using System.Text.RegularExpressions;
using Launcher.Contracts;

namespace Launcher.Plugins.Infrastructure;

public sealed class DiagnosticsService(ILauncherStorage storage) : IDiagnosticsService
{
    public Task<ILauncherLog> CreateLogAsync(string category, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if (!Regex.IsMatch(category, "^[a-z-]{1,40}$")) throw new ArgumentException("Invalid log category.", nameof(category));
        var path = storage.Resolve(Path.Combine("logs", $"{DateTime.UtcNow:yyyyMMdd-HHmmss}-{category}-{Guid.NewGuid():N}.log"));
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        return Task.FromResult<ILauncherLog>(new RedactedLog(path));
    }

    public static string Redact(string message)
    {
        const string sensitive = "access[_-]?token|refresh[_-]?token|id[_-]?token|authorization|identityToken|RpsTicket|code_verifier|client_secret|password";
        var options = RegexOptions.IgnoreCase | RegexOptions.CultureInvariant;
        var redacted = Regex.Replace(message, "(?i)Bearer\\s+[^\\s\"',;]+", "Bearer [redacted]", options, TimeSpan.FromMilliseconds(200));
        redacted = Regex.Replace(redacted, "((?:--)?(?:" + sensitive + ")[\"']?\\s*[:=]?\\s*[\"']?)([^\\s\"',;&}]+)", "$1[redacted]", options, TimeSpan.FromMilliseconds(200));
        redacted = Regex.Replace(redacted, "[A-Za-z0-9_-]{12,}\\.[A-Za-z0-9_-]{12,}\\.[A-Za-z0-9_-]{12,}", "[redacted-jwt]", options, TimeSpan.FromMilliseconds(200));
        redacted = Regex.Replace(redacted, "([?&](?:code|state|token)=)[^&\\s]+", "$1[redacted]", options, TimeSpan.FromMilliseconds(200));
        return redacted;
    }

    private sealed class RedactedLog(string path) : ILauncherLog
    {
        private readonly SemaphoreSlim _gate = new(1, 1);
        private readonly object _sync = new();
        private readonly HashSet<string> _secrets = new(StringComparer.Ordinal);
        private StreamWriter? _writer = new(new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.Read, 4096, true), new UTF8Encoding(false)) { AutoFlush = true };
        private long _written;
        public string Path => path;
        public void AddSecrets(params string[] secrets)
        { lock (_sync) foreach (var secret in secrets.Where(s => !string.IsNullOrEmpty(s))) _secrets.Add(secret); }
        public async ValueTask WriteAsync(string message, CancellationToken cancellationToken = default)
        {
            await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
            try
            {
                if (_writer is null || _written >= 16 * 1024 * 1024) return;
                lock (_sync) foreach (var secret in _secrets.OrderByDescending(s => s.Length)) message = message.Replace(secret, "[redacted]", StringComparison.Ordinal);
                try { message = Redact(message); }
                catch (RegexMatchTimeoutException) { message = "[Log line omitted: redaction limit]"; }
                var line = $"{DateTimeOffset.UtcNow:O} {message}";
                _written += Encoding.UTF8.GetByteCount(line) + 1;
                await _writer.WriteLineAsync(line.AsMemory(), cancellationToken).ConfigureAwait(false);
            }
            finally { _gate.Release(); }
        }
        public async ValueTask DisposeAsync()
        {
            await _gate.WaitAsync().ConfigureAwait(false);
            try { if (_writer is not null) { await _writer.DisposeAsync().ConfigureAwait(false); _writer = null; } }
            finally { _gate.Release(); }
        }
    }
}
