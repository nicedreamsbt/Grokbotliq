use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counter (Prometheus counter semantics).
#[derive(Debug, Default)]
pub struct Counter {
    value: AtomicU64,
}

impl Counter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn inc(&self) {
        self.add(1);
    }
    pub fn add(&self, n: u64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// Last-value gauge.
#[derive(Debug, Default)]
pub struct Gauge {
    value: AtomicU64,
}

impl Gauge {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, v: u64) {
        self.value.store(v, Ordering::Relaxed);
    }
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// Simple histogram with fixed buckets (seconds-or-us counts).
#[derive(Debug)]
pub struct Histogram {
    // bucket upper bounds inclusive
    bounds: Vec<u64>,
    counts: Vec<AtomicU64>,
    sum: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    pub fn with_buckets(bounds: Vec<u64>) -> Self {
        let n = bounds.len();
        let mut counts = Vec::with_capacity(n + 1);
        for _ in 0..=n {
            counts.push(AtomicU64::new(0));
        }
        Self {
            bounds,
            counts,
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, v: u64) {
        self.sum.fetch_add(v, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        for (i, b) in self.bounds.iter().enumerate() {
            if v <= *b {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        let last = self.counts.len() - 1;
        self.counts[last].fetch_add(1, Ordering::Relaxed);
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
}

/// Core bot metrics registry.
#[derive(Debug)]
pub struct Metrics {
    pub oracle_updates: Counter,
    pub trigger_hits: Counter,
    pub candidates_woken: Counter,
    pub liquidations_attempted: Counter,
    pub liquidations_succeeded: Counter,
    pub liquidations_failed: Counter,
    pub dry_run_skips: Counter,
    pub circuit_breaker_trips: Counter,
    pub candidates_critical: Gauge,
    pub candidates_hot: Gauge,
    pub submit_latency_us: Histogram,
    labels: Mutex<HashMap<String, String>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            oracle_updates: Counter::new(),
            trigger_hits: Counter::new(),
            candidates_woken: Counter::new(),
            liquidations_attempted: Counter::new(),
            liquidations_succeeded: Counter::new(),
            liquidations_failed: Counter::new(),
            dry_run_skips: Counter::new(),
            circuit_breaker_trips: Counter::new(),
            candidates_critical: Gauge::new(),
            candidates_hot: Gauge::new(),
            submit_latency_us: Histogram::with_buckets(vec![100, 500, 1_000, 5_000, 20_000, 100_000]),
            labels: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_label(&self, k: impl Into<String>, v: impl Into<String>) {
        self.labels.lock().insert(k.into(), v.into());
    }

    /// Encode a Prometheus text exposition fragment (no HTTP server yet).
    pub fn encode_prometheus(&self) -> String {
        let mut out = String::new();
        fn line(out: &mut String, name: &str, help: &str, ty: &str, v: u64) {
            out.push_str(&format!("# HELP {name} {help}\n"));
            out.push_str(&format!("# TYPE {name} {ty}\n"));
            out.push_str(&format!("{name} {v}\n"));
        }
        line(&mut out, "liq_oracle_updates_total", "Oracle price updates", "counter", self.oracle_updates.get());
        line(&mut out, "liq_trigger_hits_total", "Price trigger crossings", "counter", self.trigger_hits.get());
        line(&mut out, "liq_candidates_woken_total", "Candidates woken by triggers", "counter", self.candidates_woken.get());
        line(&mut out, "liq_liquidations_attempted_total", "Liquidation attempts", "counter", self.liquidations_attempted.get());
        line(&mut out, "liq_liquidations_succeeded_total", "Successful liquidations", "counter", self.liquidations_succeeded.get());
        line(&mut out, "liq_liquidations_failed_total", "Failed liquidations", "counter", self.liquidations_failed.get());
        line(&mut out, "liq_dry_run_skips_total", "Dry-run skips", "counter", self.dry_run_skips.get());
        line(&mut out, "liq_circuit_breaker_trips_total", "Circuit breaker trips", "counter", self.circuit_breaker_trips.get());
        line(&mut out, "liq_candidates_critical", "CRITICAL band size", "gauge", self.candidates_critical.get());
        line(&mut out, "liq_candidates_hot", "HOT band size", "gauge", self.candidates_hot.get());
        out
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub oracle_updates: u64,
    pub trigger_hits: u64,
    pub liquidations_attempted: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            oracle_updates: self.oracle_updates.get(),
            trigger_hits: self.trigger_hits.get(),
            liquidations_attempted: self.liquidations_attempted.get(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prometheus_encode_contains_counters() {
        let m = Metrics::new();
        m.oracle_updates.inc();
        let s = m.encode_prometheus();
        assert!(s.contains("liq_oracle_updates_total 1"));
    }
}
