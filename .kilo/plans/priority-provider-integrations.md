# Plan: Priority Provider Integrations

## Goal

Add native, opt-in usage or balance integrations in priority order while using
only documented, read-only provider interfaces. Do not scrape dashboards,
replay browser sessions, or make billable requests merely to measure usage.

## Existing Coverage

- DeepSeek, Kimi, MiniMax, and Grok are already native providers.
- `opencode-go` reports the separate OpenCode Go subscription usage endpoint.
- Tavily, Firecrawl, Requesty, ZenMux, and Vercel AI Gateway are complete.

## Priority Slices

### 1. Parallel Account Balance

**Viable now.** `GET https://api.parallel.ai/account/service/v1/balance`
returns organization-level `credit_balance_cents`,
`pending_debit_balance_cents`, and `will_invoice`.

- Authenticate with a Parallel Account API OAuth access token, not
  `PARALLEL_API_KEY`; accept an explicit `PARALLEL_ACCOUNT_ACCESS_TOKEN`
  environment variable first.
- Keep the first slice read-only and token-only. Device OAuth login and refresh
  persistence are separate work because they require registering a client and
  securely storing refresh credentials.
- Display prepaid balance and pending debit as text. Invoice organizations show
  their billing mode instead of a fake zero balance.
- Add scoped cache, widget, TUI, `usage --json`, Settings, desktop adapters,
  tests, docs, and an ignored live smoke test.

### 2. Context7 Library Metrics

**Viable with explicit scope.** `GET /v2/libs/metrics` requires an API key and
a teamspace-owned `libraryId`; it reports per-library lifetime and daily
request counters, not the teamspace's billing total.

- Require `CONTEXT7_API_KEY` and `[context7] library_id`.
- Show cumulative request counts and recent daily counts as informational text.
- Never represent library metrics as team billing, plan quota, or remaining
  credits.

### 3. Ollama Local Runtime

**Viable as availability/inventory, not billing.** The local API has no
authentication and exposes installed/running models. Per-request token metrics
are returned only after a generation, so ai-usagebar must not issue a
generation just to create usage data.

- Add an opt-in local endpoint probe and model inventory.
- Do not present a balance, quota, or synthetic usage percentage.

## Deferred Pending Official Read-Only Account APIs

| Provider | Current blocker |
|---|---|
| OpenCode Zen | Documented API-key endpoint reports Go subscription windows; no public API-key balance endpoint for Zen PAYG credits. |
| Mercury / Inception | Public docs provide model pricing and rate limits, but no documented account billing or remaining-token endpoint. |
| Exa Search | Search and Contents responses report per-request estimated `costDollars`; no documented aggregate account usage or balance endpoint. |
| Brave Search | Dashboard exposes usage/credits; public APIs document per-request usage headers but no account usage endpoint. |
| Dappier | Public endpoints document per-request pricing and ZeroClick metering, not tenant balance or aggregate usage. |
| Cohere | Responses expose request token metadata; no documented aggregate billing endpoint for an API key. |
| Abacus, Gemini, Qwen, Bedrock, GitHub Copilot | Require separate contract research because their billing is account, cloud-project, organization, or subscription scoped rather than a generic API-key balance. |

## Guardrails

- Each provider defaults `enabled = false` and has a stable `VendorId` slug.
- Use only documented endpoints and fields verified immediately before each
  slice.
- Cache data by non-secret credential scope with atomic writes and async locks.
- Keep balances, counters, and non-percentage values as `Text` or `Block` rows.
- Treat API errors as sanitized diagnostics; never print a key, token, raw
  authentication body, account identifier, or dashboard response.
- Keep the Windows notification-area frontend paused until these provider slices
  are committed. It continues to consume only `usage --json`.

## References

- [Parallel Account API](https://docs.parallel.ai/integrations/account-api)
- [Parallel balance](https://docs.parallel.ai/service-api/balance/get-balance)
- [Context7 usage](https://context7.com/docs/howto/usage)
- [Context7 library metrics](https://context7.com/docs/api-reference/metrics/get-library-usage-metrics)
- [Ollama usage](https://docs.ollama.com/api/usage)
- [OpenCode Zen](https://opencode.ai/docs/zen/)
- [Inception pricing](https://docs.inceptionlabs.ai/get-started/models)
- [Exa Search API](https://docs.exa.ai/reference/search)
- [Brave Answers usage headers](https://api-dashboard.search.brave.com/documentation/services/answers)
- [Cohere pricing](https://docs.cohere.com/docs/how-does-cohere-pricing-work)
