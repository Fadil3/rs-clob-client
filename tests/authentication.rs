mod common;

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use httpmock::MockServer;
use polymarket_client_sdk::auth::Credentials;
use polymarket_client_sdk::clob::{Client, Config};
use polymarket_client_sdk::errors::Error;
use polymarket_client_sdk::{POLYGON, Result};
use reqwest::StatusCode;
use serde_json::json;

use crate::common::{API_KEY, PASSPHRASE, POLY_ADDRESS, PRIVATE_KEY, SECRET, create_authenticated};

#[tokio::test]
async fn authenticate_with_explicit_credentials_should_succeed() -> Result<()> {
    let server = MockServer::start();

    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));
    let client = Client::new(&server.base_url(), Config::default())?
        .authentication_builder(signer.clone())
        .credentials(Credentials::default())
        .authenticate()
        .await?;

    assert_eq!(signer.address(), client.address());

    Ok(())
}

#[tokio::test]
async fn authenticate_with_nonce_should_succeed() -> Result<()> {
    let server = MockServer::start();

    let mock = server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/auth/derive-api-key");
        then.status(StatusCode::OK).json_body(json!({
            "apiKey": API_KEY,
            "passphrase": PASSPHRASE,
            "secret": SECRET
        }));
    });

    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));
    let client = Client::new(&server.base_url(), Config::default())?
        .authentication_builder(signer.clone())
        .nonce(123)
        .authenticate()
        .await?;

    assert_eq!(signer.address(), client.address());

    mock.assert();

    Ok(())
}

#[tokio::test]
async fn authenticate_with_explicit_credentials_and_nonce_should_fail() -> Result<()> {
    let server = MockServer::start();

    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));
    let result = Client::new(&server.base_url(), Config::default())?
        .authentication_builder(signer.clone())
        .nonce(123)
        .credentials(Credentials::default())
        .authenticate()
        .await;

    match result {
        Ok(_) => panic!("expected failure"),
        Err(Error::Validation(msg)) => {
            assert_eq!(
                msg,
                "Credentials and nonce are both set. If nonce is set, then you must not supply credentials"
            );
        }
        Err(e) => panic!("unexpected error: {e}"),
    }

    Ok(())
}

#[tokio::test]
async fn authenticated_to_unauthenticated_should_succeed() -> Result<()> {
    let server = MockServer::start();
    let client = create_authenticated(&server).await?;

    assert_eq!(client.host().as_str(), format!("{}/", server.base_url()));
    client.deauthenticate()?;

    Ok(())
}

#[tokio::test]
async fn authenticate_with_multiple_strong_references_should_fail() -> Result<()> {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/auth/derive-api-key");
        then.status(StatusCode::OK).json_body(json!({
            "apiKey": API_KEY,
            "passphrase": PASSPHRASE,
            "secret": SECRET
        }));
    });

    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));
    let client = Client::new(&server.base_url(), Config::default())?;

    let _client_clone = client.clone();

    let result = client
        .authentication_builder(signer.clone())
        .authenticate()
        .await;

    match result {
        Ok(_) => panic!("expected failure"),
        Err(Error::Synchronization) => {}
        Err(e) => panic!("unexpected error: {e}"),
    }

    Ok(())
}

#[tokio::test]
async fn deauthenticated_with_multiple_strong_references_should_fail() -> Result<()> {
    let server = MockServer::start();
    let client = create_authenticated(&server).await?;

    let _client_clone = client.clone();

    let Error::Synchronization = client.deauthenticate().unwrap_err() else {
        panic!("Expected synchronization error");
    };

    Ok(())
}

#[tokio::test]
async fn create_api_key_should_succeed() -> Result<()> {
    let server = MockServer::start();
    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));
    let client = Client::new(&server.base_url(), Config::default())?;

    let mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/auth/api-key")
            .header(POLY_ADDRESS, signer.address().to_string().to_lowercase());
        then.status(StatusCode::OK).json_body(json!({
            "apiKey": API_KEY.to_string(),
            "passphrase": PASSPHRASE,
            "secret": SECRET
        }));
    });

    let credentials = client.create_api_key(&signer, None).await?;

    assert_eq!(
        credentials,
        Credentials::new(API_KEY, SECRET.to_owned(), PASSPHRASE.to_owned())
    );
    mock.assert();

    Ok(())
}

#[tokio::test]
async fn derive_api_key_should_succeed() -> Result<()> {
    let server = MockServer::start();
    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));
    let client = Client::new(&server.base_url(), Config::default())?;

    let mock = server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/auth/derive-api-key")
            .header(POLY_ADDRESS, signer.address().to_string().to_lowercase());
        then.status(StatusCode::OK).json_body(json!({
            "apiKey": API_KEY.to_string(),
            "passphrase": PASSPHRASE,
            "secret": SECRET
        }));
    });

    let credentials = client.derive_api_key(&signer, None).await?;

    assert_eq!(
        credentials,
        Credentials::new(API_KEY, SECRET.to_owned(), PASSPHRASE.to_owned())
    );
    mock.assert();

    Ok(())
}

#[tokio::test]
async fn create_or_derive_api_key_should_succeed() -> Result<()> {
    let server = MockServer::start();
    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));
    let client = Client::new(&server.base_url(), Config::default())?;

    let mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/auth/api-key");
        then.status(StatusCode::NOT_FOUND);
    });
    let mock2 = server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/auth/derive-api-key")
            .header(POLY_ADDRESS, signer.address().to_string().to_lowercase());
        then.status(StatusCode::OK).json_body(json!({
            "apiKey": API_KEY.to_string(),
            "passphrase": PASSPHRASE,
            "secret": SECRET
        }));
    });

    let credentials = client.create_or_derive_api_key(&signer, None).await?;

    assert_eq!(
        credentials,
        Credentials::new(API_KEY, SECRET.to_owned(), PASSPHRASE.to_owned())
    );
    mock.assert();
    mock2.assert();

    Ok(())
}
