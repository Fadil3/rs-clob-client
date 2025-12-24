use std::collections::HashMap;
use polymarket_client_sdk::clob::types::{MarketResponse};
use rust_decimal::Decimal;


// ----------------------------------------------------------------------------
// 4. Rust Data Structures (Core) from PRD
// ----------------------------------------------------------------------------

// The "State" of a single event (e.g. Election)
#[derive(Debug, Clone)]
pub struct MarketGroup {
    pub condition_id: String,
    pub outcomes: Vec<Outcome>, // [Trump, Harris, Other]
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub token_id: String,
    pub name: String,
    pub best_ask: Decimal,
    pub liquidity: Decimal,
}

// ----------------------------------------------------------------------------
// Logic: Market Grouping (The "Eyes")
// ----------------------------------------------------------------------------

/// Groups a list of markets by `condition_id`.
/// 
/// Requirements:
/// - Filter: `active: true` and `closed: false`.
/// - Filter: `negrisk: true`.
/// - Grouping: Aggregate tokens from markets sharing the same `condition_id`.
pub fn group_markets(markets: Vec<MarketResponse>) -> Vec<MarketGroup> {
    let mut groups: HashMap<String, MarketGroup> = HashMap::new();
    let mut stats_total = 0;
    let mut stats_inactive = 0;
    let mut stats_not_negrisk = 0;

    for market in markets {
        stats_total += 1;
        // 1. Filter: Active and Not Closed
        if !market.active || market.closed {
            stats_inactive += 1;
            continue;
        }

        // 2. Filter: Must be Negative Risk capable
        if !market.neg_risk {
            stats_not_negrisk += 1;
            continue;
        }

        let condition_id = market.condition_id.clone();
        
        let entry = groups.entry(condition_id.clone()).or_insert_with(|| MarketGroup {
            condition_id,
            outcomes: Vec::new(),
        });

        // 3. Grouping: Add tokens to the group
        for token in market.tokens {
            // Avoid duplicates if multiple markets list the same token
            if !entry.outcomes.iter().any(|o| o.token_id == token.token_id) {
               entry.outcomes.push(Outcome {
                   token_id: token.token_id,
                   name: token.outcome,
                   // Initial state: 0/Empty until Orderbook feed updates it
                   best_ask: Decimal::ZERO, 
                   liquidity: Decimal::ZERO,
               });
            }
        }
    }
    
    println!("Stats: Total scanned: {}", stats_total);
    println!("Stats: Skipped (Inactive/Closed): {}", stats_inactive);
    println!("Stats: Skipped (Not NegRisk): {}", stats_not_negrisk);

    groups.into_values().collect()
}

use polymarket_client_sdk::clob::{Client, Config};
use std::env;

use futures::StreamExt; // Need this for .next() on stream

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Starting Polybot-rs (Live Mode)...");

    let host = env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob.polymarket.com".to_string());
    println!("Connecting to {}...", host);
    
    let client = Client::new(&host, Config::default())?;

    println!("Scanning markets for active Negative Risk opportunities...");
    println!("(This will stream pages until we find 20 suitable markets or scan 5000 total)");

    let mut active_negrisk_markets = Vec::new();
    let mut total_scanned = 0;
    
    let mut stream = Box::pin(client.stream_data(|c, cursor| c.sampling_markets(cursor)));

    while let Some(market_result) = stream.next().await {
        let market = market_result?;
        total_scanned += 1;

        if total_scanned % 500 == 0 || total_scanned == 1 {
            if let Some(end_date) = market.end_date_iso {
                 println!("Scanned {} markets... (Sample date: {})", total_scanned, end_date);
            } else {
                 println!("Scanned {} markets...", total_scanned);
            }
        }

        if market.active && !market.closed && market.neg_risk {
            active_negrisk_markets.push(market);
            print!("."); // Progress dot for found market
            use std::io::Write;
            std::io::stdout().flush().ok();
        }

        if active_negrisk_markets.len() >= 20 || total_scanned >= 5000 {
            break;
        }
    }
    println!("\nStopped after scanning {} markets.", total_scanned);
    println!("Found {} active Negative Risk markets.", active_negrisk_markets.len());

    // 4. Group Markets
    let groups = group_markets(active_negrisk_markets);
    println!("Detected {} opportunities (Market Groups).", groups.len());
    
    // 5. Sample Output
    for (i, group) in groups.iter().take(3).enumerate() {
         println!("\nGroup #{}: Condition ID {}", i+1, group.condition_id);
         for outcome in &group.outcomes {
             println!(" - [{}] {}", outcome.token_id, outcome.name);
         }
    }

    Ok(())
}
