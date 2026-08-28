using System.Diagnostics;
using System.IO;
using System.Text;

namespace AiUsageBar.Windows;

public sealed class ReportClient
{
    public const int OutputLimitBytes = 1_048_576;
    public static readonly TimeSpan Timeout = TimeSpan.FromSeconds(15);

    private readonly string? _explicitBinary;

    public ReportClient(string? explicitBinary = null)
    {
        _explicitBinary = string.IsNullOrWhiteSpace(explicitBinary) ? null : explicitBinary;
    }

    public async Task<UsageReport> FetchAsync(CancellationToken cancellationToken)
    {
        var binary = ResolveBinary(_explicitBinary);
        using var process = new Process {
            StartInfo = new ProcessStartInfo {
                FileName = binary,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true,
            },
        };
        process.StartInfo.ArgumentList.Add("usage");
        process.StartInfo.ArgumentList.Add("--json");
        if (!process.Start())
        {
            throw new InvalidOperationException("Could not start ai-usagebar.");
        }

        var stdoutTask = ReadCappedAsync(process.StandardOutput, OutputLimitBytes);
        var stderrTask = ReadCappedAsync(process.StandardError, OutputLimitBytes);
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(Timeout);
        try
        {
            await process.WaitForExitAsync(timeout.Token);
        }
        catch (OperationCanceledException)
        {
            TryKill(process);
            if (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            throw new TimeoutException("ai-usagebar usage --json timed out after 15 seconds.");
        }

        var stdout = await stdoutTask;
        var stderr = await stderrTask;
        if (stdout.Truncated || stderr.Truncated)
        {
            throw new InvalidDataException("ai-usagebar output exceeded the 1 MiB safety limit.");
        }
        if (process.ExitCode != 0)
        {
            throw new InvalidOperationException(SanitizeDiagnostic(stderr.Text));
        }
        return UsageReportParser.Parse(stdout.Text);
    }

    public static string ResolveBinary(string? explicitBinary)
    {
        if (!string.IsNullOrWhiteSpace(explicitBinary) && File.Exists(explicitBinary))
        {
            return explicitBinary;
        }

        var sibling = Path.Combine(AppContext.BaseDirectory, "ai-usagebar.exe");
        if (File.Exists(sibling))
        {
            return sibling;
        }

        foreach (var path in (Environment.GetEnvironmentVariable("PATH") ?? string.Empty).Split(Path.PathSeparator))
        {
            var directory = path.Trim().Trim('"');
            if (!Path.IsPathFullyQualified(directory))
            {
                continue;
            }
            var candidate = Path.Combine(directory, "ai-usagebar.exe");
            if (File.Exists(candidate))
            {
                return candidate;
            }
        }

        throw new FileNotFoundException("Could not find ai-usagebar.exe. Place it beside this app or add it to PATH.");
    }

    private static async Task<CappedText> ReadCappedAsync(StreamReader reader, int limit)
    {
        var buffer = new char[4096];
        var output = new StringBuilder(Math.Min(limit, 16_384));
        var count = 0;
        var truncated = false;
        while (true)
        {
            var read = await reader.ReadAsync(buffer.AsMemory());
            if (read == 0)
            {
                return new CappedText(output.ToString(), truncated);
            }
            var available = Math.Max(0, limit - count);
            if (read > available)
            {
                truncated = true;
            }
            if (available > 0)
            {
                output.Append(buffer, 0, Math.Min(read, available));
                count += Math.Min(read, available);
            }
        }
    }

    private static string SanitizeDiagnostic(string stderr)
    {
        var text = stderr.Trim();
        if (string.IsNullOrWhiteSpace(text))
        {
            return "ai-usagebar could not produce a usage report.";
        }
        return new string(text.Where(character => !char.IsControl(character) || character is '\n' or '\r' or '\t').Take(512).ToArray());
    }

    private static void TryKill(Process process)
    {
        try { process.Kill(entireProcessTree: true); } catch (InvalidOperationException) { } catch (System.ComponentModel.Win32Exception) { }
    }

    private sealed record CappedText(string Text, bool Truncated);
}
