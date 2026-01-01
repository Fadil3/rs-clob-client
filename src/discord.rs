use reqwest::Client;
use serde_json::json;
use std::env;

/// Sends a fire-and-forget Discord alert.
/// Does not block execution.
pub fn send_alert(content: String) {
    tokio::spawn(async move {
        if let Ok(url) = env::var("DISCORD_WEBHOOK_URL") {
            if url.is_empty() { return; }
            
            let client = Client::new();
            let payload = json!({
                "content": content
            });

            if let Err(e) = client.post(&url).json(&payload).send().await {
                eprintln!("Failed to send Discord alert: {}", e);
            }
        }
    });
}

/// Formats a robust alert message from arbitrage details
pub fn format_arb_alert(condition_id: &str, profit: String, sum: String, link: &str) -> String {
    format!(
        "🚨 **Arbitrage Detected!** 🚨\n\n**Condition**: `{}`\n**Profit**: `${}`\n**Sum**: `{}`\n[View Market]({})",
        condition_id, profit, sum, link
    )
}
