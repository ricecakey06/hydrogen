using Launcher.Plugins.Infrastructure;

namespace Launcher.Launch.Tests;

public sealed class DiagnosticsTests
{
    [Fact]
    public async Task Credential_redaction_covers_arguments_headers_json_and_registered_secrets()
    {
        await using var test = await TestScope.CreateAsync();
        var diagnostics = new DiagnosticsService(test.Storage);
        var log = await diagnostics.CreateLogAsync("game", default);
        log.AddSecrets("custom-secret-without-label");
        await log.WriteAsync("--accessToken game-secret Authorization: Bearer bearer-secret {\"refresh_token\":\"refresh-secret\"} custom-secret-without-label https://localhost/?code=auth-code&state=random-state");
        await log.DisposeAsync();
        var text = await File.ReadAllTextAsync(log.Path);
        foreach (var secret in new[] { "game-secret", "bearer-secret", "refresh-secret", "custom-secret-without-label", "auth-code", "random-state" }) Assert.DoesNotContain(secret, text);
    }
}
