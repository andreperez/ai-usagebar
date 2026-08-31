# Plan: TUI Corrections

## Goal

Clarify provider data sources, reduce detail-panel noise, and make the model-price browser's ordering explicit and composable.

## Phase 1: Provider Presentation [x]

- [x] Remove the Requesty organization label from the Overview and detail title.
- [x] Keep only Requesty balance and month-to-date spend/request count in its detail panel.
- [x] Render both Codex subscription windows in the Overview.
- [x] Identify the Codex subscription windows separately from OpenAI API credits.
- [x] Remove the OpenRouter historical-credit gauge and label its API-reported purchased balance accurately.
- [x] Document that OpenRouter credit grants are not exposed by the currently used public API endpoints.

## Phase 2: Period Usage In Details

- [x] Inventory period totals already exposed by each provider snapshot.
- [ ] Add concise period usage blocks where the provider API supplies trustworthy daily, weekly, monthly, or billing-cycle totals.
- [ ] Do not invent periods or derive totals from quota percentages.

Available totals: OpenRouter reports daily, weekly, and monthly API-key usage;
Requesty and Vercel AI Gateway report month-to-date totals. Other supported
providers currently expose quota windows or current billing-cycle counters only.

## Phase 3: Model Price Browser

- [ ] Add an input/output average column for every gateway price row.
- [ ] Define `BEST OVERALL` from the lowest average price, with explicit ties.
- [ ] Replace the single cycling sort with visible ordered sort keys and directions.
- [ ] Support multiple sort keys, such as name then average price or average price then name.

## Verification

- [ ] Add focused panel, Overview, and price-browser tests for each completed phase.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test --all-targets --locked`.
- [ ] Run `cargo clippy --all-targets --locked -- -D warnings`.
- [ ] Run `make desktop-test`.
- [ ] Run `cargo machete`.
