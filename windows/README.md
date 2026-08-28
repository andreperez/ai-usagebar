# AI Usage Bar for Windows

Native Windows 11 notification-area frontend for `ai-usagebar usage --json`.
It is a read-only presentation adapter: it never fetches provider APIs, reads
credentials, or writes `config.toml`.

## Requirements

- Windows 11.
- .NET 10 SDK.
- `ai-usagebar.exe` on `PATH`, or alongside the frontend executable.

## Build and Run

```powershell
dotnet build .\windows\AiUsageBar.Windows.csproj -c Release
dotnet run --project .\windows\AiUsageBar.Windows.csproj --configuration Release
```

Pass an explicit `ai-usagebar.exe` path as the first argument when it is not on
`PATH` and is not alongside the app:

```powershell
dotnet run --project .\windows\AiUsageBar.Windows.csproj --configuration Release -- "C:\Tools\ai-usagebar.exe"
```

## Behavior

- Uses the fixed `ai-usagebar usage --json` command without a shell.
- Limits the subprocess to 15 seconds and 1 MiB for each output stream.
- Refreshes every 60 seconds without overlapping a running refresh.
- Shows the configured primary provider, falling back to the first ready entry.
- Provides notification-area actions for dashboard, refresh, terminal user
  interface (TUI), and exit.

The terminal UI action requires `ai-usagebar-tui.exe` beside
`ai-usagebar.exe`.

## Validation

```powershell
dotnet build .\windows\AiUsageBar.Windows.csproj -c Release
dotnet run --project .\windows\tests\AiUsageBar.Windows.Tests.csproj -c Release
```
