use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = Url::parse("wss://ws-subscriptions-clob.polymarket.com/ws/market")?;
    println!("Connecting to {}...", url);

    let (ws_stream, _) = connect_async(url.to_string()).await?;
    println!("Connected!");

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to a known active market (or asset) 
    // Example: Subscribing to "price" or "orderbook" for a Trump market asset ID
    // We'll use a hardcoded asset ID just for testing connection.
    // Asset ID for "Trump 2024 Presidential Winner" (Yes): 21742633143463906290569050155826241533067272736897614950488156847949938836455
    let msg = serde_json::json!({
        "assets_ids": ["21742633143463906290569050155826241533067272736897614950488156847949938836455"],
        "type": "market"
    });

    write.send(Message::Text(msg.to_string())).await?;
    println!("Sent Subscription: {}", msg);

    while let Some(msg) = read.next().await {
        let msg = msg?;
        if msg.is_text() {
             println!("Received: {}", msg.to_text()?);
        }
    }

    Ok(())
}
