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
    jsonrpc_get_program_accounts_filtered(program_id, commitment, &[])
}

/// `filters` are Solana RPC filter objects, e.g. `{"dataSize": 3344}` or
/// `{"memcmp": {"offset": 32, "bytes": "<base58>"}}`.
pub fn jsonrpc_get_program_accounts_filtered(
    program_id: &str,
    commitment: &str,
    filters: &[Value],
) -> Value {
    let mut cfg = json!({
        "encoding": "base64",
        "commitment": commitment
    });
    if !filters.is_empty() {
        cfg["filters"] = Value::Array(filters.to_vec());
    }
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [program_id, cfg]
    })
}

pub fn jsonrpc_get_slot(commitment: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getSlot",
        "params": [{ "commitment": commitment }]
    })
}

pub fn jsonrpc_get_health() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getHealth",
        "params": []
    })
}

pub fn jsonrpc_get_latest_blockhash(commitment: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getLatestBlockhash",
        "params": [{ "commitment": commitment }]
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
            .field("rpc_url", &crate::redact::rpc_url_host_only(&self.rpc_url))
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
            .timeout(std::time::Duration::from_secs(25))
            .json(body)
            .send()
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if e.is_timeout() || msg.to_lowercase().contains("timed out") {
                    BootstrapError::Http(format!("timeout: {msg}"))
                } else {
                    BootstrapError::Http(msg)
                }
            })?;
        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| BootstrapError::Http(format!("json body ({status}): {e}")))?;
        if status.as_u16() == 429 {
            return Err(BootstrapError::Http(format!("HTTP 429: {val}")));
        }
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

    pub fn with_commitment(mut self, commitment: impl Into<String>) -> Self {
        self.commitment = commitment.into();
        self
    }

    pub async fn get_slot(&self) -> Result<u64, BootstrapError> {
        let body = jsonrpc_get_slot(&self.commitment);
        let resp = self.transport.post_json(&body).await?;
        resp.get("result")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| BootstrapError::Decode("getSlot result".into()))
    }

    pub async fn get_health(&self) -> Result<String, BootstrapError> {
        let body = jsonrpc_get_health();
        let resp = self.transport.post_json(&body).await?;
        match resp.get("result") {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(v) => Ok(v.to_string()),
            None => Err(BootstrapError::Decode("getHealth result".into())),
        }
    }

    /// Fetch latest blockhash bytes (32) via getLatestBlockhash.
    pub async fn get_latest_blockhash(&self) -> Result<([u8; 32], u64), BootstrapError> {
        let body = jsonrpc_get_latest_blockhash(&self.commitment);
        let resp = self.transport.post_json(&body).await?;
        let hash_s = resp
            .pointer("/result/value/blockhash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BootstrapError::Decode("getLatestBlockhash blockhash".into()))?;
        let bytes = bs58::decode(hash_s)
            .into_vec()
            .map_err(|e| BootstrapError::Decode(format!("blockhash b58: {e}")))?;
        if bytes.len() != 32 {
            return Err(BootstrapError::Decode(format!(
                "blockhash len {}",
                bytes.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        let slot = result_slot(&resp);
        Ok((out, slot))
    }

    pub async fn get_program_accounts_filtered(
        &self,
        program_id: &Pubkey,
        filters: &[Value],
    ) -> Result<Vec<RawAccount>, BootstrapError> {
        let body = jsonrpc_get_program_accounts_filtered(
            &program_id.to_base58(),
            &self.commitment,
            filters,
        );
        let resp = self.transport.post_json(&body).await?;
        let slot = result_slot(&resp);
        // filtered GPA may return result as array OR {value: [...]} depending on provider
        let arr = resp
            .pointer("/result")
            .and_then(|v| v.as_array())
            .or_else(|| resp.pointer("/result/value").and_then(|v| v.as_array()))
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


/// Compact-u16 (Solana shortvec) encode.
fn shortvec_encode(n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = n;
    while v >= 0x80 {
        out.push(((v & 0x7f) as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
    out
}

/// Build a minimal unsigned legacy transaction (ComputeBudget::SetComputeUnitLimit only).
/// Valid wire format for `simulateTransaction` with `sigVerify=false` + `replaceRecentBlockhash`.
/// Fee-payer is the system program id placeholder (64 zero signature).
pub fn minimal_cu_limit_tx_base64(units: u32) -> String {
    minimal_cu_limit_tx_base64_with_payer(units, &[1u8; 32])
}

/// Same as [`minimal_cu_limit_tx_base64`] but with an explicit 32-byte fee-payer pubkey.
/// Prefer a real system-owned mainnet account so simulate does not return AccountNotFound.
pub fn minimal_cu_limit_tx_base64_with_payer(units: u32, fee_payer: &[u8; 32]) -> String {
    use liq_core::programs;
    let fee_payer = *fee_payer;
    let cu_program = programs::compute_budget().0;
    // Message header: 1 required sig, 0 readonly signed, 1 readonly unsigned (program)
    let mut msg = Vec::new();
    msg.push(1u8); // num_required_signatures
    msg.push(0u8); // num_readonly_signed
    msg.push(1u8); // num_readonly_unsigned
    // account keys: fee_payer, compute_budget
    msg.extend_from_slice(&shortvec_encode(2));
    msg.extend_from_slice(&fee_payer);
    msg.extend_from_slice(&cu_program);
    // recent blockhash (zeros — replaced by RPC when replaceRecentBlockhash=true)
    msg.extend_from_slice(&[0u8; 32]);
    // instructions: one compiled ix
    // program_id_index = 1, accounts empty, data = [2] + units le
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    let mut ix = Vec::new();
    ix.push(1u8); // program index
    ix.extend_from_slice(&shortvec_encode(0)); // no accounts
    ix.extend_from_slice(&shortvec_encode(data.len()));
    ix.extend_from_slice(&data);
    msg.extend_from_slice(&shortvec_encode(1));
    msg.extend_from_slice(&ix);
    // signatures: 1 x 64 zero bytes
    let mut tx = Vec::new();
    tx.extend_from_slice(&shortvec_encode(1));
    tx.extend_from_slice(&[0u8; 64]);
    tx.extend_from_slice(&msg);
    base64::engine::general_purpose::STANDARD.encode(tx)
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
        let v2f = jsonrpc_get_program_accounts_filtered(
            "Prog",
            "confirmed",
            &[json!({"dataSize": 3344})],
        );
        assert_eq!(v2f["params"][1]["filters"][0]["dataSize"], 3344);
        let v3 = jsonrpc_simulate_transaction("AQ==", false);
        assert_eq!(v3["method"], "simulateTransaction");
        assert_eq!(v3["params"][1]["sigVerify"], false);
        assert_eq!(jsonrpc_get_slot("processed")["method"], "getSlot");
        assert_eq!(jsonrpc_get_health()["method"], "getHealth");
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

    #[test]
    fn minimal_cu_tx_is_small_wire() {
        let b64 = minimal_cu_limit_tx_base64(200_000);
        let raw = base64::engine::general_purpose::STANDARD.decode(&b64).unwrap();
        assert!(raw.len() < 200, "len={}", raw.len());
        assert!(raw.len() > 64);
    }
}
