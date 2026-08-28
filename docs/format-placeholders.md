# Format placeholders

Use placeholders with `--format` and `--tooltip-format`. Unsupported or absent
metrics expand to an empty string unless noted otherwise.

## Cross-provider formats

`{vendor_short}` identifies the active provider:

| Provider | Value | Provider | Value |
|---|---:|---|---:|
| Claude | `cld` | Codex | `gpt` |
| Z.AI | `zai` | OpenRouter | `opr` |
| DeepSeek | `dsk` | Kimi | `kmi` |
| Kilo | `klo` | Novita | `nvt` |
| Moonshot | `msh` | Grok | `grk` |
| SuperGrok | `sgk` | Anthropic API | `aac` |
| Antigravity | `agy` | Cursor | `cur` |
| MiniMax | `mmx` | Kiro CLI | `kir` |
| Tavily | `tav` | Firecrawl | `fcw` |
| Requesty | `rqy` | ZenMux | `zmx` |
| Vercel AI Gateway | `vag` | | |
| Nous Research | `nrs` | OpenCode Go | `ocg` |

The same codes ride the `ai-usagebar usage --json` report as each entry's
`short_name`, so a native frontend can draw a Waybar-style provider tag without
restating the table. Every account of one provider shares that provider's code.

Use `{session_pct}`, `{session_reset}`, `{weekly_pct}`, and `{weekly_reset}`
when one format must work across providers. Providers without matching time
windows return neutral values. Cursor maps Cursor Models to the session slot
and Other Models to the weekly slot; both reset with the billing cycle. Kiro
has one pool, so it maps `kiro_pct` to both percentage slots.

Claude and Codex also provide `*_elapsed`, `*_pace`, and `*_bar` families.
Z.AI and MiniMax provide elapsed aliases plus provider-specific pace families.
Antigravity provides elapsed values for all four windows plus
`{session_model}`, `{weekly_model}`, `{scoped_model}`, and `{extra_model}`.
Provider-specific families such as `{oai_*}`, `{zai_*}`, and `{or_*}` are empty
for providers that do not define them.

## Shared and Claude placeholders

These are compatible with claudebar.

| Placeholder | Example |
|---|---|
| `{plan}` | `Max 5x` |
| `{session_pct}`, `{session_reset}`, `{session_bar}`, `{session_elapsed}` | `62`, `1h 30m`, `█████████████░░░░░░░`, `58` |
| `{session_pace}`, `{session_pace_indicator}`, `{session_pace_pct}`, `{session_pace_pts}`, `{session_pace_delta}`, `{session_pace_abs_delta}` | `↑`, `↑`, `12% ahead`, `4pts ahead`, `4`, `4` |
| `{weekly_*}` | The same family for the seven-day window. |
| `{sonnet_*}` | The same family for the seven-day Sonnet window. Empty when absent. |
| `{scoped_model}`, `{scoped_pct}`, `{scoped_reset}`, `{scoped_elapsed}`, `{scoped_bar}` | `Fable`, `84`, `5d 2h`, `27`, `█████████████████░░░` |
| `{extra_spent}`, `{extra_limit}`, `{extra_pct}`, `{extra_bar}` | `$2.50`, `$50.00`, `5`, `█░░░░░░░░░░░░░░░░░░░` |

The scoped family describes the first model-specific weekly window. When that
window is absent, it returns neutral empty, `0`, or `—` values as appropriate.

## Codex

`{oai_plan}`, `{oai_session_pct}`, `{oai_session_reset}`,
`{oai_session_elapsed}`, `{oai_session_pace}`,
`{oai_session_pace_indicator}`, `{oai_weekly_*}`,
`{oai_code_review_pct}`, `{oai_credit_balance}`, `{oai_local_msgs}`,
`{oai_cloud_msgs}`

Session and weekly families are empty when the API omits that window. The
default widget automatically uses weekly values for a weekly-only response.

## Z.AI

`{zai_plan}`, `{zai_session_pct}`, `{zai_session_reset}`,
`{zai_session_elapsed}`, `{zai_session_pace}`,
`{zai_session_pace_indicator}`, `{zai_weekly_pct}`, `{zai_weekly_reset}`,
`{zai_weekly_elapsed}`, `{zai_weekly_pace}`,
`{zai_weekly_pace_indicator}`, `{zai_mcp_pct}`, `{zai_mcp_reset}`,
`{zai_mcp_elapsed}`, `{zai_mcp_pace}`, `{zai_mcp_pace_indicator}`

`{session_elapsed}` and `{weekly_elapsed}` are cross-provider aliases. An
absent window returns empty values. A present window whose API response omits
its reset uses the shared neutral pacing values: elapsed `0` and arrow `→`.

## MiniMax

`{minimax_plan}`, `{minimax_session_pct}`, `{minimax_session_reset}`,
`{minimax_session_elapsed}`, `{minimax_session_pace}`,
`{minimax_session_pace_indicator}`, `{minimax_weekly_pct}`,
`{minimax_weekly_reset}`, `{minimax_weekly_elapsed}`,
`{minimax_weekly_pace}`, `{minimax_weekly_pace_indicator}`,
`{minimax_video_pct}`, `{minimax_video_reset}`, `{minimax_video_elapsed}`,
`{minimax_video_pace}`, `{minimax_video_pace_indicator}`,
`{minimax_video_weekly_pct}`, `{minimax_video_weekly_reset}`,
`{minimax_video_weekly_elapsed}`, `{minimax_video_weekly_pace}`,
`{minimax_video_weekly_pace_indicator}`

`{session_elapsed}` and `{weekly_elapsed}` alias the text pool. Optional video
windows return `—` when absent. As with Z.AI, a present window without a reset
uses elapsed `0` and the neutral `→` pace marker.

## OpenRouter

`{or_label}`, `{or_balance}`, `{or_total}`, `{or_used}`,
`{or_used_today}`, `{or_used_week}`, `{or_used_month}`,
`{or_consumed_pct}`, `{or_free_tier}`, `{or_limit}`,
`{or_limit_remaining}`, `{or_balance_bar}`

## DeepSeek

`{ds_balance}`, `{ds_granted}`, `{ds_topped_up}`, `{ds_available}`

These report the `/user/balance` credit balance. USD is preferred when both
currencies are present; otherwise they use CNY.

## Kimi

`{kimi_plan}`, `{kimi_weekly_pct}`, `{kimi_weekly_used}`,
`{kimi_weekly_limit}`, `{kimi_weekly_remaining}`, `{kimi_weekly_reset}`,
`{kimi_window_pct}`, `{kimi_window_used}`, `{kimi_window_limit}`,
`{kimi_window_remaining}`, `{kimi_window_reset}`

These cover the subscription quota and rolling five-hour window from
`api.kimi.com/coding/v1/usages`. The default format is
`{kimi_weekly_pct}% · {kimi_weekly_reset}`. Generic aliases are `{plan}` for the
plan, `{weekly_pct}` for weekly usage, and `{session_pct}` for the five-hour
window.

## Kilo

`{kilo_balance}` is the remaining USD balance from
`api.kilo.ai/api/profile/balance`.

## Novita

`{nv_balance}`, `{nv_cash}`, `{nv_credit_limit}`, `{nv_owed}` report the USD
balance and its breakdown from `api.novita.ai/openapi/v1/billing/balance/detail`.

## Moonshot

`{km_balance}`, `{km_voucher}`, `{km_cash}`, `{currency}` report the account
balance from `api.moonshot.ai` or `api.moonshot.cn`. The global service uses
USD; the China service uses CNY.

## Grok

`{grok_balance}` is the prepaid USD balance from the xAI Management API.

## SuperGrok

`{sgk_plan}`, `{sgk_pct}`, `{sgk_reset}`, `{sgk_period}`, `{sgk_prepaid}`

- `{sgk_period}` is `Weekly`, `Monthly`, or `Current period`.
- The default bar format is `{sgk_pct}% · {sgk_reset}`.
- `{session_pct}` and `{weekly_pct}` remain aliases for `sgk_pct`.
- `{plan}` is the subscription tier when Grok Build supplies one.

SuperGrok is the subscription path provided by Grok Build's `x.ai/billing` ACP
extension. It is separate from the Grok Management API prepaid balance.
ai-usagebar never parses, copies, caches, refreshes, or sends the SuperGrok
token in ACP messages. It hashes auth and config files only to keep caches
separate between logins.

The default executable is `$GROK_HOME/bin/grok`, or `~/.grok/bin/grok` when
`GROK_HOME` is unset. ai-usagebar does not search `PATH`. Set
`[supergrok] grok_binary` only when the trusted official binary lives elsewhere.

## Anthropic API

`{aapi_headline}`, `{aapi_spent}`, `{aapi_limit}`, `{aapi_pct}` report
month-to-date spend from the Admin API `cost_report`.

- With a positive finite `monthly_limit`, the headline looks like
  `$1.34 / $1000 · 0%`.
- Without a limit, it looks like `$1.34/mo`.
- `{plan}`, `{session_pct}`, and `{weekly_pct}` are generic aliases. Both
  percentage aliases use spend versus limit.

This is spend, not prepaid balance. Anthropic does not expose prepaid balance
through the API. The
[Cost API documentation](https://platform.claude.com/docs/en/manage-claude/usage-cost-api)
also says that Priority Tier costs are omitted.

## Cursor

`{cursor_plan}`, `{cursor_auto_pct}`, `{cursor_api_pct}`,
`{cursor_total_pct}`, `{cursor_reset}`, `{cursor_on_demand}`,
`{cursor_unlimited}`

- `{cursor_auto_pct}` is the Cursor Models pool (Auto and Composer).
- `{cursor_api_pct}` is the Other Models pool (named and API models).
- `{cursor_total_pct}` is the overall included-usage headline.
- `{cursor_on_demand}` and `{cursor_unlimited}` return `on`/`off` and
  `yes`/`no`.
- `{session_pct}`, `{weekly_pct}`, and `{plan}` alias Cursor Models, Other
  Models, and `Cursor <Plan>`.

A pool can exceed 100%. The default format is
`{cursor_auto_pct}·{cursor_api_pct}%` and uses the worse pool's severity color.

Cursor's dashboard also reports overage and per-member team spend; ai-usagebar
does not. Team payloads without `individualUsage.plan` fall back to the
dashboard's display messages and add `(team)` to the inferred plan. This path
has not been verified against a live team account.

## Kiro CLI

`{kiro_plan}`, `{kiro_pct}`, `{kiro_used}`, `{kiro_limit}`, `{kiro_reset}`

These describe the current credit cycle returned by
`AmazonCodeWhispererService.GetUsageLimits`, the same call used by kiro-cli's
`/usage` command. Raw credit counts keep two decimal places only when needed.
The default format is `{kiro_pct}%`; `{session_pct}` and `{weekly_pct}` alias
the same pool, and `{plan}` aliases the subscription title.

Kiro access tokens expire after roughly an hour. ai-usagebar refreshes them
through the documented AWS SSO OIDC `CreateToken` API and stores refreshed or
rotated credentials in an account-scoped `kiro/oauth.json` file. That file is
mode `0600` on Unix. kiro-cli's database is opened read-only and is never
modified.

## Tavily

`{tav_plan}`, `{tav_headline}`, `{tav_plan_pct}`, `{tav_plan_used}`,
`{tav_plan_limit}`, `{tav_payg_used}`, `{tav_payg_limit}`, `{tav_key_used}`,
`{tav_key_limit}`, `{tav_search}`, `{tav_extract}`, `{tav_crawl}`,
`{tav_map}`, `{tav_research}` report the documented `/usage` billing-cycle
credit counts.

- `{tav_headline}` is the bar's default: `62%` when the plan has a positive
  limit, otherwise the truthful `620 used` — never a fabricated percentage.
- `{tav_plan_pct}` is `—` and `{session_pct}`/`{weekly_pct}` are `0` when the
  plan has no positive limit (unlimited plans).
- `{tav_plan_limit}` and `{tav_key_limit}` are `unlimited` for uncapped plans;
  `{tav_payg_limit}` is `—` when the API omits it.
- There is no reset timestamp in the API response, so reset placeholders
  render as `—`.
- `{plan}` and `{session_pct}`/`{weekly_pct}` alias the plan name and plan
  percentage. An optional `[tavily] project_id` scopes the query and cache;
  it does not change any placeholder.

## Firecrawl

`{fcw_headline}`, `{fcw_remaining}`, `{fcw_plan}`, `{fcw_used}`, `{fcw_pct}`,
and `{fcw_reset}` report the documented team credit endpoints.

- `{fcw_headline}` is the bar's default: the current-period percentage when
  matching historical usage exists, otherwise the truthful remaining-credit
  count.
- `{fcw_pct}` preserves values above 100 in text; the visual gauge clamps only
  its fill. `{fcw_used}` is `—` when no historical row matches the current
  billing period.
- `{session_pct}` and `{weekly_pct}` alias the current-period percentage when
  available; reset aliases use the billing-period countdown.

## Requesty

`{rqy_headline}`, `{rqy_org}`, `{rqy_balance}`, `{rqy_mtd}`, `{rqy_requests}`,
`{rqy_tokens}`, `{rqy_input}`, and `{rqy_output}` report organization balance
and optional month-to-date management usage.

- `{rqy_headline}` and `{rqy_balance}` are USD balances. `{plan}` aliases the
  organization name.
- `{rqy_mtd}` is month-to-date spend. Request and token placeholders are
  aggregate totals returned by the ungrouped `resolution=day` query.
- If the usage endpoint is unavailable or the key lacks permission, all
  `{rqy_*}` usage placeholders are `—`; the live organization balance remains
  available.
- Requesty has no rolling quota denominator or reset timestamp, so
  `{session_pct}` and `{weekly_pct}` are neutral `0` values and reset aliases
  are `—`.

## ZenMux

`{zmx_headline}`, `{zmx_payg}`, `{zmx_topup}`, `{zmx_bonus}`, `{zmx_tier}`,
`{zmx_status}`, `{zmx_five}`, `{zmx_five_reset}`, `{zmx_seven}`,
`{zmx_seven_reset}`, and `{zmx_expiry}` report PAYG balance and subscription
quota data from the Management API.

- `{zmx_payg}`, `{zmx_topup}`, and `{zmx_bonus}` are USD PAYG balances.
- `{zmx_five}` and `{zmx_seven}` are the five-hour and seven-day subscription
  quota percentages. Their elapsed and pace families are `zmx_five_*` and
  `zmx_seven_*`.
- `{plan}`, `{session_pct}`, `{session_reset}`, `{weekly_pct}`, and
  `{weekly_reset}` alias subscription data only when the subscription endpoint
  succeeds. They are empty for a PAYG-only snapshot, preventing fake quota
  windows in desktop adapters.
- `{zmx_status}` preserves documented account status; an unknown status is
  shown as received and is never treated as healthy.

## Vercel AI Gateway

`{vag_balance}`, `{vag_used}`, `{vag_mtd}`, `{vag_requests}`, `{vag_input}`,
and `{vag_output}` report AI Gateway credits, lifetime spend, and optional
month-to-date Custom Reporting totals.

- `{vag_balance}` is the current USD credit balance and `{vag_used}` is
  lifetime spend; they are distinct values.
- `{vag_mtd}`, `{vag_requests}`, `{vag_input}`, and `{vag_output}` are `—`
  unless `[vercel-ai-gateway] report_enabled = true` and a report is available.
- The report intentionally omits `api_key_id`, so its totals are account/team
  wide, matching the credit balance scope rather than a single API key.
- Vercel has no usage quota denominator, so generic session/weekly percentage
  aliases remain empty and `usage --json` emits text and block sections rather
  than fabricated metrics.
