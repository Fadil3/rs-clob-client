use alloy::signers::local::PrivateKeySigner;
use alloy::hex;

#[tokio::main]
async fn main() {
    println!("\nGenerating 4 Fresh Swarm Wallets...\n");

    let prefixes = ["0x0", "0x4", "0x8", "0xC"];

    for (i, prefix) in prefixes.iter().enumerate() {
        let signer = PrivateKeySigner::random();
        let address = signer.address();
        let key_hex = hex::encode(signer.to_bytes());

        println!("--- Bot #{} (Shard {}) ---", i + 1, prefix);
        println!("Address:     {}", address);
        println!("Private Key: {}", key_hex);
        println!("Config:      SHARD_PREFIX=\"{}\"", prefix);
        println!();
    }

    println!("INSTRUCTIONS:");
    println!("1. Send $21 MATIC/USDC to each Address.");
    println!("2. Create .env.{} for each bot with the Private Key.", 1);
    println!("3. Run: SHARD_PREFIX=... cargo run --bin polybot");
}
