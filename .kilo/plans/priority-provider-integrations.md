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

### 1. Kilo Code Account Usage and Pricing

Kilo Gateway documents a public model catalog, but no account-balance or
aggregate-usage endpoint. Add its catalog to the read-only price comparison;
defer account usage until Kilo publishes a stable read-only interface.

- [x] Fetch `GET https://api.kilo.ai/api/gateway/models` without authentication.
- [x] Compare only exact model IDs and default input/output token prices.

### 2. Model Price Comparison  ✅

**Viable now for four gateways.** Compare published, current catalog prices
for matching model identifiers from Kilo Gateway, OpenRouter, Requesty, and
Vercel AI Gateway.

- [x] Add a read-only `ai-usagebar prices` command rather than mixing price catalog
  refreshes into quota widgets or provider credit refreshes.
- [x] Fetch `GET /api/v1/models` from OpenRouter, `GET /v1/models` from Requesty,
  and `GET /v1/models` from Vercel AI Gateway. Requesty receives its configured
  key when available so the catalog reflects organization-approved models.
- [x] Normalize only exact canonical model IDs first. Never infer equivalence from
  display names, aliases, or provider marketing labels.
- [x] Compare default input and output USD-per-token prices separately. Preserve
  cache read/write, request, image, web-search, context-tier, region, temporal,
  and conditional pricing as metadata; do not select a universal "cheapest"
  when those terms differ.
- [x] Output each comparable model with the cheapest input provider, cheapest output
  provider, and a same-provider winner only when one provider is no more
  expensive in both values. Mark ties and incomplete entries explicitly.
- [x] Cache catalogs independently with a 6-hour TTL. Public OpenRouter/Vercel
  catalogs must work without a usage credential; Requesty catalog errors must
  not affect its balance integration.

### 3. Parallel Account Balance

**Viable now.** `GET https://api.parallel.ai/account/service/v1/balance`
returns organization-level `credit_balance_cents`,
`pending_debit_balance_cents`, and `will_invoice`.

- [x] Authenticate with the parallel-cli OAuth session (device-OAuth tokens
  from `parallel-cli login`), never a standard data API key; refresh the
  ~7-minute access token via `platform.parallel.ai` and persist the rotated
  pair back to `~/.config/parallel-web-tools/auth.json` under a lock.
- [x] Keep the first slice read-only and token-only. Device OAuth login itself
  stays with parallel-cli; `PARALLEL_API_KEY` is honored only as a JWT-shaped
  Account API token override.
- [x] Display prepaid balance and pending debit as text. Invoice organizations show
  their billing mode instead of a fake zero balance.
- [x] Add scoped cache, widget, TUI, `usage --json`, Settings, desktop adapters,
  tests, docs, and an ignored live smoke test.

### 4. Context7 Library Metrics

**Viable with explicit scope.** `GET /v2/libs/metrics` requires an API key and
a teamspace-owned `libraryId`; it reports per-library lifetime and daily
request counters, not the teamspace's billing total.

- Require `CONTEXT7_API_KEY` and `[context7] library_id`.
- Show cumulative request counts and recent daily counts as informational text.
- Never represent library metrics as team billing, plan quota, or remaining
  credits.

### 5. Ollama Local Runtime

**Viable as availability/inventory, not billing.** The local API has no
authentication and exposes installed/running models. Per-request token metrics
are returned only after a generation, so ai-usagebar must not issue a
generation just to create usage data.

- Add an opt-in local endpoint probe and model inventory.
- Do not present a balance, quota, or synthetic usage percentage.

## Deferred Pending Official Read-Only Account APIs

| Provider | Current blocker |
|---|---|
| GitHub Copilot | User billing endpoints expose only personally billed subscriptions. Organization or enterprise billing requires a separately configured administrator or billing-manager scope, which does not serve the current account. |
| OpenCode Zen | Documented API-key endpoint reports Go subscription windows; no public API-key balance endpoint for Zen PAYG credits. Existing `opencode-go` already covers that separate subscription endpoint. |
| Mercury / Inception | Public docs provide model pricing and rate limits, but no documented account billing or remaining-token endpoint. |
| Exa Search | Search and Contents responses report per-request estimated `costDollars`; no documented aggregate account usage or balance endpoint. |
| Brave Search | Dashboard exposes usage/credits; public APIs document per-request usage headers but no account usage endpoint. |
| Dappier | Public endpoints document per-request pricing and ZeroClick metering, not tenant balance or aggregate usage. |
| Cohere | Responses expose request token metadata; no documented aggregate billing endpoint for an API key. |
| Abacus, Gemini, Qwen, Bedrock | Require separate contract research because their billing is account, cloud-project, organization, or subscription scoped rather than a generic API-key balance. |

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

- [Kilo Gateway API reference](https://kilo.ai/docs/gateway/api-reference)
- [Kilo Gateway usage and billing](https://kilo.ai/docs/gateway/usage-and-billing)
- [Parallel Account API](https://docs.parallel.ai/integrations/account-api)
- [Parallel balance](https://docs.parallel.ai/service-api/balance/get-balance)
- [Context7 usage](https://context7.com/docs/howto/usage)
- [Context7 library metrics](https://context7.com/docs/api-reference/metrics/get-library-usage-metrics)
- [Ollama usage](https://docs.ollama.com/api/usage)
- [OpenCode Zen](https://opencode.ai/docs/zen/)
- [GitHub Copilot billing usage](https://docs.github.com/en/rest/billing/usage)
- [GitHub Copilot usage metrics](https://docs.github.com/en/rest/copilot/copilot-usage-metrics)
- [Inception pricing](https://docs.inceptionlabs.ai/get-started/models)
- [Exa Search API](https://docs.exa.ai/reference/search)
- [Brave Answers usage headers](https://api-dashboard.search.brave.com/documentation/services/answers)
- [Cohere pricing](https://docs.cohere.com/docs/how-does-cohere-pricing-work)
- [OpenRouter model catalog](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties)
- [Requesty model catalog](https://docs.requesty.ai/api-reference/endpoint/models-list)
- [Vercel AI Gateway models](https://vercel.com/docs/ai-gateway/sdks-and-apis/rest-api)
