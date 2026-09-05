use liq_telemetry::Metrics;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskLimits {
    pub max_concurrent_liquidations: u32,
    pub max_notional_per_liq_usd_micro: u64,
    pub max_notional_per_minute_usd_micro: u64,
    pub max_consecutive_failures: u32,
    pub oracle_max_staleness_slots: u64,
    pub pause_on_breaker: bool,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_concurrent_liquidations: 4,
            max_notional_per_liq_usd_micro: 500_000_000_000, // $500k
            max_notional_per_minute_usd_micro: 2_000_000_000_000, // $2M
            max_consecutive_failures: 10,
            oracle_max_staleness_slots: 30,
            pause_on_breaker: true,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RiskReject {
    #[error("circuit breaker open: {0}")]
    CircuitOpen(&'static str),
    #[error("exceeds max notional")]
    MaxNotional,
    #[error("exceeds rolling notional budget")]
    RollingNotional,
    #[error("too many concurrent liquidations")]
    Concurrency,
    #[error("oracle too stale")]
    StaleOracle,
    #[error("globally paused")]
    Paused,
}

#[derive(Debug)]
struct RollingWindow {
    events: Vec<(Instant, u64)>,
    window: Duration,
}

impl RollingWindow {
    fn new(window: Duration) -> Self {
        Self {
            events: Vec::new(),
            window,
        }
    }
    fn prune(&mut self, now: Instant) {
        self.events.retain(|(t, _)| now.duration_since(*t) <= self.window);
    }
    fn sum(&mut self, now: Instant) -> u64 {
        self.prune(now);
        self.events.iter().map(|(_, n)| *n).sum()
    }
    fn push(&mut self, now: Instant, n: u64) {
        self.prune(now);
        self.events.push((now, n));
    }
}

#[derive(Debug)]
pub struct CircuitBreaker {
    limits: RiskLimits,
    open: RwLock<bool>,
    reason: RwLock<Option<&'static str>>,
    consecutive_failures: RwLock<u32>,
    in_flight: RwLock<u32>,
    rolling: RwLock<RollingWindow>,
    paused: RwLock<bool>,
    metrics: Arc<Metrics>,
}

impl CircuitBreaker {
    pub fn new(limits: RiskLimits, metrics: Arc<Metrics>) -> Self {
        Self {
            limits,
            open: RwLock::new(false),
            reason: RwLock::new(None),
            consecutive_failures: RwLock::new(0),
            in_flight: RwLock::new(0),
            rolling: RwLock::new(RollingWindow::new(Duration::from_secs(60))),
            paused: RwLock::new(false),
            metrics,
        }
    }

    pub fn set_paused(&self, paused: bool) {
        *self.paused.write() = paused;
    }

    pub fn is_open(&self) -> bool {
        *self.open.read() || *self.paused.read()
    }

    pub fn trip(&self, reason: &'static str) {
        *self.open.write() = true;
        *self.reason.write() = Some(reason);
        self.metrics.circuit_breaker_trips.inc();
    }

    pub fn reset(&self) {
        *self.open.write() = false;
        *self.reason.write() = None;
        *self.consecutive_failures.write() = 0;
    }

    pub fn check_allow(
        &self,
        notional_usd_micro: u64,
        oracle_staleness_slots: u64,
    ) -> Result<(), RiskReject> {
        if *self.paused.read() {
            return Err(RiskReject::Paused);
        }
        if *self.open.read() {
            return Err(RiskReject::CircuitOpen(
                self.reason.read().unwrap_or("open"),
            ));
        }
        if oracle_staleness_slots > self.limits.oracle_max_staleness_slots {
            return Err(RiskReject::StaleOracle);
        }
        if notional_usd_micro > self.limits.max_notional_per_liq_usd_micro {
            return Err(RiskReject::MaxNotional);
        }
        if *self.in_flight.read() >= self.limits.max_concurrent_liquidations {
            return Err(RiskReject::Concurrency);
        }
        let mut rolling = self.rolling.write();
        let now = Instant::now();
        let sum = rolling.sum(now);
        if sum.saturating_add(notional_usd_micro) > self.limits.max_notional_per_minute_usd_micro {
            return Err(RiskReject::RollingNotional);
        }
        Ok(())
    }

    pub fn begin(&self, notional_usd_micro: u64) {
        *self.in_flight.write() += 1;
        self.rolling.write().push(Instant::now(), notional_usd_micro);
    }

    pub fn end_success(&self) {
        {
            let mut in_flight = self.in_flight.write();
            *in_flight = in_flight.saturating_sub(1);
        }
        *self.consecutive_failures.write() = 0;
    }

    pub fn end_failure(&self) {
        {
            let mut in_flight = self.in_flight.write();
            *in_flight = in_flight.saturating_sub(1);
        }
        let mut f = self.consecutive_failures.write();
        *f += 1;
        let should_trip = *f >= self.limits.max_consecutive_failures && self.limits.pause_on_breaker;
        drop(f);
        if should_trip {
            self.trip("max_consecutive_failures");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_trips_on_failures() {
        let metrics = Arc::new(Metrics::new());
        let mut limits = RiskLimits::default();
        limits.max_consecutive_failures = 3;
        let cb = CircuitBreaker::new(limits, metrics);
        assert!(cb.check_allow(1_000_000, 0).is_ok());
        cb.begin(1_000_000);
        cb.end_failure();
        cb.begin(1_000_000);
        cb.end_failure();
        cb.begin(1_000_000);
        cb.end_failure();
        assert!(cb.is_open());
        assert!(matches!(
            cb.check_allow(1_000_000, 0),
            Err(RiskReject::CircuitOpen(_))
        ));
    }

    #[test]
    fn rejects_stale_oracle() {
        let metrics = Arc::new(Metrics::new());
        let cb = CircuitBreaker::new(RiskLimits::default(), metrics);
        assert!(matches!(
            cb.check_allow(1_000_000, 100),
            Err(RiskReject::StaleOracle)
        ));
    }
}
