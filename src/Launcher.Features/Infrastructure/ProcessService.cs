using System.Diagnostics;
using Launcher.Contracts;

namespace Launcher.Plugins.Infrastructure;

public sealed class ProcessService : IProcessService
{
    public IRunningProcess Start(ProcessRequest request, Func<ProcessOutput, ValueTask>? output = null)
    {
        var info = new ProcessStartInfo(request.Executable)
        {
            WorkingDirectory = request.WorkingDirectory,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var argument in request.Arguments) info.ArgumentList.Add(argument);
        foreach (var name in new[] { "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "CLASSPATH" }) info.Environment.Remove(name);
        if (request.Environment is not null)
            foreach (var (name, value) in request.Environment) info.Environment[name] = value;
        var process = new Process { StartInfo = info };
        try
        {
            if (!process.Start()) throw new IOException("The process did not start.");
            return new RunningProcess(process, output);
        }
        catch { process.Dispose(); throw; }
    }

    private sealed class RunningProcess : IRunningProcess
    {
        private readonly Process _process;
        private readonly CancellationTokenSource _stop = new();
        private readonly Task _stdout;
        private readonly Task _stderr;
        private int _disposed;
        public int Id { get; }

        internal RunningProcess(Process process, Func<ProcessOutput, ValueTask>? output)
        {
            _process = process;
            Id = process.Id;
            _stdout = PumpAsync(process.StandardOutput, false, output);
            _stderr = PumpAsync(process.StandardError, true, output);
        }

        private async Task PumpAsync(StreamReader reader, bool error, Func<ProcessOutput, ValueTask>? output)
        {
            try
            {
                while (await reader.ReadLineAsync(_stop.Token).ConfigureAwait(false) is { } line)
                    if (output is not null)
                    {
                        // A full disk or unavailable log must not stop draining the child's pipes.
                        try { await output(new ProcessOutput(line.Length > 65536 ? line[..65536] : line, error)).ConfigureAwait(false); }
                        catch (Exception failure) when (failure is IOException or UnauthorizedAccessException or ObjectDisposedException) { }
                    }
            }
            catch (OperationCanceledException) when (_stop.IsCancellationRequested) { }
            catch (IOException) when (_stop.IsCancellationRequested) { }
        }

        public async Task<int> WaitForExitAsync(CancellationToken cancellationToken)
        {
            await _process.WaitForExitAsync(cancellationToken).ConfigureAwait(false);
            await Task.WhenAll(_stdout, _stderr).WaitAsync(cancellationToken).ConfigureAwait(false);
            return _process.ExitCode;
        }

        public void Terminate()
        {
            try { if (!_process.HasExited) _process.Kill(entireProcessTree: true); }
            catch (InvalidOperationException) { }
        }

        public async ValueTask DisposeAsync()
        {
            if (Interlocked.Exchange(ref _disposed, 1) != 0) return;
            await _stop.CancelAsync().ConfigureAwait(false);
            await Task.WhenAll(_stdout, _stderr).ConfigureAwait(false);
            _process.Dispose();
            _stop.Dispose();
        }
    }
}
