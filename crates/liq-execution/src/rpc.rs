//! Multi-RPC sender trait + mock with failover (no network).

use crate::{ExecError, PreparedTx, SubmitResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEndpoint {
    pub name: String,
    pub url: String,
}

#[async_trait]
pub trait MultiRpcSender: Send + Sync {
    async fn send_tx(&self, tx: &PreparedTx) -> Result<SubmitResult, ExecError>;
}

#[derive(Default)]
pub struct MockMultiRpc {
    pub endpoints: Vec<RpcEndpoint>,
    /// Indices that should fail.
    pub failing: Mutex<Vec<usize>>,
    pub attempts: Mutex<Vec<String>>,
}

impl MockMultiRpc {
    pub fn new(endpoints: Vec<RpcEndpoint>) -> Self {
        Self {
            endpoints,
            failing: Mutex::new(Vec::new()),
            attempts: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl MultiRpcSender for MockMultiRpc {
    async fn send_tx(&self, tx: &PreparedTx) -> Result<SubmitResult, ExecError> {
        let failing = self.failing.lock().clone();
        for (i, ep) in self.endpoints.iter().enumerate() {
            self.attempts.lock().push(ep.name.clone());
            if failing.contains(&i) {
                continue;
            }
            return Ok(SubmitResult {
                signature: Some(format!("mock-rpc-{}-{}", ep.name, tx.label)),
                dry_run: false,
                accepted: true,
                detail: format!("via {}", ep.name),
            });
        }
        Err(ExecError::Submit("all rpc endpoints failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failover_to_second() {
        let m = MockMultiRpc::new(vec![
            RpcEndpoint { name: "a".into(), url: "http://a".into() },
            RpcEndpoint { name: "b".into(), url: "http://b".into() },
        ]);
        *m.failing.lock() = vec![0];
        let tx = PreparedTx {
            label: "t".into(),
            protocol: "p0".into(),
            account: "x".into(),
            notional_usd_micro: 1,
            expected_profit_usd_micro: 1,
            wire: vec![],
            instructions: vec![],
            funding_strategy: None,
            ixs: vec![],
        };
        let r = m.send_tx(&tx).await.unwrap();
        assert!(r.detail.contains("b"));
        assert_eq!(m.attempts.lock().clone(), vec!["a", "b"]);
    }
}
