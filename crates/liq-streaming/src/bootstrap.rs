//! RPC bootstrap trait: getProgramAccounts / getMultipleAccounts.
//! Prefer fixture/mock without live creds; HTTP JSON-RPC shape is documented for live hook.

use async_trait::async_trait;
use liq_core::{Pubkey, StateStore, StoredAccount, UpdateSource};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("http: {0}")]
    Http(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAccount {
    pub pubkey: Pubkey,
    pub lamports: u64,
    pub owner: Pubkey,
    pub data: Vec<u8>,
    pub slot: u64,
}

/// Bootstrap obligations/banks/reserves into a StateStore.
#[async_trait]
pub trait RpcBootstrap: Send + Sync {
    async fn get_multiple_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<RawAccount>>, BootstrapError>;

    async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
    ) -> Result<Vec<RawAccount>, BootstrapError>;
}

/// Apply raw accounts into a byte-level StateStore (decoded adapters plug in later).
pub fn apply_raw_to_store(store: &StateStore<Vec<u8>>, accounts: &[RawAccount], source: UpdateSource) {
    for a in accounts {
        store.upsert(StoredAccount::new(
            a.slot,
            0,
            a.pubkey,
            a.data.clone(),
            source,
        ));
    }
}

/// In-memory fixture bootstrap (LIQ_FIXTURES / tests).
pub struct FixtureBootstrap {
    pub accounts: Vec<RawAccount>,
}

#[async_trait]
impl RpcBootstrap for FixtureBootstrap {
    async fn get_multiple_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<RawAccount>>, BootstrapError> {
        Ok(keys
            .iter()
            .map(|k| self.accounts.iter().find(|a| &a.pubkey == k).cloned())
            .collect())
    }

    async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
    ) -> Result<Vec<RawAccount>, BootstrapError> {
        Ok(self
            .accounts
            .iter()
            .filter(|a| &a.owner == program_id)
            .cloned()
            .collect())
    }
}

impl FixtureBootstrap {
    pub fn demo_for_protocols() -> Self {
        use liq_core::programs;
        let owners = [programs::klend(), programs::save(), programs::marginfi()];
        let mut accounts = Vec::new();
        for (i, owner) in owners.iter().enumerate() {
            for j in 0..3u64 {
                accounts.push(RawAccount {
                    pubkey: Pubkey::test(50 + i as u8, j),
                    lamports: 1_000_000,
                    owner: *owner,
                    data: vec![i as u8, j as u8, 0xDE, 0xAD],
                    slot: 1,
                });
            }
        }
        Self { accounts }
    }
}

/// JSON-RPC request builders (getMultipleAccounts / getProgramAccounts).
/// Transport is pluggable so unit tests need no network; live hook uses HttpJsonRpcTransport.
pub fn jsonrpc_get_multiple_accounts(keys_b58_or_hex: &[String], commitment: &str) -> Value {
    // Keys expected as base58 in live mode; we pass hex placeholders in stub.
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getMultipleAccounts",
        "params": [
            keys_b58_or_hex,
            { "encoding": "base64", "commitment": commitment }
        ]
    })
}

pub fn jsonrpc_get_program_accounts(program_id: &str, commitment: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            program_id,
            { "encoding": "base64", "commitment": commitment }
        ]
    })
}

#[async_trait]
pub trait JsonRpcTransport: Send + Sync {
    async fn post_json(&self, body: &Value) -> Result<Value, BootstrapError>;
}

/// Concrete HTTP transport stub — clearly the live hook.
/// Does not perform network I/O until `enabled`; returns descriptive error otherwise.
pub struct HttpJsonRpcTransport {
    pub rpc_url: String,
    pub enabled: bool,
}

#[async_trait]
impl JsonRpcTransport for HttpJsonRpcTransport {
    async fn post_json(&self, body: &Value) -> Result<Value, BootstrapError> {
        if !self.enabled
            || self.rpc_url.contains("YOUR_")
            || self.rpc_url.is_empty()
        {
            return Err(BootstrapError::Http(format!(
                "HTTP JSON-RPC transport not enabled (url={}); set real RPC_URL and enabled=true",
                self.rpc_url
            )));
        }
        // Live path: would `reqwest::Client::post(&self.rpc_url).json(body).send()`.
        // Kept as explicit hook without pulling reqwest until credentials exist.
        let _ = body;
        Err(BootstrapError::Http(
            "reqwest not linked — enable live RPC feature / wire client (see bootstrap.rs)"
                .into(),
        ))
    }
}

pub struct JsonRpcBootstrap<T: JsonRpcTransport> {
    pub transport: T,
    pub commitment: String,
}

#[async_trait]
impl<T: JsonRpcTransport> RpcBootstrap for JsonRpcBootstrap<T> {
    async fn get_multiple_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<RawAccount>>, BootstrapError> {
        let key_strs: Vec<String> = keys.iter().map(|k| format!("{k}")).collect();
        let body = jsonrpc_get_multiple_accounts(&key_strs, &self.commitment);
        let _resp = self.transport.post_json(&body).await?;
        Err(BootstrapError::Rpc(
            "live decode not wired — use FixtureBootstrap or enable HTTP client".into(),
        ))
    }

    async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
    ) -> Result<Vec<RawAccount>, BootstrapError> {
        let body = jsonrpc_get_program_accounts(&format!("{program_id}"), &self.commitment);
        let _resp = self.transport.post_json(&body).await?;
        Err(BootstrapError::Rpc(
            "live decode not wired — use FixtureBootstrap or enable HTTP client".into(),
        ))
    }
}

/// Apply a streaming AccountUpdate into StateStore.
pub fn apply_account_update(store: &StateStore<Vec<u8>>, update: &crate::AccountUpdate) -> bool {
    store.upsert(StoredAccount::new(
        update.slot,
        update.write_version,
        update.pubkey,
        update.data.clone(),
        update.source,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fixture_bootstrap_loads_by_owner() {
        let boot = FixtureBootstrap::demo_for_protocols();
        let klend = liq_core::programs::klend();
        let accts = boot.get_program_accounts(&klend).await.unwrap();
        assert_eq!(accts.len(), 3);
        let store = StateStore::new();
        apply_raw_to_store(&store, &accts, UpdateSource::Rpc);
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn jsonrpc_shapes() {
        let v = jsonrpc_get_multiple_accounts(&["Abc".into()], "processed");
        assert_eq!(v["method"], "getMultipleAccounts");
        let v2 = jsonrpc_get_program_accounts("Prog", "confirmed");
        assert_eq!(v2["method"], "getProgramAccounts");
    }
}
