# Vendor endpoints and live tests

Some providers do not publish a stable usage API. ai-usagebar keeps its parsers
defensive and includes opt-in live tests for catching response changes.

## Support matrix

| Vendor | Endpoint | What you see | Native desktop selector (v0.13) |
|---|---|---|---|
| **Claude** | `api.anthropic.com/api/oauth/usage` (undocumented) | Session (5h), Weekly (7d), model-scoped weekly (e.g. Fable), Extra usage $ | Yes |
| **Codex** | `chatgpt.com/backend-api/wham/usage` (undocumented; used by official `codex` CLI) | Codex 5h and/or weekly, Code-review weekly, Credits | Yes |
| **GitHub Copilot** | `api.github.com/copilot_internal/user` (private; used by VS Code) | Premium requests, Chat, and Completions quota %, counts when supplied, plan, reset | Yes |
| **Z.AI** | `api.z.ai/api/monitor/usage/quota/limit` (undocumented) | Session 5h, Weekly 7d, MCP tools monthly | Yes |
| **OpenRouter** | `openrouter.ai/api/v1/{credits,key}` (documented) | Balance, today/week/month spend, free vs paid tier | Yes |
| **DeepSeek** | `api.deepseek.com/user/balance` (documented) | Balance, granted, topped-up credits | Yes |
| **Kimi** | `api.kimi.com\|.ai/coding/v1/usages` (undocumented; community-confirmed), plus `auth.kimi.com\|.ai/api/oauth/token` to refresh a Kimi Code CLI login | Weekly subscription quota + 5h rolling rate-limit window | No — widget/TUI only; desktop protocol and marker parity are future work |
| **MiniMax** | `api.minimax.io/v1/token_plan/remains` (official Token Plan quota route) | Token Plan rolling interval window + weekly, per model bucket (text, video) | No — widget/TUI only |
| **Kilo** | `api.kilo.ai/api/profile/balance` (undocumented; extension-internal) | Remaining credit balance ($) | No — widget/TUI only |
| **Novita** | `api.novita.ai/openapi/v1/billing/balance/detail` (documented) | Remaining credit balance ($) | No — widget/TUI only |
| **Moonshot** | `api.moonshot.ai\|.cn/v1/users/me/balance` (documented) | Account balance ($ on `.ai`, ¥ on `.cn`) | No — widget/TUI only |
| **Grok (xAI)** | `management-api.x.ai/v1/billing/teams/{team}/prepaid/balance` (Management API; documented) | Prepaid credit balance ($) | No — widget/TUI only |
| **SuperGrok** | `cli-chat-proxy.grok.com/v1/billing` with the Grok Build login's key, falling back to its `x.ai/billing` ACP extension | Current weekly/monthly included-credit %, prepaid API balance, reset | No — widget/TUI only |
| **Anthropic API** | `api.anthropic.com/v1/organizations/cost_report` (Admin API; documented) | Month-to-date spend ($, excludes Priority Tier), optional spend-vs-limit % | No — widget/TUI only |
| **Google Antigravity** | A loopback RPC on the local Antigravity product's own port, discovered from `/proc` (Linux), `lsof` (macOS), or the process/TCP tables (Windows); no remote endpoint and no credential | Whichever quota windows the running product reports — Gemini and Claude/GPT pools, 5-hour and weekly | Yes |
| **Cursor** | `cursor.com/api/usage-summary` (undocumented; the dashboard's own frontend) | Two included-usage pools this billing cycle — Cursor Models (Auto/Composer) % and Other Models (named/API) % — plus plan, reset, on-demand | Yes |
| **Kiro CLI** | `codewhisperer.<region>.amazonaws.com` `GetUsageLimits` (undocumented; the same call kiro-cli's own `/usage` slash command makes) | Single credit pool this cycle — used/limit/%, plan, reset | No — widget/TUI only |
| **Nous Research** | `portal.nousresearch.com/api/oauth/account` (OAuth-authenticated Portal account response) | Subscription usage %, subscription credits, top-up/purchased credits, total usable credits, renewal | Yes |
| **OpenCode Go** | `opencode.ai/zen/go/v1/usage` | Rolling, weekly, and monthly `percent` windows with absolute reset timestamps | Yes |
| **Tavily** | `api.tavily.com/usage` (documented; optional `X-Project-ID` header) | Plan billing-cycle used/limit (%), pay-as-you-go, this key's used/limit, per-endpoint breakdown (search/extract/crawl/map/research) | Yes |
| **Firecrawl** | `api.firecrawl.dev/v2/team/credit-usage` + `/team/credit-usage/historical?byApiKey=false` (documented) | Remaining credits, plan credits, matching billing-period usage %, billing-period reset | Yes |
| **Parallel** | `api.parallel.ai/account/service/v1/balance` (documented Account API) | Prepaid balance and pending debit, or invoice billing mode | Yes |
| **Requesty** | `api-v2.requesty.ai/v1/manage/org` + `/org/usage?start=<RFC3339>&end=<RFC3339>&resolution=day` (documented) | Organization balance plus optional month-to-date spend, request, and token totals | Yes |
| **ZenMux** | `zenmux.ai/api/v1/management/payg/balance` + `/subscription/detail` (documented Management API) | PAYG balance plus optional subscription 5h/7d quotas, status, and monthly cap | Yes |
| **Vercel AI Gateway** | `ai-gateway.vercel.sh/v1/credits` + optional `/report` (documented REST API) | Credit balance, lifetime spend, and optional month-to-date reporting totals | Yes |
| **Command Code** | `api.commandcode.ai` `/alpha/billing/credits` + `/alpha/billing/subscriptions` (undocumented; the same calls the official `commandcode` CLI's `/usage` makes) | 5-hour and weekly rolling spend windows ($ used of $ cap), plan, and remaining monthly credits | No — widget/TUI only |

## Model Catalogs

`ai-usagebar prices` uses independent, read-only catalog caches to compare
default input and output token prices for exact model IDs. It does not affect
provider balance or quota refreshes.

| Gateway | Endpoint | Authentication |
|---|---|---|
| Kilo Gateway | `GET api.kilo.ai/api/gateway/models` | Not required |
| OpenRouter | `GET openrouter.ai/api/v1/models` | Not required |
| Requesty | `GET router.requesty.ai/v1/models` | Optional `REQUESTY_API_KEY`; authenticated results honor organization approvals |
| Vercel AI Gateway | `GET ai-gateway.vercel.sh/v1/models` | Not required |

The comparison does not infer equivalence from aliases or display names. It
compares base per-token input/output rates only; pricing tiers, caches, request
fees, modality charges, regions, and time-based overrides are retained by the
upstream catalogs but are not collapsed into a misleading single price.


## Stability notes

| Provider | Status |
|---|---|
| Claude | Undocumented usage endpoint, but used by the official `claude` CLI. Less fragile than a scraped web page. |
| Codex | Undocumented ChatGPT usage endpoint used by the official `codex` CLI. Windows are identified by duration instead of response position. |
| GitHub Copilot | Private endpoint used by VS Code. It requires a GitHub OAuth token and VS Code-compatible client headers; ai-usagebar gets it from the official `gh auth token` command after `gh auth login --web`. A non-empty `GITHUB_COPILOT_TOKEN` is an optional explicit override. GitHub CLI/editor/browser credentials are never parsed, copied, or stored. |
| Z.AI | Reverse-engineered from a third-party plugin. Treat this as the most fragile integration. |
| Kimi | Community-confirmed `/coding/v1/usages` route used by third-party quota tools. Drift is possible. The refresh grant is the Kimi Code CLI's own documented-by-behaviour device-flow token endpoint, using the CLI's public client id. |
| Cursor | Undocumented endpoint called by Cursor's dashboard. Its shape may change with Cursor pricing. |
| MiniMax | The Token Plan route is official, but no formal response schema is published. |
| Kiro CLI | `GetUsageLimits` is the same undocumented CodeWhisperer operation used by kiro-cli's `/usage` command. AWS SSO OIDC `CreateToken`, used for refresh, is documented. |
| Tavily | Documented `/usage` route with published OpenAPI schema. The `plan_limit`/`key.limit` fields are `null` for unlimited plans; the payload carries no reset timestamp. |
| Firecrawl | Documented v2 billing routes. Historical usage is optional detail: a current-credit response remains usable when historical usage fails or has no unique matching period. The live API may return `creditsUsed` and a null `endDate`; the parser accepts those forms alongside the published `totalCredits` shape. |
| Parallel | Documented Account API endpoint. Authenticates with the parallel-cli OAuth session (`~/.config/parallel-web-tools/auth.json`), refreshed via `platform.parallel.ai/getServiceKeys/token` with the rotated pair persisted back; data API keys are rejected (401). Invoice organizations intentionally show no prepaid balance. |
| Requesty | Documented management routes. Organization balance is primary; ungrouped `usage` map aggregation is optional detail, so a 403 or transient usage failure retains the live balance with a sanitized warning. |
| ZenMux | Documented Management API routes. PAYG and subscription responses are independent `success/data` envelopes; either valid block remains visible when the other fails. Standard inference keys are rejected, and `422` is a rate-limit diagnostic. |
| Vercel AI Gateway | Credits are the primary 60-second fetch. Custom Reporting is opt-in, billed per query, and cached independently for six hours; its `403` plan restriction retains live credits and a sanitized warning. |
| Command Code | Undocumented `/alpha/*` routes called by the official `commandcode` CLI. The `alpha` path segment is the vendor's own signal that these may move. Windows are read by name (`fiveHour`, `weekly`) rather than by position, and `windowLimits` is accepted both at the top level and beside the ledger, so the most likely reshuffles are already tolerated. |

Codex's known five-hour and seven-day windows are matched by their reported
duration, not by `primary_window` or `secondary_window` position. This handles
both the normal response and the temporary
[weekly-only response](https://github.com/openai/codex/issues/32707) without a
config switch.

## Run the live tests

```bash
make smoke
```

Claude, Codex, Z.AI, and OpenRouter tests require their normal credentials or
API keys. Command Code needs no key of its own — it reuses whichever local
agent harness is signed in, and skips when none is. Kimi is optional: its test
prints a skip reason when `KIMI_API_KEY` is unset (the smoke test covers the
API-key path; a subscription login is exercised by `ai-usagebar --vendor kimi`).
Tavily, Firecrawl, Requesty, ZenMux, and Vercel AI Gateway are likewise
optional and skip without `TAVILY_API_KEY`, `FIRECRAWL_API_KEY`,
`REQUESTY_API_KEY`, `ZENMUX_MANAGEMENT_API_KEY`, or `AI_GATEWAY_API_KEY`.

To test only Kimi:

```bash
cargo test --test live kimi_live -- --ignored --nocapture
```

The tests validate the fields used by ai-usagebar and report which part of a
response changed.
