use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::str::FromStr;

use anyhow::Context;
use futures::{stream, StreamExt};
use polymarket_client_sdk::clob::types::{
    MarketResponse, OrderBookSummaryRequest, OrderBookSummaryRequestBuilder, 
    BalanceAllowanceRequest
};
use polymarket_client_sdk::clob::Client;
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::Normal;
use polymarket_client_sdk::clob::ConfigBuilder;
use polymarket_client_sdk::{POLYGON, PRIVATE_KEY_VAR};
use alloy::signers::local::PrivateKeySigner as LocalSigner;
use alloy::signers::Signer;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Serialize;
use tokio::time::sleep;
use std::time::Duration;

// ----------------------------------------------------------------------------
// Constants
// ----------------------------------------------------------------------------
const SCAN_INTERVAL_SECS: u64 = 30;
const MAX_CONCURRENT_REQUESTS: usize = 5;
const MIN_SAMPLES_TO_FIND: usize = 20;
const MAX_SCANS_PER_TICK: usize = 5000;

// ----------------------------------------------------------------------------
// Data Structures
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MarketGroup {
    pub condition_id: String,
    pub outcomes: Vec<Outcome>, 
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub token_id: String,
    pub name: String,
    pub best_ask: Decimal,
    pub liquidity: Decimal,
}

#[derive(Serialize)]
struct LogEntry<'a> {
    level: &'static str,
    msg: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    condition_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sum_probs: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profit: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<'a> LogEntry<'a> {
    fn info(msg: &'a str) -> Self {
        Self { level: "INFO", msg, condition_id: None, sum_probs: None, profit: None, error: None }
    }
    fn error(msg: &'a str, err: impl std::fmt::Display) -> Self {
        Self { level: "ERROR", msg, condition_id: None, sum_probs: None, profit: None, error: Some(err.to_string()) }
    }
    fn arb(condition_id: &'a str, sum: Decimal, profit: Decimal) -> Self {
        Self { level: "ARBITRAGE", msg: "Opportunity Detected", condition_id: Some(condition_id), sum_probs: Some(sum), profit: Some(profit), error: None }
    }
    fn near_miss(condition_id: &'a str, sum: Decimal, profit: Decimal) -> Self {
        Self { level: "NEAR_MISS", msg: "Close Opportunity (Simulated)", condition_id: Some(condition_id), sum_probs: Some(sum), profit: Some(profit), error: None }
    }
}

// ----------------------------------------------------------------------------
// Main Loop
// ----------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 0. Load .env
    dotenvy::dotenv().ok();

    // 1. Setup Logging
    log_json(LogEntry::info("Starting Polybot-rs (Execution Phase)..."));

    // 1. Load Private Key & Auth
    let mut private_key = env::var(PRIVATE_KEY_VAR).context("POLYMARKET_PRIVATE_KEY not set")?;
    private_key = private_key.trim().trim_matches('\"').trim_matches('\'').to_string();
    if private_key.starts_with("0x") {
        private_key = private_key[2..].to_string();
    }
    let signer = LocalSigner::from_str(&private_key).map_err(|e| {
        anyhow::anyhow!("Invalid private key format (length: {}): {}", private_key.len(), e)
    })?.with_chain_id(Some(POLYGON));

    // 2. Connect & Authenticate
    let host = env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob.polymarket.com".to_string());
    log_json(LogEntry::info(&format!("Connecting to {}...", host)));

    let config = ConfigBuilder::default().use_server_time(true).build()?;
    let client = Client::new(&host, config)?
        .authentication_builder(&signer)
        .authenticate()
        .await
        .context("Failed to authenticate")?;

    // 3. Verify Auth (Check Balance)
    let balance = client.balance_allowance(&BalanceAllowanceRequest::default()).await;
    match balance {
        Ok(b) => log_json(LogEntry::info(&format!("Auth Success! Balance: {:?}", b))),
        Err(e) => log_json(LogEntry::error("Failed to fetch balance", e)),
    }

    let client = Arc::new(client);

    loop {
        match run_scan_cycle(client.clone()).await {
            Ok(_) => {},
            Err(e) => log_json(LogEntry::error("Scan cycle failed", e)),
        }

        log_json(LogEntry::info(&format!("Sleeping for {}s...", SCAN_INTERVAL_SECS)));
        sleep(Duration::from_secs(SCAN_INTERVAL_SECS)).await;
    }
}

// ----------------------------------------------------------------------------
// Core Logic
// ----------------------------------------------------------------------------

async fn run_scan_cycle(client: Arc<Client<Authenticated<Normal>>>) -> anyhow::Result<()> {
    log_json(LogEntry::info("Starting market scan..."));

    // 1. Scan for Active Negative Risk Markets
    let markets = find_active_negrisk_markets(&client).await?;
    
    // 2. Group them by Condition ID
    let groups = group_markets(markets);

    let count = groups.len();
    log_json(LogEntry::info(&format!("Detected {} opportunities. Checking prices...", count)));

    // 3. Process Groups concurrently (Price Check & Arb Calc)
    stream::iter(groups)
        .for_each_concurrent(MAX_CONCURRENT_REQUESTS, |mut group| {
            let client = client.clone();
            async move {
                if let Err(e) = fetch_prices_and_calc(&client, &mut group).await {
                    log_json(LogEntry::error("Failed to process group", e));
                }
            }
        })
        .await;

    Ok(())
}

async fn find_active_negrisk_markets(client: &Client<Authenticated<Normal>>) -> anyhow::Result<Vec<MarketResponse>> {
    let mut all_markets = Vec::new();
    let mut scanned_count = 0;

    // Use stream_data to automatically handle pagination for sampling_markets
    let mut stream = Box::pin(client.stream_data(|c, cursor| c.sampling_markets(cursor)));
    
    while let Some(response_result) = stream.next().await {
         let response = response_result.context("Failed to fetch markets")?;
         scanned_count += 1;
         
         if response.active && response.neg_risk && !response.closed {
             all_markets.push(response);
         }

         if all_markets.len() >= MIN_SAMPLES_TO_FIND || scanned_count >= MAX_SCANS_PER_TICK {
             break;
         }
    }
    
    log_json(LogEntry::info(&format!("Scanned {} markets. Found {} active NegRisk markets.", scanned_count, all_markets.len())));
    Ok(all_markets)
}

async fn process_all_groups(client: Arc<Client<Authenticated<Normal>>>, groups: Vec<MarketGroup>) {
    // Convert groups to a stream for concurrent processing
    stream::iter(groups)
        .for_each_concurrent(MAX_CONCURRENT_REQUESTS, |mut group| {
            let client = client.clone();
            async move {
                if let Err(e) = fetch_prices_and_calc(&client, &mut group).await {
                    log_json(LogEntry::error("Failed to process group", e));
                }
            }
        })
        .await;
}

const MIN_LIQUIDITY: Decimal = dec!(5.0); // Minimum 5 shares to count as valid price
const NEAR_MISS_THRESHOLD: Decimal = dec!(1.05);
const SIMULATED_BALANCE: Decimal = dec!(20.0); // User's hypothetical deposit

async fn fetch_prices_and_calc(client: &Client<Authenticated<Normal>>, group: &mut MarketGroup) -> anyhow::Result<()> {
    // Prepare requests (using OrderBooks to get Size/Liquidity)
    let book_requests: Vec<OrderBookSummaryRequest> = group.outcomes.iter().map(|outcome| {
        OrderBookSummaryRequestBuilder::default()
            .token_id(outcome.token_id.clone())
            .build()
            .expect("Failed to build orderbook request")
    }).collect();

    // Fetch OrderBooks (Batch)
    let books_response = client.order_books(&book_requests).await.context("API request failed")?;
    
    // Map responses by asset_id (token_id) for easy lookup. 
    // Note: OrderBookSummaryResponse has `asset_id` field which corresponds to token_id.
    let books_map: HashMap<String, _> = books_response.into_iter()
        .map(|b| (b.asset_id.clone(), b))
        .collect();

    let mut sum_probabilities = Decimal::ZERO;
    let mut incomplete_data = false;

const MAX_SPREAD: Decimal = dec!(0.10); // Max allowed spread $0.10

    for outcome in &mut group.outcomes {
         if let Some(book) = books_map.get(&outcome.token_id) {
             // 1. Find Best Ask (Selling to us) with Liquidity
             let best_valid_ask = book.asks.iter().find(|ask| ask.size >= MIN_LIQUIDITY);
             
             // 2. Find Best Bid (Buying from us) - no liquidity check needed for bid usually, just top of book
             // But to be safe, let's say we need someone willing to buy at least a little bit.
             // Actually, for spread check, usually top of book is fine.
             let best_bid = book.bids.iter().next().map(|bid| bid.price).unwrap_or(Decimal::ZERO);

             if let Some(ask) = best_valid_ask {
                 let spread = ask.price - best_bid;
                 
                 if spread > MAX_SPREAD {
                     // Spread too wide, skipping for safety
                     // Optional: Log strict rejection
                     // println!("Skipping {} due to wide spread: {}", outcome.name, spread);
                     incomplete_data = true;
                     continue;
                 }

                 outcome.best_ask = ask.price;
                 outcome.liquidity = ask.size;
                 sum_probabilities += ask.price;
             } else {
                 incomplete_data = true;
             }
         } else {
             incomplete_data = true;
         }
    }

    if incomplete_data {
        // Can't calculate full arb if some outcomes are missing liquidity
        return Ok(());
    }

    // Log Logic
    if sum_probabilities > Decimal::ZERO {
        if sum_probabilities < NEAR_MISS_THRESHOLD {
            // ROI = (Payout / Cost) - 1
            // Payout is always 1.0 (if we hold to expiry and one wins).
            // Cost is sum_probabilities.
            // ROI = (1.0 / sum_probabilities) - 1.0
            
            let roi = (Decimal::ONE / sum_probabilities) - Decimal::ONE;
            let simulated_profit_usd = SIMULATED_BALANCE * roi;

            if sum_probabilities < Decimal::ONE {
                 // ARBITRAGE (Profit)
                 log_json(LogEntry::arb(&group.condition_id, sum_probabilities, simulated_profit_usd));
            } else {
                 // NEAR MISS (Loss if executed)
                 log_json(LogEntry::near_miss(&group.condition_id, sum_probabilities, simulated_profit_usd));
            }
        } else {
            // Log efficient markets too for visibility (USER REQUEST)
            let roi = (Decimal::ONE / sum_probabilities) - Decimal::ONE;
            let simulated_profit_usd = SIMULATED_BALANCE * roi;
            log_json(LogEntry::info(&format!("Group {}: Sum {} | PnL: ${:.2}", group.condition_id, sum_probabilities, simulated_profit_usd)));
        }
    }

    Ok(())
}

fn group_markets(markets: Vec<MarketResponse>) -> Vec<MarketGroup> {
    let mut groups: HashMap<String, MarketGroup> = HashMap::new();

    for market in markets {
        if !market.active || market.closed || !market.neg_risk {
            continue;
        }

        let condition_id = market.condition_id.clone();
        
        let entry = groups.entry(condition_id.clone()).or_insert_with(|| MarketGroup {
            condition_id,
            outcomes: Vec::new(),
        });

        for token in market.tokens {
            if !entry.outcomes.iter().any(|o| o.token_id == token.token_id) {
               entry.outcomes.push(Outcome {
                   token_id: token.token_id,
                   name: token.outcome,
                   best_ask: Decimal::ZERO, 
                   liquidity: Decimal::ZERO,
               });
            }
        }
    }

    groups.into_values().collect()
}

fn log_json(entry: LogEntry) {
    if let Ok(json) = serde_json::to_string(&entry) {
        println!("{}", json);
    }
}
