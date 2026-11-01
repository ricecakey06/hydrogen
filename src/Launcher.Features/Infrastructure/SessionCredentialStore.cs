using System.Security.Cryptography;
using Launcher.Contracts;

namespace Launcher.Plugins.Infrastructure;

/// <summary>Platforms without secure persistent storage keep credentials in memory, never plain-text files.</summary>
public sealed class SessionCredentialStore(ILauncherStorage? storage = null) : ICredentialStore, IDisposable
{
    private readonly object _sync = new();
    private byte[]? _value;
    private string? _root;
    public bool IsPersistent => false;
    public Task<byte[]?> ReadAsync(CancellationToken cancellationToken)
    { cancellationToken.ThrowIfCancellationRequested(); lock (_sync) { CheckRoot(); return Task.FromResult(_value?.ToArray()); } }
    public Task SaveAsync(byte[] credentials, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        lock (_sync) { CheckRoot(); if (_value is not null) CryptographicOperations.ZeroMemory(_value); _value = credentials.ToArray(); }
        return Task.CompletedTask;
    }
    public Task DeleteAsync(CancellationToken cancellationToken)
    { cancellationToken.ThrowIfCancellationRequested(); Dispose(); return Task.CompletedTask; }
    public void Dispose() { lock (_sync) { if (_value is not null) CryptographicOperations.ZeroMemory(_value); _value = null; } }
    private void CheckRoot()
    {
        if (storage is null || _root == storage.Root) return;
        if (_value is not null) CryptographicOperations.ZeroMemory(_value);
        _value = null;
        _root = storage.Root;
    }
}
