//! Jito bundle sender trait + in-memory mock (no network).

use crate::{ExecError, PreparedTx, SubmitResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JitoBundle {
    pub txs: Vec<PreparedTx>,
    pub tip_lamports: u64,
}

#[async_trait]
pub trait JitoBundleSender: Send + Sync {
    async fn send_bundle(&self, bundle: &JitoBundle) -> Result<SubmitResult, ExecError>;
}

#[derive(Default)]
pub struct MockJitoSender {
    pub sent: Mutex<Vec<JitoBundle>>,
    pub fail: Mutex<bool>,
}

impl MockJitoSender {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fail(&self, fail: bool) {
        *self.fail.lock() = fail;
    }

    pub fn sent_count(&self) -> usize {
        self.sent.lock().len()
    }
}

#[async_trait]
impl JitoBundleSender for MockJitoSender {
    async fn send_bundle(&self, bundle: &JitoBundle) -> Result<SubmitResult, ExecError> {
        if *self.fail.lock() {
            return Err(ExecError::Submit("mock jito fail".into()));
        }
        self.sent.lock().push(bundle.clone());
        Ok(SubmitResult {
            signature: Some(format!("mock-jito-{}", self.sent_count())),
            dry_run: false,
            accepted: true,
            detail: format!("mock bundle len={} tip={}", bundle.txs.len(), bundle.tip_lamports),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_records_bundle() {
        let m = MockJitoSender::new();
        let bundle = JitoBundle {
            txs: vec![PreparedTx {
                label: "x".into(),
                protocol: "save".into(),
                account: "a".into(),
                notional_usd_micro: 1,
                expected_profit_usd_micro: 1,
                wire: vec![1, 2],
                ixs: vec![],
            }],
            tip_lamports: 10_000,
        };
        let r = m.send_bundle(&bundle).await.unwrap();
        assert!(r.accepted);
        assert_eq!(m.sent_count(), 1);
    }
}
