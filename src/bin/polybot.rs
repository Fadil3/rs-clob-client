use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::str::FromStr;

use anyhow::Context;
use futures::StreamExt;
use polymarket_client_sdk::clob::types::{
    MarketResponse, OrderType, Side, OrderBookSummaryRequest, OrderBookSummaryRequestBuilder
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

use alloy::sol;
use alloy::providers::ProviderBuilder;
use alloy::primitives::{Address, U256};
use polymarket_client_sdk::contract_config;
use url::Url;

// ----------------------------------------------------------------------------
// Constants
// ----------------------------------------------------------------------------
const MIN_SAMPLES_TO_FIND: usize = 100;
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


// ----------------------------------------------------------------------------
// Main Loop
// ----------------------------------------------------------------------------


// ----------------------------------------------------------------------------
// Phase 5: Event-Driven Logic
// ----------------------------------------------------------------------------
use polymarket_client_sdk::websocket::{WsEvent, WsCommand};


// Map: ConditionID -> Map: AssetID -> Price (Decimal)
type OrderBookCache = HashMap<String, MarketState>;

#[derive(Debug, Clone)]
struct MarketState {
    condition_id: String,
    outcomes: HashMap<String, OutcomePrice>, // key: asset_id
    last_update_ts: u64,
}

#[derive(Debug, Clone)]
struct OutcomePrice {
    best_ask: Decimal,
    // best_bid usually not needed for simple arb buying, but good to have
}

// ----------------------------------------------------------------------------
// Main Loop
// ----------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // [Setup Code Omitted for Brevity - Same as before]
    dotenvy::dotenv().ok();
    log_json(LogEntry::info("Starting Polybot-rs (Phase 5: Event-Driven)..."));

    let mut private_key = env::var(PRIVATE_KEY_VAR).context("POLYMARKET_PRIVATE_KEY not set")?;
    private_key = private_key.trim().trim_matches('\"').trim_matches('\'').to_string();
    if private_key.starts_with("0x") { private_key = private_key[2..].to_string(); }
    let signer = LocalSigner::from_str(&private_key).map_err(|e| anyhow::anyhow!("Invalid key: {}", e))?.with_chain_id(Some(POLYGON));

    let host = env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob.polymarket.com".to_string());
    let config = ConfigBuilder::default().use_server_time(true).build()?;
    let client = Client::new(&host, config)?
        .authentication_builder(&signer)
        .authenticate()
        .await
        .context("Failed to authenticate")?;
        
    let client = Arc::new(client); // Wrap early

    // Stats Tracking
    let mut total_events = 0;
    let start_time = std::time::Instant::now();

    // -----------------------------------------------------------------------
    // Phase 5: WebSocket Integration
    // -----------------------------------------------------------------------
    // -----------------------------------------------------------------------
    // Phase 8: Dynamic Market Rotation (Setup)
    // -----------------------------------------------------------------------
    let (ws_tx, mut ws_rx) = tokio::sync::mpsc::channel(100);
    // Command Channel for Rotation
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(100);
    
    // 1. Initial Scan
    log_json(LogEntry::info("Initial Scan to populate OrderBook Cache..."));
    let initial_markets = find_active_negrisk_markets(&client, MIN_SAMPLES_TO_FIND).await?;
    
    // 2. Build Cache & Assets List
    let mut cache: OrderBookCache = HashMap::new();
    let mut assets_to_track = Vec::new();
    
    // Map: AssetID -> ConditionID (Reverse lookup for WS events which only have AssetID sometimes)
    let mut asset_to_condition: HashMap<String, String> = HashMap::new();

    for m in &initial_markets {
        let condition_id = m.condition_id.clone();
        let mut outcomes_map = HashMap::new();
        
        for t in &m.tokens {
            assets_to_track.push(t.token_id.clone());
            asset_to_condition.insert(t.token_id.clone(), condition_id.clone());
            outcomes_map.insert(t.token_id.clone(), OutcomePrice { best_ask: Decimal::ZERO });
        }
        
        cache.insert(condition_id.clone(), MarketState {
            condition_id: condition_id.clone(),
            outcomes: outcomes_map,
            last_update_ts: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        });
    }

    log_json(LogEntry::info(&format!("Subscribing to {} assets via WebSocket...", assets_to_track.len())));

    // 2.5 Populate Prices Immediately (Snapshot) to trigger simulation
    log_json(LogEntry::info("Fetching initial price snapshot..."));
    populate_initial_and_sim(&client, &mut cache, &initial_markets).await;

    // Spawn WS Task (Phase 8: Pass cmd_rx)
    tokio::spawn(async move {
        if let Err(e) = polymarket_client_sdk::websocket::connect_and_listen(assets_to_track, ws_tx, cmd_rx).await {
            eprintln!("WebSocket Task Error: {}", e);
        }
    });
    
    log_json(LogEntry::info("Event Loop Started. Waiting for Market Updates..."));

    // EVENT LOOP (No more sleep!)
    loop {
        // Wait for next WS Event
        match ws_rx.recv().await {
            Some(event) => {
                total_events += 1;
                
                // Heartbeat
                // Heartbeat
                if total_events % 100 == 0 {
                    let uptime = start_time.elapsed().as_secs();
                    log_json(LogEntry::info(&format!("HEARTBEAT | Uptime: {}s | Events: {}", uptime, total_events)));
                    
                    // Verify On-Chain Connectivity by checking one random asset balance
                    if let Some(market_state) = cache.values().next() {
                         if let Some(asset_id) = market_state.outcomes.keys().next() {
                              let user_address = client.address();
                              let asset_id_clone = asset_id.clone();
                              tokio::spawn(async move {
                                  match check_on_chain_balance(user_address, &asset_id_clone).await {
                                      Ok(bal) => println!("{{\"level\":\"INVENTORY\",\"msg\":\"On-Chain Balance Verified\",\"asset\":\"{}\",\"balance\":\"{}\"}}", asset_id_clone, bal),
                                      Err(e) => eprintln!("Failed checking balance: {}", e),
                                  }
                              });
                         }
                    }

                    // Phase 8: Rotation Logic (Infinite Scanning)
                    // Every 500 events (approx 1-2 mins), rotate 5 stale markets.
                    if total_events % 500 == 0 {
                        // 1. Identify Stale Markets (Oldest 5)
                        let mut market_timestamps: Vec<(String, u64)> = cache.iter()
                            .map(|(k, v)| (k.clone(), v.last_update_ts))
                            .collect();
                        market_timestamps.sort_by_key(|k| k.1); // Sort by TS ascending (Oldest first)
                        
                        let to_remove: Vec<String> = market_timestamps.iter()
                            .take(5)
                            .map(|(id, _)| id.clone())
                            .collect();

                        if !to_remove.is_empty() {
                             let mut assets_to_unsub = Vec::new();
                             
                             // 2. Remove from Cache & Maps
                             for condition_id in &to_remove {
                                 if let Some(state) = cache.remove(condition_id) {
                                     for asset_id in state.outcomes.keys() {
                                         asset_to_condition.remove(asset_id);
                                         assets_to_unsub.push(asset_id.clone());
                                     }
                                 }
                             }
                             
                             // 3. Send Unsub Command
                             // let _ = cmd_tx.try_send(WsCommand::Unsubscribe(assets_to_unsub));
                             // (Skipping actual unsub command for now as confirmed unstable, rely on new subs or reconnect if needed)
                             // Actually, let's just log it.
                             log_json(LogEntry::info(&format!("Rotation: Dropped {} stale markets.", to_remove.len())));

                             // 4. Recruit New Markets (Fetch 5)
                             // We fetch slightly more (10) to ensure we find unique ones not in cache
                             if let Ok(new_markets) = find_active_negrisk_markets(&client, 10).await {
                                 let mut added_count = 0;
                                 let mut assets_to_sub = Vec::new();
                                 
                                 for m in new_markets {
                                     if added_count >= 5 { break; }
                                     if cache.contains_key(&m.condition_id) { continue; } // Skip if exists
                                     
                                     // Add to Cache
                                     let condition_id = m.condition_id.clone();
                                     let mut outcomes_map = HashMap::new();
                                     for t in &m.tokens {
                                         assets_to_sub.push(t.token_id.clone());
                                         asset_to_condition.insert(t.token_id.clone(), condition_id.clone());
                                         outcomes_map.insert(t.token_id.clone(), OutcomePrice { best_ask: Decimal::ZERO });
                                     }
                                     
                                     cache.insert(condition_id.clone(), MarketState {
                                         condition_id: condition_id.clone(),
                                         outcomes: outcomes_map,
                                         last_update_ts: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
                                     });
                                     added_count += 1;
                                 }
                                 
                                 // 5. Send Subscribe Command
                                 if !assets_to_sub.is_empty() {
                                     if let Err(e) = cmd_tx.try_send(WsCommand::Subscribe(assets_to_sub)) {
                                          eprintln!("Failed to send Subscribe cmd: {}", e);
                                     } else {
                                          log_json(LogEntry::info(&format!("Rotation: Added {} fresh markets.", added_count)));
                                     }
                                 }
                             }
                        }
                    }
                }

                // PROCESS EVENT
                if let Some(changes) = event.price_changes {
                    for change in changes {
                        // We only care about ASKS (selling to us) updates?
                        // Actually if we want to arb we need best ASK.
                        // WS sends updates for both sides.
                        
                        // Parse Price
                        let price = match Decimal::from_str(&change.price) {
                            Ok(p) => p,
                            Err(_) => continue,
                        };

                        // Update Cache
                        if let Some(condition_id) = asset_to_condition.get(&change.asset_id) {
                            if let Some(market_state) = cache.get_mut(condition_id) {
                                // Update Staleness Tracker
                                market_state.last_update_ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

                                if let Some(outcome) = market_state.outcomes.get_mut(&change.asset_id) {
                                    if change.side == "SELL" {
                                         // 'SELL' side in orderbook means someone is selling TO us -> This is the ASK price.
                                         // If size is 0, it means that price level is gone.
                                         // But WS usually sends TOP of book. 
                                         // If size == 0, we should technically look for next best?
                                         // Our simplified cache handles "Best Ask". 
                                         // If size > 0, update best ask.
                                         // If size == 0, we might need to invalidate? For now, let's assume valid updates.
                                         if change.size != "0" {
                                             outcome.best_ask = price;
                                         }
                                    }
                                }
                                
                                // CHECK ARB!
                                check_and_execute_arb(&client, market_state).await;
                            }
                        }
                    }
                }
            },
            None => {
                log_json(LogEntry::error("WS Channel Closed", "Exiting..."));
                break;
            }
        }
    }
    
    Ok(())
}

async fn check_and_execute_arb(client: &Arc<Client<Authenticated<Normal>>>, state: &MarketState) {
    let mut sum = Decimal::ZERO;
    let mut incomplete = false;
    let mut market_group_for_exec = MarketGroup {
        condition_id: state.condition_id.clone(),
        outcomes: Vec::new(), // We will populate this IF we find arb
    };

    for (asset_id, outcome) in &state.outcomes {
        if outcome.best_ask <= Decimal::ZERO {
            incomplete = true; 
            break;
        }
        sum += outcome.best_ask;
        
        market_group_for_exec.outcomes.push(Outcome {
            token_id: asset_id.clone(),
            name: "Unknown".to_string(), // We don't track name in simplified cache, maybe add it later
            best_ask: outcome.best_ask,
            liquidity: dec!(100.0), // Placeholder, WS update doesn't always send total liquidity
        });
    }

    if incomplete { return; }

    if sum < NEAR_MISS_THRESHOLD {
        let roi = (Decimal::ONE / sum) - Decimal::ONE;
        let profit = SIMULATED_BALANCE * roi;

        if sum < Decimal::ONE {
             // ARB FOUND!
             let entry = LogEntry::arb(&state.condition_id, sum, profit);
             log_json_with_file(&entry);
             
             if LIVE_TRADING_ENABLED {
                  // Execute
                  log_json(LogEntry::info(">>> EXECUTING ARBITRAGE (EVENT_DRIVEN) <<<"));
                  if let Err(e) = execute_arbitrage_batch(client, &market_group_for_exec).await {
                        log_json(LogEntry::error("Execution Failed", e));
                  }
             } else {
                  log_json(LogEntry::info("Trading Disabled (Simulation Mode)"));
             }
        } else {
             // Near Miss (Log for visibility during initial check, maybe limit to top 5?)
             if !LIVE_TRADING_ENABLED {
                 let roi = (Decimal::ONE / sum) - Decimal::ONE;
                 let profit = SIMULATED_BALANCE * roi;
                 // Only log if somewhat reasonable (ROI > -5%) to avoid garbage
                 if roi > dec!(-0.05) {
                    log_json(LogEntry::near_miss(&state.condition_id, sum, profit));
                 }
             }
        }
    }
}

// ----------------------------------------------------------------------------
// Core Logic
// ----------------------------------------------------------------------------


// ----------------------------------------------------------------------------
// Obsolete Polling Logic Removed (Phase 5 Transition)
// ----------------------------------------------------------------------------

async fn find_active_negrisk_markets(client: &Client<Authenticated<Normal>>, limit: usize) -> anyhow::Result<Vec<MarketResponse>> {
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

         if all_markets.len() >= limit || scanned_count >= MAX_SCANS_PER_TICK {
             break;
         }
    }
    
    // Only log if significant
    if limit > 20 {
        log_json(LogEntry::info(&format!("Scanned {} markets. Found {} active NegRisk markets.", scanned_count, all_markets.len())));
    }
    Ok(all_markets)
}

async fn populate_initial_and_sim(client: &Arc<Client<Authenticated<Normal>>>, cache: &mut OrderBookCache, markets: &[MarketResponse]) {
    // Collect all token IDs
    let mut all_tokens = Vec::new();
    for m in markets {
        for t in &m.tokens {
            all_tokens.push(t.token_id.clone());
        }
    }

    // Requests (Batching logic is handled by client if we send vector?)
    // Actually fetching 40+ orderbooks might fail if URL too long or rate limit. 
    // Let's do it in chunks.
    for chunk in all_tokens.chunks(20) {
        let reqs: Vec<OrderBookSummaryRequest> = chunk.iter().map(|id| {
             OrderBookSummaryRequestBuilder::default().token_id(id.clone()).build().unwrap()
        }).collect();
        
        if let Ok(books) = client.order_books(&reqs).await {
            for book in books {
                // Update Cache
                // Need to find which market this asset belongs to
                 for market_state in cache.values_mut() {
                     if let Some(outcome) = market_state.outcomes.get_mut(&book.asset_id) {
                         if let Some(ask) = book.asks.first() {
                             if ask.size >= dec!(5.0) { // Min Liquidity check
                                 outcome.best_ask = ask.price;
                             }
                         }
                     }
                 }
            }
        }
    }

    // Run Simulation
    for market_state in cache.values() {
        check_and_execute_arb(client, market_state).await;
    }
    log_json(LogEntry::info("Simulation Snapshot Complete."));
}


const NEAR_MISS_THRESHOLD: Decimal = dec!(1.05);
const SIMULATED_BALANCE: Decimal = dec!(20.0); // User's hypothetical deposit
const LIVE_TRADING_ENABLED: bool = false; // SAFETY SWITCH
const MAX_SPEND: Decimal = dec!(5.0); // Max spend per trade cycle

async fn execute_arbitrage_batch(client: &Client<Authenticated<Normal>>, group: &MarketGroup) -> anyhow::Result<()> {
    // Strategy: Buy MAX_SPEND of EACH outcome.
    // If Sum < 1.0, Total Cost < MAX_SPEND * Outcomes.
    // Payout = MAX_SPEND * Outcomes (since 1 share of winning outcome pays $1).
    // Note: This logic assumes we buy EQUAL SIZE of every outcome.
    
    let size_to_buy = MAX_SPEND; // 5 shares of each

    // Re-create signer for signing orders
    let mut private_key = env::var(PRIVATE_KEY_VAR)?;
    private_key = private_key.trim().trim_matches('\"').trim_matches('\'').to_string();
    if private_key.starts_with("0x") { private_key = private_key[2..].to_string(); }
    let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));

    for outcome in &group.outcomes {
        log_json(LogEntry::info(&format!("Placing order for {} ({} shares @ {})", outcome.name, size_to_buy, outcome.best_ask)));
        
        // Build Order
        let order = client.limit_order()
            .token_id(&outcome.token_id)
            .side(Side::Buy)
            .order_type(OrderType::FOK) // Fill Or Kill for safety
            .price(outcome.best_ask)
            .size(size_to_buy)
            .build()
            .await;

        let order = match order {
            Ok(o) => o,
            Err(e) => {
                log_json(LogEntry::error("Failed to build order", e));
                continue;
            }
        };

        // Sign & Post
        let signed_result = client.sign(&signer, order).await;
        match signed_result {
            Ok(signed_order) => {
                let post_result = client.post_order(signed_order).await;
                match post_result {
                    Ok(resp) => {
                         if let Some(r) = resp.first() {
                             log_json(LogEntry::info(&format!("Order Placed! ID: {}", r.order_id)));
                         } else {
                             log_json(LogEntry::info("Order Placed (No ID returned)"));
                         }
                    },
                    Err(e) => log_json(LogEntry::error("Post Order Failed", e)),
                }
            },
            Err(e) => log_json(LogEntry::error("Signing Failed", e)),
        }
    }

    Ok(())
}


fn log_json(entry: LogEntry) {
    if let Ok(json) = serde_json::to_string(&entry) {
        println!("{}", json);
    }
}

fn log_json_with_file(entry: &LogEntry) {
    if let Ok(json) = serde_json::to_string(entry) {
        println!("{}", json);
        

        // Append to file
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("opportunities.jsonl") 
        {
            if let Err(e) = writeln!(file, "{}", json) {
                eprintln!("Failed to write to log file: {}", e);
            }
        }
    }
}

// ----------------------------------------------------------------------------
// Phase 7: On-Chain Logic
// ----------------------------------------------------------------------------

sol! {
    #[sol(rpc)]
    contract ConditionalTokens {
        function balanceOf(address account, uint256 id) external view returns (uint256);
    }
}

async fn check_on_chain_balance(user_address: Address, token_id_str: &str) -> anyhow::Result<Decimal> {
    // 1. Setup Provider (RPC)
    let rpc_url = env::var("POLYGON_RPC_URL").unwrap_or_else(|_| "https://polygon-rpc.com".to_string());
    let provider = ProviderBuilder::new().connect_http(Url::parse(&rpc_url)?);

    // 2. Get Contract Address
    let config = contract_config(POLYGON, false).ok_or(anyhow::anyhow!("No contract config"))?;
    let ct_address = config.conditional_tokens;

    // 3. Parse Token ID
    let token_id = U256::from_str_radix(token_id_str, 10).unwrap_or(U256::ZERO);

    // 4. Call Contract
    // Note: The `ConditionalTokens` struct is generated by `sol!` macro above.
    // It should be available as `ConditionalTokens`.
    let contract = ConditionalTokens::new(ct_address, provider);
    let result = contract.balanceOf(user_address, token_id).call().await?;
    
    // 5. Convert to Decimal
    let bal_dec = Decimal::from_str(&result.to_string())? / dec!(1_000_000); 
    
    Ok(bal_dec)
}
