//! Read-only comparison of gateway model catalog prices.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cache::{Cache, acquire_lock_async};
use crate::config::Config;
use crate::error::{AppError, Result};
use crate::vendor::{
    HTTP_CLIENT_TIMEOUT, MAX_BODY_BYTES, read_body_capped, same_origin_redirect_policy,
};

const CATALOG_TTL: Duration = Duration::from_secs(21_600);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const LOCK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrice {
    pub model_id: String,
    pub gateway: Gateway,
    pub input_per_token: f64,
    pub output_per_token: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gateway {
    KiloGateway,
    OpenRouter,
    Requesty,
    VercelAiGateway,
}

impl Gateway {
    pub const fn label(self) -> &'static str {
        match self {
            Self::KiloGateway => "Kilo Gateway",
            Self::OpenRouter => "OpenRouter",
            Self::Requesty => "Requesty",
            Self::VercelAiGateway => "Vercel AI Gateway",
        }
    }
}

#[derive(Debug, Serialize)]
struct Comparison<'a> {
    model_id: &'a str,
    input_winner: Gateway,
    output_winner: Gateway,
    overall_winner: Option<Gateway>,
    overall_tied: bool,
    overall_winners: Vec<Gateway>,
    prices: Vec<PriceRow<'a>>,
}

#[derive(Debug, Serialize)]
struct PriceRow<'a> {
    gateway: Gateway,
    input_per_million: f64,
    output_per_million: f64,
    model_id: &'a str,
}

pub async fn run(json: bool, model: Option<&str>) -> i32 {
    match load_comparisons(model).await {
        Ok(comparisons) => {
            if json {
                println!("{}", serde_json::json!({"comparisons": comparisons}));
            } else if comparisons.is_empty() {
                println!("No exact model IDs are available from two or more catalogs.");
            } else {
                for comparison in comparisons {
                    println!("{}", comparison.model_id);
                    println!("  Cheapest input: {}", comparison.input_winner.label());
                    println!("  Cheapest output: {}", comparison.output_winner.label());
                    match comparison.overall_winner {
                        Some(gateway) => println!("  Cheapest overall: {}", gateway.label()),
                        None if comparison.overall_tied => println!("  Cheapest overall: tie"),
                        None => println!("  Cheapest overall: none (input/output tradeoff)"),
                    }
                    for price in comparison.prices {
                        println!(
                            "  {}: input ${:.4}/M, output ${:.4}/M",
                            price.gateway.label(),
                            price.input_per_million,
                            price.output_per_million
                        );
                    }
                }
            }
            0
        }
        Err(error) => {
            eprintln!("ai-usagebar prices: {}", error.user_message());
            1
        }
    }
}

pub async fn load_comparisons(model: Option<&str>) -> Result<Vec<PriceComparison>> {
    let config = Config::load()?;
    let client = reqwest::Client::builder()
        .timeout(HTTP_CLIENT_TIMEOUT)
        .redirect(same_origin_redirect_policy())
        .build()?;
    let requesty_key = config
        .is_configured(crate::vendor::VendorId::Requesty)
        .then(|| {
            crate::config::resolve_api_key(
                "Requesty",
                &config.requesty.api_key_env,
                config.requesty.api_key.as_deref(),
            )
        })
        .transpose()?;
    let (kilo, openrouter, vercel, requesty) = tokio::join!(
        load_catalog(
            &client,
            Gateway::KiloGateway,
            "https://api.kilo.ai/api/gateway/models",
            None
        ),
        load_catalog(
            &client,
            Gateway::OpenRouter,
            "https://openrouter.ai/api/v1/models",
            None
        ),
        load_catalog(
            &client,
            Gateway::VercelAiGateway,
            "https://ai-gateway.vercel.sh/v1/models",
            None
        ),
        load_catalog(
            &client,
            Gateway::Requesty,
            "https://router.requesty.ai/v1/models",
            requesty_key.as_deref()
        ),
    );
    let mut prices = kilo?;
    prices.extend(openrouter?);
    prices.extend(vercel?);
    if let Ok(requesty) = requesty {
        prices.extend(requesty);
    }
    let groups = comparable_models(&prices);
    let requested = model.map(str::to_owned);
    let mut comparisons = Vec::new();
    for id in groups.keys() {
        if requested
            .as_deref()
            .is_none_or(|requested| requested == *id)
            && let Some(comparison) = compare(&prices, id)
        {
            comparisons.push(owned(comparison));
        }
    }
    if let Some(requested) = requested
        && comparisons.is_empty()
    {
        return Err(AppError::Other(format!(
            "no comparable published prices for exact model ID {requested}"
        )));
    }
    Ok(comparisons)
}

#[derive(Debug, Clone, Serialize)]
pub struct PriceComparison {
    pub model_id: String,
    pub input_winner: Gateway,
    pub output_winner: Gateway,
    pub overall_winner: Option<Gateway>,
    pub overall_tied: bool,
    /// Gateways with the same dominant input/output pair. A tie can include
    /// every gateway (identical prices) or only a cheaper subset.
    pub overall_winners: Vec<Gateway>,
    pub prices: Vec<PriceRowOwned>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PriceRowOwned {
    pub gateway: Gateway,
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub model_id: String,
}

fn owned(comparison: Comparison<'_>) -> PriceComparison {
    PriceComparison {
        model_id: comparison.model_id.into(),
        input_winner: comparison.input_winner,
        output_winner: comparison.output_winner,
        overall_winner: comparison.overall_winner,
        overall_tied: comparison.overall_tied,
        overall_winners: comparison.overall_winners,
        prices: comparison
            .prices
            .into_iter()
            .map(|price| PriceRowOwned {
                gateway: price.gateway,
                input_per_million: price.input_per_million,
                output_per_million: price.output_per_million,
                model_id: price.model_id.into(),
            })
            .collect(),
    }
}

async fn load_catalog(
    client: &reqwest::Client,
    gateway: Gateway,
    url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelPrice>> {
    let cache = Cache::for_vendor(&format!("prices-{}", gateway as u8))?;
    cache.ensure_dir()?;
    let _lock = acquire_lock_async(&cache.lock_path(), LOCK_TIMEOUT).await?;
    if let Some(bytes) = cache.fresh_payload(CATALOG_TTL)?
        && let Ok(prices) = parse_catalog(&bytes, gateway)
    {
        return Ok(prices);
    }
    let response = tokio::time::timeout(HTTP_TIMEOUT, async {
        let request = client.get(url).header("Accept", "application/json");
        if let Some(api_key) = api_key {
            request
                .header("Authorization", format!("Bearer {api_key}"))
                .send()
                .await
        } else {
            request.send().await
        }
    })
    .await
    .map_err(|_| AppError::Transport(format!("price catalog timed out: {url}")))??;
    let status = response.status();
    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    if !status.is_success() {
        return Err(AppError::Http {
            status: status.as_u16(),
            body: format!(
                "{} price catalog returned HTTP {}",
                gateway.label(),
                status.as_u16()
            ),
        });
    }
    let prices = parse_catalog(&body, gateway)?;
    cache.write_payload(&body)?;
    Ok(prices)
}

#[derive(Deserialize)]
struct Catalog {
    data: Vec<CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    id: String,
    #[serde(default)]
    pricing: serde_json::Value,
    #[serde(default)]
    input_price: Option<f64>,
    #[serde(default)]
    output_price: Option<f64>,
}

fn parse_catalog(bytes: &[u8], gateway: Gateway) -> Result<Vec<ModelPrice>> {
    let catalog: Catalog = serde_json::from_slice(bytes).map_err(|error| {
        AppError::Schema(format!("{} price catalog schema: {error}", gateway.label()))
    })?;
    Ok(catalog
        .data
        .into_iter()
        .filter_map(|model| {
            let prices = if gateway == Gateway::Requesty {
                model
                    .pricing
                    .as_array()
                    .and_then(|tiers| {
                        tiers.iter().find(|tier| {
                            tier.get("prompt_tokens_threshold")
                                .and_then(serde_json::Value::as_u64)
                                == Some(0)
                        })
                    })
                    .and_then(|tier| {
                        Some((
                            tier.get("input_price")?.as_f64()?,
                            tier.get("output_price")?.as_f64()?,
                        ))
                    })
                    .or_else(|| Some((model.input_price?, model.output_price?)))
            } else {
                Some((
                    parse_price(
                        model
                            .pricing
                            .get("prompt")
                            .or_else(|| model.pricing.get("input")),
                    )?,
                    parse_price(
                        model
                            .pricing
                            .get("completion")
                            .or_else(|| model.pricing.get("output")),
                    )?,
                ))
            };
            let (input_per_token, output_per_token) = prices?;
            (input_per_token.is_finite()
                && output_per_token.is_finite()
                && input_per_token >= 0.0
                && output_per_token >= 0.0
                && !model.id.trim().is_empty())
            .then_some(ModelPrice {
                model_id: model.id,
                gateway,
                input_per_token,
                output_per_token,
            })
        })
        .collect::<Vec<_>>())
}

fn parse_price(value: Option<&serde_json::Value>) -> Option<f64> {
    match value? {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn compare<'a>(prices: &'a [ModelPrice], model: &str) -> Option<Comparison<'a>> {
    let mut matching: Vec<_> = prices
        .iter()
        .filter(|price| price.model_id == model)
        .collect();
    if matching.len() < 2 {
        return None;
    }
    matching.sort_by_key(|price| price.gateway);
    let input_winner = matching
        .iter()
        .min_by(|a, b| a.input_per_token.total_cmp(&b.input_per_token))?
        .gateway;
    let output_winner = matching
        .iter()
        .min_by(|a, b| a.output_per_token.total_cmp(&b.output_per_token))?
        .gateway;
    let overall_winners: Vec<_> = matching
        .iter()
        .filter(|candidate| {
            matching.iter().all(|other| {
                candidate.input_per_token <= other.input_per_token
                    && candidate.output_per_token <= other.output_per_token
            })
        })
        .map(|price| price.gateway)
        .collect();
    let overall_tied = overall_winners.len() > 1;
    let overall_winner = if overall_winners.len() == 1 {
        Some(overall_winners[0])
    } else {
        None
    };
    let model_id = &matching[0].model_id;
    Some(Comparison {
        model_id,
        input_winner,
        output_winner,
        overall_winner,
        overall_tied,
        overall_winners,
        prices: matching
            .into_iter()
            .map(|price| PriceRow {
                gateway: price.gateway,
                input_per_million: price.input_per_token * 1_000_000.0,
                output_per_million: price.output_per_token * 1_000_000.0,
                model_id: &price.model_id,
            })
            .collect(),
    })
}

fn comparable_models(prices: &[ModelPrice]) -> BTreeMap<&str, Vec<&ModelPrice>> {
    let mut groups: BTreeMap<&str, Vec<&ModelPrice>> = BTreeMap::new();
    for price in prices {
        groups.entry(&price.model_id).or_default().push(price);
    }
    groups.retain(|_, values| values.len() >= 2);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(gateway: Gateway, input: f64, output: f64) -> ModelPrice {
        ModelPrice {
            model_id: "openai/gpt-test".into(),
            gateway,
            input_per_token: input,
            output_per_token: output,
        }
    }

    #[test]
    fn exact_model_id_comparison_keeps_separate_input_and_output_winners() {
        let prices = [
            price(Gateway::Requesty, 2e-6, 8e-6),
            price(Gateway::OpenRouter, 3e-6, 6e-6),
        ];
        let comparison = compare(&prices, "openai/gpt-test").unwrap();
        assert_eq!(comparison.input_winner, Gateway::Requesty);
        assert_eq!(comparison.output_winner, Gateway::OpenRouter);
        assert_eq!(comparison.overall_winner, None);
    }

    #[test]
    fn dominant_gateway_is_the_only_overall_winner() {
        let prices = [
            price(Gateway::Requesty, 2e-6, 6e-6),
            price(Gateway::OpenRouter, 3e-6, 7e-6),
        ];
        let comparison = compare(&prices, "openai/gpt-test").unwrap();
        assert_eq!(comparison.overall_winner, Some(Gateway::Requesty));
        assert_eq!(comparison.prices[0].gateway.label(), "OpenRouter");
    }

    #[test]
    fn only_exact_id_intersections_are_comparable() {
        let mut alias = price(Gateway::OpenRouter, 1e-6, 1e-6);
        alias.model_id = "gpt-test".into();
        let prices = [price(Gateway::Requesty, 1e-6, 1e-6), alias];
        assert!(comparable_models(&prices).is_empty());
    }

    #[test]
    fn equal_prices_are_an_explicit_overall_tie() {
        let prices = [
            price(Gateway::OpenRouter, 2e-6, 6e-6),
            price(Gateway::VercelAiGateway, 2e-6, 6e-6),
        ];
        let comparison = compare(&prices, "openai/gpt-test").unwrap();
        assert_eq!(comparison.overall_winner, None);
        assert!(comparison.overall_tied);
        assert_eq!(comparison.overall_winners.len(), comparison.prices.len());
    }

    #[test]
    fn partial_equal_best_values_keep_every_winner() {
        let prices = [
            price(Gateway::KiloGateway, 2e-6, 6e-6),
            price(Gateway::OpenRouter, 2e-6, 6e-6),
            price(Gateway::VercelAiGateway, 3e-6, 7e-6),
        ];
        let comparison = compare(&prices, "openai/gpt-test").unwrap();
        assert_eq!(comparison.overall_winner, None);
        assert!(comparison.overall_tied);
        assert_eq!(
            comparison.overall_winners,
            vec![Gateway::KiloGateway, Gateway::OpenRouter]
        );
    }

    #[test]
    fn catalogs_parse_documented_default_prices_without_model_aliases() {
        let openrouter = parse_catalog(br#"{"data":[{"id":"openai/gpt-test","pricing":{"prompt":"0.000002","completion":"0.000006"}}]}"#, Gateway::OpenRouter).unwrap();
        let kilo = parse_catalog(br#"{"data":[{"id":"openai/gpt-test","pricing":{"prompt":"0.000004","completion":"0.000008"}}]}"#, Gateway::KiloGateway).unwrap();
        let vercel = parse_catalog(br#"{"data":[{"id":"openai/gpt-test","pricing":{"input":"0.000003","output":"0.000005"}}]}"#, Gateway::VercelAiGateway).unwrap();
        let requesty = parse_catalog(br#"{"data":[{"id":"openai/gpt-test","pricing":[{"prompt_tokens_threshold":0,"input_price":0.000001,"output_price":0.000007},{"prompt_tokens_threshold":200000,"input_price":0.000002,"output_price":0.000008}]}]}"#, Gateway::Requesty).unwrap();
        let prices = [
            openrouter[0].clone(),
            kilo[0].clone(),
            vercel[0].clone(),
            requesty[0].clone(),
        ];
        let comparison = compare(&prices, "openai/gpt-test").unwrap();
        assert_eq!(comparison.input_winner, Gateway::Requesty);
        assert_eq!(comparison.output_winner, Gateway::VercelAiGateway);
        assert_eq!(comparison.overall_winner, None);
    }

    #[test]
    fn catalog_rejects_entries_without_both_finite_default_prices() {
        let prices = parse_catalog(br#"{"data":[{"id":"openai/gpt-test","pricing":{"prompt":"NaN","completion":"0.000006"}},{"id":"openai/missing","pricing":{"prompt":"0.000001"}}]}"#, Gateway::OpenRouter).unwrap();
        assert!(prices.is_empty());
    }
}
