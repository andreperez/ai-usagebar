# Plan: Windows Notification-Area Frontend

## Goal

Provide a native Windows 11 notification-area application that presents
`ai-usagebar usage --json` without fetching providers or managing credentials.

## Decision

Use C# with Windows Presentation Foundation (WPF) on .NET 10 and the built-in
Windows Forms `NotifyIcon` component. This is the smallest supported desktop
stack available in the current environment and avoids a third-party tray
library. The app remains a full-trust desktop process, as Windows tray icons
use the Shell notification-area API.

## Contracts

- Invoke a resolved `ai-usagebar` executable with exactly `usage --json`.
- Apply a bounded subprocess timeout and output-size limit.
- Deserialize only the documented additive report fields: primary, entries,
  identity fields, status, stale, fetched_at, metrics, and sections.
- Never fetch vendor APIs, read API keys, mutate `config.toml`, or execute
  provider-specific commands.
- Render the selected provider and its ordered sections in a read-only window.
- Keep a persistent notification-area icon with Refresh, Open dashboard, Open
  terminal UI, and Exit actions.
- Use the primary entry where available; otherwise select the first ready entry.
- Report missing binaries, invalid JSON, timeouts, and all-error responses in
  the UI without exposing subprocess command output beyond a bounded,
  sanitized diagnostic.

## Implementation Tasks

### 1. Project Foundation  ✅

- [x] Create `windows/AiUsageBar.Windows.csproj` for `net10.0-windows` WPF.
- [x] Create a test project for pure report parsing, executable resolution, and
  bounded subprocess behavior.
- [x] Add local build/test instructions and ignore generated `bin/`/`obj/`.

### 2. Report Client

- [ ] Resolve an explicit executable override, then a sibling executable, then
  `ai-usagebar` from PATH using no shell.
- [ ] Run `usage --json` with redirected streams, 15-second timeout, and 1 MiB
  stdout/stderr caps.
- [ ] Parse additive JSON safely and preserve ordered `sections`.

### 3. Notification UI

- [ ] Add a tray icon and accessible context menu.
- [ ] Show primary-provider summary in the tray tooltip.
- [ ] Render provider selection and ordered details in a WPF dashboard.
- [ ] Refresh on demand and at a bounded configurable interval without
  overlapping subprocesses.

### 4. Validation

- [ ] Run `dotnet test windows/AiUsageBar.Windows.sln`.
- [ ] Run `dotnet build windows/AiUsageBar.Windows.sln -c Release`.
- [ ] Verify no provider URL, credential environment variable, or TOML write is
  referenced by the Windows frontend.
- [ ] Retain `make test`, `make desktop-test`, `cargo machete`, and Windows
  Rust release build coverage for the shared binary.

## Rollback

The frontend is isolated under `windows/`; removing that directory and its
documentation leaves Rust providers, `usage --json`, and existing Linux/macOS
adapters unchanged.

## References

- [Shell_NotifyIcon](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicona)
- [NotifyIcon](https://learn.microsoft.com/en-us/dotnet/api/system.windows.forms.notifyicon)
- [WinUI 3](https://learn.microsoft.com/en-us/windows/apps/winui/winui3/)
