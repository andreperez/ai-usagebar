# Together AI

Native provider for Together AI's documented beta organization billing API.
It reports finalized spend for the current billing month and the largest
product costs.

## Limitations

- Together must enable `GET /v1/billing/usage` for your organization. The
  endpoint is in beta and enabled per organization: request access through
  [Together support](https://portal.usepylon.com/together-ai/forms/support-request).
  An organization without access receives HTTP 404.
- The endpoint currently permits a project API key to read organization-wide
  billing. Together documents that organization-level keys and a mandatory
  `organization_id` parameter may be required before general availability.
- The API does not return the prepaid credit balance shown in the billing
  dashboard. ai-usagebar does not estimate it from spend.

## Configure

Keep the key in the environment:

```bash
export TOGETHER_API_KEY="..."
```

PowerShell:

```powershell
$env:TOGETHER_API_KEY = "..."
```

`api_key_env` accepts any variable name, so an existing environment variable
can be reused instead of exporting a new one. For example, with
`TOGETHER_AI_API_KEY` already set:

```toml
[together]
enabled = true
api_key_env = "TOGETHER_AI_API_KEY"
```

To use the default name instead:

```toml
[together]
enabled = true
api_key_env = "TOGETHER_API_KEY"
# api_key = "..."  # fallback only; protect the file when used
```

The default locations are `~/.config/ai-usagebar/config.toml` on Linux and
`%APPDATA%\ai-usagebar\config\config.toml` on Windows.

## Run

```bash
ai-usagebar --vendor together
ai-usagebar-tui
ai-usagebar usage --json
```

The widget's default text is the current-month spend. The tooltip and TUI show
the billing period, latest finalized window and up to five products ordered by
cost.

## Placeholders

| Placeholder | Value |
|---|---|
| `{vendor_short}` | `tgt` |
| `{together_spend}` | formatted month-to-date spend |
| `{together_period}` | billing month (`YYYY-MM`) |
| `{currency}` | response currency, currently `USD` |

## Troubleshooting

**HTTP 401.** The key is missing, malformed or revoked. Create a Together AI
API key and export it under the configured `api_key_env` name.

**HTTP 404.** Billing API beta access is not enabled for the organization.
Request access through
[Together support](https://portal.usepylon.com/together-ai/forms/support-request).
The web dashboard can still show billing through a private session-backed
service; that does not imply public API access.

**Spend differs from the dashboard temporarily.** The endpoint returns
finalized billing windows. The latest activity may not be finalized yet.

**Credit balance is absent.** This is intentional. No documented API-key
endpoint returns that field.

## Live validation

```bash
TOGETHER_API_KEY=… cargo test --test live together_live -- --ignored --nocapture
```

The ignored test uses a temporary cache and never modifies the normal user
cache.
