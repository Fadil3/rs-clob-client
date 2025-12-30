use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;
use anyhow::Result;
use serde_json::Value;
use tokio::sync::mpsc;

pub async fn connect_and_listen(assets: Vec<String>, tx: mpsc::Sender<Value>) -> Result<()> {
    let url = Url::parse("wss://ws-subscriptions-clob.polymarket.com/ws/market")?;
    
    // Connect
    let (ws_stream, _) = connect_async(url.to_string()).await?;
    let (mut write, mut read) = ws_stream.split();

    // Subscribe
    let msg = serde_json::json!({
        "assets_ids": assets,
        "type": "market"
    });
    write.send(Message::Text(msg.to_string())).await?;

    // Loop
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(json) = serde_json::from_str::<Value>(&text) {
                    if let Err(_) = tx.send(json).await {
                         break; // Channel closed
                    }
                }
            },
            Ok(Message::Ping(_)) => {
                // Pong automatically handled by tungstenite usually
            },
            Err(e) => {
                eprintln!("WS Error: {}", e);
                break; 
            },
            _ => {}
        }
    }

    Ok(())
}
