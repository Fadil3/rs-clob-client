#![allow(clippy::exhaustive_enums, reason = "Fine for examples")]
#![allow(clippy::exhaustive_structs, reason = "Fine for examples")]
#![allow(clippy::unwrap_used, reason = "Fine for examples")]
#![allow(clippy::print_stdout, reason = "Examples are okay to print to stdout")]

use std::str::FromStr as _;

use alloy::primitives::{Address, U256, address};
use alloy::providers::ProviderBuilder;
use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use alloy::sol;
use anyhow::Result;
use polymarket_client_sdk::{AMOY, POLYGON, PRIVATE_KEY_VAR, contract_config};

const RPC_URL: &str = "https://polygon-rpc.com";

const USDC_ADDRESS: Address = address!("0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174");
const TOKEN_TO_APPROVE: Address = USDC_ADDRESS;

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function approve(address spender, uint256 value) external returns (bool);
    }

    #[sol(rpc)]
    interface IERC1155 {
        function setApprovalForAll(address operator, bool approved) external;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let chain = POLYGON;

    let private_key = std::env::var(PRIVATE_KEY_VAR).expect("Need a private key");
    let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(chain));

    let provider = ProviderBuilder::new()
        .wallet(signer.clone())
        .connect(RPC_URL)
        .await?;

    println!("Using address: {:?}", signer.address());

    let config = contract_config(chain, false).unwrap();
    let neg_risk_config = contract_config(chain, true).unwrap();
    let neg_risk_adapter = contract_config(AMOY, true).unwrap().exchange;

    let token = IERC20::new(TOKEN_TO_APPROVE, provider.clone());
    let ctf = IERC1155::new(config.conditional_tokens, provider.clone());

    approve(&token, config.conditional_tokens, U256::MAX).await?;
    set_approval_for_all(&ctf, config.conditional_tokens, true).await?;

    approve(&token, neg_risk_config.exchange, U256::MAX).await?;
    set_approval_for_all(&ctf, neg_risk_config.exchange, true).await?;

    approve(&token, neg_risk_adapter, U256::MAX).await?;
    set_approval_for_all(&ctf, neg_risk_adapter, true).await?;

    Ok(())
}

async fn approve<P: alloy::providers::Provider>(
    usdc: &IERC20::IERC20Instance<P>,
    spender: Address,
    amount: U256,
) -> Result<()> {
    println!("Calling USDC.approve({spender:?}, {amount})...");

    let receipt = usdc.approve(spender, amount).send().await?.watch().await?;

    println!("USDC approve tx mined: {receipt:?}");

    Ok(())
}

async fn set_approval_for_all<P: alloy::providers::Provider>(
    ctf: &IERC1155::IERC1155Instance<P>,
    operator: Address,
    approved: bool,
) -> Result<()> {
    println!("Calling CTF.setApprovalForAll({operator:?}, {approved})...");

    let receipt = ctf
        .setApprovalForAll(operator, approved)
        .send()
        .await?
        .watch()
        .await?;

    println!("CTF setApprovalForAll tx mined: {receipt:?}");

    Ok(())
}
