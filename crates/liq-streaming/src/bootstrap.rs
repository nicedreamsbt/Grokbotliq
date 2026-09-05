//! RPC bootstrap: getAccountInfo / getMultipleAccounts / getProgramAccounts / simulateTransaction.
//! Fixture path for CI; HttpJsonRpcTransport (reqwest) for real RPC when `rpc_url` is configured.

use async_trait::async_trait;
use base64::Engine;
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
    #[error("config: {0}")]
    Config(String),
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
    async fn get_account_info(
        &self,
        key: &Pubkey,
    ) -> Result<Option<RawAccount>, BootstrapError> {
        let mut v = self.get_multiple_accounts(&[*key]).await?;
        Ok(v.pop().flatten())
    }

    async fn get_multiple_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<RawAccount>>, BootstrapError>;

    async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
    ) -> Result<Vec<RawAccount>, BootstrapError>;

    async fn simulate_transaction(
        &self,
        _tx_base64: &str,
        _sig_verify: bool,
    ) -> Result<SimulateResult, BootstrapError> {
        Err(BootstrapError::Rpc(
            "simulate_transaction not implemented for this bootstrap".into(),
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulateResult {
    pub err: Option<Value>,
    pub logs: Vec<String>,
    pub units_consumed: Option<u64>,
    pub raw: Value,
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

    async fn simulate_transaction(
        &self,
        _tx_base64: &str,
        _sig_verify: bool,
    ) -> Result<SimulateResult, BootstrapError> {
        Ok(SimulateResult {
            err: None,
            logs: vec!["fixture-simulate: ok (no broadcast)".into()],
            units_consumed: Some(0),
            raw: json!({"context":{"slot":0},"value":{"err":null}}),
        })
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

pub fn jsonrpc_get_account_info(key: &str, commitment: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [key, { "encoding": "base64", "commitment": commitment }]
    })
}

pub fn jsonrpc_get_multiple_accounts(keys_b58: &[String], commitment: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getMultipleAccounts",
        "params": [
            keys_b58,
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

pub fn jsonrpc_simulate_transaction(tx_base64: &str, sig_verify: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "simulateTransaction",
        "params": [
            tx_base64,
            {
                "encoding": "base64",
                "sigVerify": sig_verify,
                "replaceRecentBlockhash": !sig_verify
            }
        ]
    })
}

fn b64_decode(s: &str) -> Result<Vec<u8>, BootstrapError> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| BootstrapError::Decode(format!("base64: {e}")))
}

fn parse_account_value(pubkey: Pubkey, slot: u64, val: &Value) -> Result<Option<RawAccount>, BootstrapError> {
    if val.is_null() {
        return Ok(None);
    }
    let lamports = val
        .get("lamports")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| BootstrapError::Decode("missing lamports".into()))?;
    let owner_s = val
        .get("owner")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BootstrapError::Decode("missing owner".into()))?;
    let owner = Pubkey::from_base58(owner_s).unwrap_or_else(|| {
        // hex fallback for mocks that don't speak base58 owners
        Pubkey::default()
    });
    let owner = if owner == Pubkey::default() {
        // try raw 32-byte hex
        if let Ok(bytes) = hex_32(owner_s) {
            Pubkey::new(bytes)
        } else {
            Pubkey::from_base58(owner_s).unwrap_or(Pubkey::default())
        }
    } else {
        owner
    };
    let data_arr = val
        .get("data")
        .ok_or_else(|| BootstrapError::Decode("missing data".into()))?;
    let data = if let Some(arr) = data_arr.as_array() {
        let s = arr
            .first()
            .and_then(|v| v.as_str())
            .ok_or_else(|| BootstrapError::Decode("data[0]".into()))?;
        b64_decode(s)?
    } else if let Some(s) = data_arr.as_str() {
        b64_decode(s)?
    } else {
        return Err(BootstrapError::Decode("data encoding".into()));
    };
    let _ = owner_s;
    Ok(Some(RawAccount {
        pubkey,
        lamports,
        owner: Pubkey::from_base58(owner_s).unwrap_or(owner),
        data,
        slot,
    }))
}

fn hex_32(s: &str) -> Result<[u8; 32], ()> {
    if s.len() != 64 {
        return Err(());
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| ())?;
    }
    Ok(out)
}

#[async_trait]
pub trait JsonRpcTransport: Send + Sync {
    async fn post_json(&self, body: &Value) -> Result<Value, BootstrapError>;
}

/// HTTP JSON-RPC transport via reqwest. Fails clearly if URL missing/placeholder.
pub struct HttpJsonRpcTransport {
    pub rpc_url: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for HttpJsonRpcTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpJsonRpcTransport")
            .field("rpc_url", &self.rpc_url)
            .finish()
    }
}

impl HttpJsonRpcTransport {
    pub fn new(rpc_url: impl Into<String>) -> Result<Self, BootstrapError> {
        let rpc_url = rpc_url.into();
        if rpc_url.is_empty() || rpc_url.contains("YOUR_") {
            return Err(BootstrapError::Config(format!(
                "rpc_url missing or placeholder ({rpc_url}); set config rpc_url / RPC_URL"
            )));
        }
        Ok(Self {
            rpc_url,
            client: reqwest::Client::new(),
        })
    }

    /// Construct without validating URL (tests inject mock transport instead).
    pub fn unchecked(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl JsonRpcTransport for HttpJsonRpcTransport {
    async fn post_json(&self, body: &Value) -> Result<Value, BootstrapError> {
        if self.rpc_url.is_empty() || self.rpc_url.contains("YOUR_") {
            return Err(BootstrapError::Config(format!(
                "rpc_url missing or placeholder ({})",
                self.rpc_url
            )));
        }
        let resp = self
            .client
            .post(&self.rpc_url)
            .json(body)
            .send()
            .await
            .map_err(|e| BootstrapError::Http(e.to_string()))?;
        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| BootstrapError::Http(format!("json body ({status}): {e}")))?;
        if !status.is_success() {
            return Err(BootstrapError::Http(format!("HTTP {status}: {val}")));
        }
        if let Some(err) = val.get("error") {
            return Err(BootstrapError::Rpc(err.to_string()));
        }
        Ok(val)
    }
}

/// In-memory transport for unit tests (recorded responses).
pub struct MockJsonRpcTransport {
    pub handler: std::sync::Mutex<Box<dyn FnMut(&Value) -> Result<Value, BootstrapError> + Send>>,
}

impl MockJsonRpcTransport {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(&Value) -> Result<Value, BootstrapError> + Send + 'static,
    {
        Self {
            handler: std::sync::Mutex::new(Box::new(f)),
        }
    }
}

#[async_trait]
impl JsonRpcTransport for MockJsonRpcTransport {
    async fn post_json(&self, body: &Value) -> Result<Value, BootstrapError> {
        let mut h = self.handler.lock().map_err(|e| BootstrapError::Http(e.to_string()))?;
        h(body)
    }
}

pub struct JsonRpcBootstrap<T: JsonRpcTransport> {
    pub transport: T,
    pub commitment: String,
}

impl<T: JsonRpcTransport> JsonRpcBootstrap<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            commitment: "processed".into(),
        }
    }
}

fn result_slot(resp: &Value) -> u64 {
    resp.pointer("/result/context/slot")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

#[async_trait]
impl<T: JsonRpcTransport> RpcBootstrap for JsonRpcBootstrap<T> {
    async fn get_account_info(
        &self,
        key: &Pubkey,
    ) -> Result<Option<RawAccount>, BootstrapError> {
        let body = jsonrpc_get_account_info(&key.to_base58(), &self.commitment);
        let resp = self.transport.post_json(&body).await?;
        let slot = result_slot(&resp);
        let val = resp
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null);
        parse_account_value(*key, slot, &val)
    }

    async fn get_multiple_accounts(
        &self,
        keys: &[Pubkey],
    ) -> Result<Vec<Option<RawAccount>>, BootstrapError> {
        let key_strs: Vec<String> = keys.iter().map(|k| k.to_base58()).collect();
        let body = jsonrpc_get_multiple_accounts(&key_strs, &self.commitment);
        let resp = self.transport.post_json(&body).await?;
        let slot = result_slot(&resp);
        let arr = resp
            .pointer("/result/value")
            .and_then(|v| v.as_array())
            .ok_or_else(|| BootstrapError::Decode("result.value array".into()))?;
        let mut out = Vec::with_capacity(keys.len());
        for (i, key) in keys.iter().enumerate() {
            let val = arr.get(i).cloned().unwrap_or(Value::Null);
            out.push(parse_account_value(*key, slot, &val)?);
        }
        Ok(out)
    }

    async fn get_program_accounts(
        &self,
        program_id: &Pubkey,
    ) -> Result<Vec<RawAccount>, BootstrapError> {
        let body = jsonrpc_get_program_accounts(&program_id.to_base58(), &self.commitment);
        let resp = self.transport.post_json(&body).await?;
        let slot = result_slot(&resp);
        let arr = resp
            .pointer("/result")
            .and_then(|v| v.as_array())
            .ok_or_else(|| BootstrapError::Decode("result array".into()))?;
        let mut out = Vec::new();
        for item in arr {
            let pk_s = item
                .get("pubkey")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BootstrapError::Decode("pubkey".into()))?;
            let pubkey = Pubkey::from_base58(pk_s).ok_or_else(|| {
                BootstrapError::Decode(format!("bad pubkey {pk_s}"))
            })?;
            let account = item
                .get("account")
                .ok_or_else(|| BootstrapError::Decode("account".into()))?;
            if let Some(raw) = parse_account_value(pubkey, slot, account)? {
                out.push(raw);
            }
        }
        Ok(out)
    }

    async fn simulate_transaction(
        &self,
        tx_base64: &str,
        sig_verify: bool,
    ) -> Result<SimulateResult, BootstrapError> {
        let body = jsonrpc_simulate_transaction(tx_base64, sig_verify);
        let resp = self.transport.post_json(&body).await?;
        let value = resp
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null);
        let err = value.get("err").cloned().filter(|e| !e.is_null());
        let logs = value
            .get("logs")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let units_consumed = value.get("unitsConsumed").and_then(|v| v.as_u64());
        Ok(SimulateResult {
            err,
            logs,
            units_consumed,
            raw: resp,
        })
    }
}

/// True when URL looks like a real endpoint (not placeholder / empty).
pub fn rpc_url_configured(url: &str) -> bool {
    let t = url.trim();
    !t.is_empty() && !t.contains("YOUR_") && (t.starts_with("http://") || t.starts_with("https://"))
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

/// Serialize a list of instructions into a shadow payload for simulateTransaction.
/// Not a full Solana VersionedTransaction — a portable JSON envelope the RPC mock
/// accepts; live path base64-encodes this envelope until solana-sdk signing is wired.
pub fn shadow_tx_envelope(ixs: &[liq_core::Instruction], recent_blockhash: &str) -> Value {
    json!({
        "mode": "shadow",
        "broadcast": false,
        "recentBlockhash": recent_blockhash,
        "instructions": ixs.iter().map(|ix| json!({
            "programId": ix.program_id.to_base58(),
            "accounts": ix.accounts.iter().map(|m| json!({
                "pubkey": m.pubkey.to_base58(),
                "isSigner": m.is_signer,
                "isWritable": m.is_writable,
            })).collect::<Vec<_>>(),
            "data": base64::engine::general_purpose::STANDARD.encode(&ix.data),
        })).collect::<Vec<_>>(),
    })
}

pub fn shadow_tx_base64(ixs: &[liq_core::Instruction], recent_blockhash: &str) -> String {
    let v = shadow_tx_envelope(ixs, recent_blockhash);
    base64::engine::general_purpose::STANDARD.encode(v.to_string().as_bytes())
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
        let v3 = jsonrpc_simulate_transaction("AQ==", false);
        assert_eq!(v3["method"], "simulateTransaction");
        assert_eq!(v3["params"][1]["sigVerify"], false);
    }

    #[test]
    fn http_transport_rejects_placeholder_url() {
        let err = HttpJsonRpcTransport::new("https://YOUR_PRIVATE_RPC").unwrap_err();
        assert!(matches!(err, BootstrapError::Config(_)));
    }

    #[tokio::test]
    async fn mock_transport_bootstrap_and_simulate() {
        let pk = Pubkey::test(9, 1);
        let owner = liq_core::programs::klend();
        let data_b64 = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
        let transport = MockJsonRpcTransport::new(move |body| {
            let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");
            match method {
                "getMultipleAccounts" => Ok(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "context": {"slot": 42},
                        "value": [{
                            "lamports": 10,
                            "owner": owner.to_base58(),
                            "data": [data_b64.clone(), "base64"],
                            "executable": false,
                            "rentEpoch": 0
                        }]
                    }
                })),
                "simulateTransaction" => Ok(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "context": {"slot": 42},
                        "value": {
                            "err": null,
                            "logs": ["Program log: shadow-ok"],
                            "unitsConsumed": 1234
                        }
                    }
                })),
                other => Err(BootstrapError::Rpc(format!("unexpected {other}"))),
            }
        });
        let boot = JsonRpcBootstrap::new(transport);
        let accts = boot.get_multiple_accounts(&[pk]).await.unwrap();
        assert_eq!(accts.len(), 1);
        assert_eq!(accts[0].as_ref().unwrap().data, vec![1, 2, 3]);
        assert_eq!(accts[0].as_ref().unwrap().slot, 42);

        let ix = liq_core::compute_unit_limit(200_000);
        let b64 = shadow_tx_base64(&[ix], "11111111111111111111111111111111");
        let sim = boot.simulate_transaction(&b64, false).await.unwrap();
        assert!(sim.err.is_none());
        assert_eq!(sim.units_consumed, Some(1234));
        assert!(sim.logs.iter().any(|l| l.contains("shadow-ok")));
    }

    #[test]
    fn rpc_url_configured_helper() {
        assert!(!rpc_url_configured("https://YOUR_PRIVATE_RPC"));
        assert!(!rpc_url_configured(""));
        assert!(rpc_url_configured("https://api.mainnet-beta.solana.com"));
    }
}
