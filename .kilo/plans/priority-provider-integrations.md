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

### 1. GitHub Copilot Billing and Usage

**Viable now.** GitHub documents read-only billing APIs for AI credits and
premium requests, plus 28-day Copilot usage-metrics reports.

- Require an explicit `GITHUB_TOKEN` or `GH_TOKEN`; do not parse GitHub CLI
  credential files or keyrings.
- Support exactly one configured scope per snapshot: personal username,
  organization, or enterprise. Scope selection determines the documented API
  endpoint and avoids mixing personally billed and organization-billed usage.
- Personal Copilot subscriptions use user billing endpoints. Organization and
  enterprise subscriptions use their respective billing endpoints and need the
  documented admin/billing permission.
- Begin with AI-credit and premium-request billing totals. Add adoption metrics
  only as an optional, separately cached 28-day signed-report download because
  its schema and permission policy differ.
- Display counters and dollars as text, never as a fake percentage. Document
  that organization reports are membership-attributed and cannot be summed with
  enterprise reports.

### 2. Model Price Comparison

**Viable now for three gateways.** Compare published, current catalog prices
for matching model identifiers from OpenRouter, Requesty, and Vercel AI Gateway.

- Add a read-only `ai-usagebar prices` command rather than mixing price catalog
  refreshes into quota widgets or provider credit refreshes.
- Fetch `GET /api/v1/models` from OpenRouter, `GET /v1/models` from Requesty,
  and `GET /v1/models` from Vercel AI Gateway. Requesty receives its configured
  key when available so the catalog reflects organization-approved models.
- Normalize only exact canonical model IDs first. Never infer equivalence from
  display names, aliases, or provider marketing labels.
- Compare default input and output USD-per-token prices separately. Preserve
  cache read/write, request, image, web-search, context-tier, region, temporal,
  and conditional pricing as metadata; do not select a universal "cheapest"
  when those terms differ.
- Output each comparable model with the cheapest input provider, cheapest output
  provider, and a same-provider winner only when one provider is no more
  expensive in both values. Mark ties and incomplete entries explicitly.
- Cache catalogs independently with a 6-hour TTL. Public OpenRouter/Vercel
  catalogs must work without a usage credential; Requesty catalog errors must
  not affect its balance integration.

### 3. Parallel Account Balance

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
