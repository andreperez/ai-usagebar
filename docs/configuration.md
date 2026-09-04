# Configuration reference

The config file is `~/.config/ai-usagebar/config.toml`. All fields are optional.
Claude, Codex, Z.AI, and OpenRouter are enabled by default; other providers are
opt-in. The commented example shows the defaults and provider-specific
settings.

Open the terminal interface (TUI) Settings overlay with `s` to manage provider
activation visually. Saving Settings adds every missing provider section with
its default values while preserving existing comments, values, and inline keys.
Select a provider under **Dashboard providers** to set `enabled = true` and add
it to `[ui].active_vendors`. An API-key provider whose environment variable is
already configured can be enabled this way without copying its key into the
file.

```toml
[ui]
# Explicit automatic fetch/display scope. When set, only these configured,
# enabled providers appear in the TUI/Overview, `usage --json`, and widget
# cycling/default selection. Unlisted provider keys remain saved but are not
# fetched automatically. Omit to preserve legacy enabled-provider behavior.
# active_vendors = ["anthropic", "openai", "firecrawl"]
# Which active vendor the widget shows when --vendor is omitted, AND which tab
# is selected when the TUI opens. Defaults to the first active provider.
# Only an active vendor can be primary.
# primary = "anthropic"   # anthropic | anthropic_api | openai | copilot | zai
#                         # | openrouter | deepseek | kimi | kilo | novita
#                         # | moonshot | grok | supergrok | antigravity | cursor
#                         # | minimax | kiro | nous | opencode-go | tavily | firecrawl | requesty | zenmux
#                         # | vercel-ai-gateway | commandcode

[context]
enabled = false           # opt in, then press c in ai-usagebar-tui
# projects_path = "~/.claude/projects"
# context_window_tokens = 200000  # optional fallback denominator
# [context.model_context_window_tokens]
# "claude-opus-4-6" = 1000000    # exact model id overrides the fallback

[anthropic]
enabled = true
# credentials_path = "/home/you/.claude/.credentials.json"

[anthropic_api]
enabled = true             # disabled by default; requires an organization Admin key
api_key_env = "ANTHROPIC_ADMIN_KEY"
# api_key = "sk-ant-admin01-..."  # not an inference key; chmod 600 if inline
# monthly_limit = 1000     # optional positive, finite USD display limit

[openai]
enabled = true
# codex_auth_path = "/home/you/.codex/auth.json"

[copilot]
enabled = false           # opt in after `gh auth login --web`
# Uses `gh auth token`; GITHUB_COPILOT_TOKEN is an optional explicit override.

[zai]
enabled = true
api_key_env = "ZAI_API_KEY"
# api_key = "..."          # used if ZAI_API_KEY is unset; chmod 600 the file!
# plan_tier = "lite"       # lite | pro | max — display-only

[openrouter]
enabled = true
api_key_env = "OPENROUTER_API_KEY"
# api_key = "sk-or-v1-..."
# show_default_account = false  # hide default when named accounts exist

# [[openrouter.accounts]]
# label = "work"
# api_key_env = "OPENROUTER_WORK_API_KEY"
# api_key = "sk-or-v1-..."      # optional fallback; chmod 600 if inline

[deepseek]
enabled = true             # disabled by default; enable once you add an API key
api_key_env = "DEEPSEEK_API_KEY"
# api_key = "sk-..."       # used if DEEPSEEK_API_KEY is unset; chmod 600 the file!

[kimi]
enabled = true             # disabled by default; a Kimi Code CLI login is enough
# Log in with `kimi` and ai-usagebar reads the OAuth session the CLI already
# stored, refreshing it in place when it expires — no key to create or paste.
# An API key still wins when one is set; a Kimi For Coding subscription can
# issue one at kimi.com/code/console, and a platform key works too.
api_key_env = "KIMI_API_KEY"
# api_key = "sk-..."       # used if KIMI_API_KEY is unset; chmod 600 the file!
# credentials_path = "~/.kimi-code/credentials/kimi-code.json"  # CLI login file
# region = "auto"          # auto follows ~/.kimi-code/region
#                          # cn -> api.kimi.com | global -> api.kimi.ai

[minimax]
enabled = true             # disabled by default; enable once you add an API key
api_key_env = "MINIMAX_API_KEY"
# api_key = "..."          # used if MINIMAX_API_KEY is unset; chmod 600 the file!
# region = "global"        # global -> api.minimax.io | cn -> api.minimaxi.com

# --- Account-balance vendors (all opt-in) ---

[kilo]
enabled = true             # disabled by default; enable once you add an API key
api_key_env = "KILO_API_KEY"
# api_key = "..."          # used if KILO_API_KEY is unset; chmod 600 the file!
# organization_id = "org_..."   # team balance; omit for the personal balance

[novita]
enabled = true             # disabled by default; enable once you add an API key
api_key_env = "NOVITA_API_KEY"
# api_key = "..."          # used if NOVITA_API_KEY is unset; chmod 600 the file!

[moonshot]
enabled = true             # disabled by default; enable once you add an API key
api_key_env = "MOONSHOT_API_KEY"
# api_key = "sk-..."       # used if MOONSHOT_API_KEY is unset; chmod 600 the file!
# region = "global"        # global → api.moonshot.ai (USD) | cn → api.moonshot.cn (CNY)

[grok]
enabled = true             # disabled by default; enable once you add an API key
# The xAI *Management* key, NOT the inference key.
api_key_env = "XAI_MANAGEMENT_KEY"
# api_key = "..."          # used if XAI_MANAGEMENT_KEY is unset; chmod 600 the file!
# Required for organization-scoped keys; auto-resolved for team-scoped ones.
# team_id = "..."

[supergrok]
enabled = true             # disabled by default; enable once you've run `grok login`
# No API key of its own: billing comes from Grok Build's documented HTTPS
# endpoint using the `key` already in its auth.json (read-only, sent in one
# Authorization header, never copied or rewritten), or from its ACP process.
# Defaults to $GROK_HOME/bin/grok or ~/.grok/bin/grok. Override only when the
# trusted official binary was installed elsewhere.
# grok_binary = "/opt/grok/bin/grok"
# Cache-scope fingerprint inputs. config.toml is read as opaque bytes only;
# auth.json is also read for its billing `key`. Neither is copied or written.
# auth_path = "/home/you/.grok/auth.json"
# config_path = "/home/you/.grok/config.toml"

[cursor]
enabled = true             # disabled by default; enable once you've signed in to Cursor
# No API key: reads the session token the Cursor IDE already wrote to its own
# state.vscdb after you signed in there. No desktop IDE (headless machine)?
# Sign in to the cursor-agent CLI once instead — its own auth.json is the
# fallback when the IDE database is absent.
# db_path = "/home/you/.config/Cursor/User/globalStorage/state.vscdb"
# agent_auth_path = "/home/you/.config/cursor/auth.json"

[kiro]
enabled = true             # disabled by default; enable once you've run `kiro-cli login`
# No API key: reads the AWS SSO OIDC session kiro-cli already wrote to its own
# data.sqlite3 after you logged in there.
# db_path = "/home/you/.local/share/kiro-cli/data.sqlite3"

[tavily]
enabled = true             # disabled by default; enable once you add an API key
api_key_env = "TAVILY_API_KEY"
# api_key = "tvly-..."     # used if TAVILY_API_KEY is unset; chmod 600 the file!
# Optional project id: sent as the X-Project-ID header and folded into the
# cache-scope fingerprint so one project's usage is never served for another.
# project_id = "..."

[firecrawl]
enabled = true             # disabled by default; enable once you add an API key
api_key_env = "FIRECRAWL_API_KEY"
# api_key = "fc-..."       # used if FIRECRAWL_API_KEY is unset; chmod 600 the file!

[parallel]
enabled = true             # disabled by default; reads the parallel-cli OAuth session
access_token_env = "PARALLEL_API_KEY"  # override env; must hold a JWT-shaped Account API token
# access_token = "..."     # optional explicit Account API token override
# credentials_path = "..." # default: ~/.config/parallel-web-tools/auth.json
# Run `parallel-cli login` once. Access tokens live ~7 minutes, so ai-usagebar
# refreshes them via platform.parallel.ai and writes the rotated pair back.
# PARALLEL_API_KEY only counts when it holds a JWT-shaped Account API token;
# data API keys are rejected by the balance endpoint. Invoice organizations
# show billing mode rather than a fabricated zero prepaid balance.

[requesty]
enabled = true             # disabled by default; enable once you add a management API key
api_key_env = "REQUESTY_API_KEY"
# api_key = "..."          # used if REQUESTY_API_KEY is unset; chmod 600 the file!
# Reads organization balance and requests month-to-date usage with resolution=day.
# The optional usage endpoint is not grouped; a permission failure keeps balance visible.

[zenmux]
enabled = true             # disabled by default; requires a ZenMux Management API Key
api_key_env = "ZENMUX_MANAGEMENT_API_KEY"
# api_key = "..."          # used if ZENMUX_MANAGEMENT_API_KEY is unset; chmod 600 the file!
# Standard inference keys are rejected. PAYG and subscription endpoints refresh independently.

[vercel-ai-gateway]
enabled = true             # disabled by default; accepts an AI Gateway key or Vercel OIDC token
api_key_env = "AI_GATEWAY_API_KEY"
# api_key = "..."          # used if AI_GATEWAY_API_KEY is unset; chmod 600 the file!
# Custom reporting is beta, billed per query, and requires Pro or Enterprise.
report_enabled = false
report_cache_ttl_seconds = 21600  # 300..86400; separate report-cache TTL
```

For more than one OpenRouter key, see the
[OpenRouter account guide](openrouter-accounts.md). The existing singular
`[openrouter]` key remains the default account and needs no migration.

### GitHub Copilot

GitHub Copilot uses the OAuth login managed by the official GitHub CLI. Run
`gh auth login --web`, then select **GitHub Copilot** under **Primary Provider**
in the Omarchy settings form and save; this enables `[copilot]` and sets it as
the primary provider. The normal fetch path runs only the fixed structured
command `gh auth token`. ai-usagebar never parses GitHub CLI configuration or
credential stores and never writes the OAuth token to its config or cache.

`GITHUB_COPILOT_TOKEN` is an optional explicit environment override. It takes
precedence over `gh auth token`, which can be useful for a managed runtime that
provides its own short-lived token. Do not put that token in `config.toml`.

For more than one Codex login, add `[[openai.accounts]]` — a label and that
login's own `auth.json`, the same shape `[[anthropic.accounts]]` uses:

```toml
[[openai.accounts]]
label = "work"
codex_auth_path = "~/.config/ai-usagebar/accounts/work-codex/auth.json"
```

Create the second login with `CODEX_HOME=~/.codex-work codex login` and point
`codex_auth_path` at the file it writes. Select it with `--account work`; each
account caches separately under `~/.cache/ai-usagebar/openai/<label>`. The
singular `codex_auth_path` remains the default account and needs no migration.
