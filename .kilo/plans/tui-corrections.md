# Plan: TUI Corrections

## Goal

Clarify provider data sources, reduce detail-panel noise, and make the model-price browser's ordering explicit and composable.

## Phase 1: Provider Presentation [x]

- [x] Remove the Requesty organization label from the Overview and detail title.
- [x] Keep only Requesty balance and month-to-date spend/request count in its detail panel.
- [x] Render both Codex subscription windows in the Overview.
- [x] Identify the Codex subscription windows separately from OpenAI API credits.
- [x] Label the OpenRouter historical-credit gauge as purchased API credit consumption.
- [x] Document that OpenRouter credit grants are not exposed by the currently used public API endpoints.

## Phase 2: Period Usage In Details [x]

- [x] Inventory period totals already exposed by each provider snapshot.
- [x] Add concise period usage blocks where the provider API supplies trustworthy daily, weekly, monthly, or billing-cycle totals.
- [x] Do not invent periods or derive totals from quota percentages.

Available totals: OpenRouter reports daily, weekly, and monthly API-key usage;
Requesty and Vercel AI Gateway report month-to-date totals. Other supported
providers currently expose quota windows or current billing-cycle counters only.

## Follow-up: Comparable Credit Bars

- [x] Inventory providers for cumulative credited and debited totals.
- [x] Add a credit-consumption bar when a provider exposes both cumulative credited and debited totals.
- [x] Keep OpenRouter's TUI, widget, and report wording aligned with the purchased-credit scope.

OpenRouter is currently the only provider snapshot with both totals. Firecrawl
and ZenMux expose limits or balances, not a matching cumulative debit total.

## Phase 3: Model Price Browser [x]

- [x] Add an input/output average column for every gateway price row.
- [x] Define `BEST OVERALL` from the lowest average price, with explicit ties.
- [x] Replace the single cycling sort with visible ordered sort keys and directions.
- [x] Support multiple sort keys, such as name then average price or average price then name.

## Verification

- [x] Add focused panel, Overview, and price-browser tests for each completed phase.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo test --all-targets --locked`.
- [x] Run `cargo clippy --all-targets --locked -- -D warnings`.
- [x] Run `make desktop-test`.
- [x] Run `cargo machete`.
