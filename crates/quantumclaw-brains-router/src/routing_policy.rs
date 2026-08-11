//! Which solver should see which subproblem.
//!
//! Q-Router does not assume quantum methods are better. It starts from size
//! heuristics and then prefers whatever has actually performed best on similar
//! subproblems, recorded in a [`BenchmarkLedger`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One observed solver run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerRecord {
    /// Subproblem class, for example `vehicle-assignment`.
    pub class: String,
    /// Variable-count bucket, so similar sizes compare against each other.
    pub size_bucket: usize,
    pub backend: String,
    pub objective: f64,
    pub feasible: bool,
    pub runtime_ms: u64,
}

/// Rounds a variable count into a comparison bucket.
pub fn size_bucket(variables: usize) -> usize {
    variables.max(1).next_power_of_two()
}

/// Accumulated evidence about solver performance.
///
/// Held in memory, with explicit JSON import/export so a caller can persist it
/// wherever it already stores operational data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkLedger {
    pub records: Vec<LedgerRecord>,
}

impl BenchmarkLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, record: LedgerRecord) -> &mut Self {
        self.records.push(record);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Best backend for a class and size, judged on mean objective across
    /// feasible runs. Infeasible runs never recommend a backend.
    pub fn best_backend(
        &self,
        class: &str,
        variables: usize,
        available: &[String],
    ) -> Option<(String, f64)> {
        let bucket = size_bucket(variables);
        let mut totals: BTreeMap<String, (f64, u32, u64)> = BTreeMap::new();

        for record in &self.records {
            if record.class != class || record.size_bucket != bucket || !record.feasible {
                continue;
            }
            if !available.iter().any(|name| name == &record.backend) {
                continue;
            }
            let entry = totals.entry(record.backend.clone()).or_insert((0.0, 0, 0));
            entry.0 += record.objective;
            entry.1 += 1;
            entry.2 += record.runtime_ms;
        }

        totals
            .into_iter()
            .map(|(backend, (objective, count, runtime))| {
                (
                    backend,
                    objective / f64::from(count),
                    runtime / u64::from(count),
                )
            })
            .min_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.2.cmp(&right.2))
            })
            .map(|(backend, mean_objective, _)| (backend, mean_objective))
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(value: &str) -> serde_json::Result<Self> {
        serde_json::from_str(value)
    }
}

/// The backend chosen for a subproblem, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// `None` means "solve it classically inside the brain".
    pub backend: Option<String>,
    pub reason: String,
}

/// Chooses a backend per subproblem.
#[derive(Debug, Clone)]
pub struct SolverRoutingPolicy {
    /// Above this many binary variables, the brain stops offering the
    /// subproblem to sampling backends and solves it classically.
    pub max_variables_for_sampling: usize,
    /// Backends to try, in order, when no evidence exists yet.
    pub preferred_backends: Vec<String>,
    pub ledger: BenchmarkLedger,
}

impl Default for SolverRoutingPolicy {
    fn default() -> Self {
        Self {
            max_variables_for_sampling: 200,
            preferred_backends: vec!["dwave-sa".into()],
            ledger: BenchmarkLedger::new(),
        }
    }
}

impl SolverRoutingPolicy {
    pub fn with_ledger(mut self, ledger: BenchmarkLedger) -> Self {
        self.ledger = ledger;
        self
    }

    pub fn with_preferred_backends<I, S>(mut self, backends: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.preferred_backends = backends.into_iter().map(Into::into).collect();
        self
    }

    /// Picks a backend for one subproblem.
    ///
    /// Recorded evidence wins over configured preference; an explicit caller
    /// request wins over both and is handled before this is consulted.
    pub fn choose(&self, class: &str, variables: usize, available: &[String]) -> RoutingDecision {
        if available.is_empty() {
            return RoutingDecision {
                backend: None,
                reason: "no solver backends are registered, so the brain solves it classically"
                    .into(),
            };
        }

        if variables > self.max_variables_for_sampling {
            return RoutingDecision {
                backend: None,
                reason: format!(
                    "{variables} variables exceeds the sampling threshold of {}, so the brain solves it classically",
                    self.max_variables_for_sampling
                ),
            };
        }

        if let Some((backend, mean_objective)) =
            self.ledger.best_backend(class, variables, available)
        {
            return RoutingDecision {
                reason: format!(
                    "benchmark evidence: '{backend}' averaged an objective of {mean_objective:.3} on {class} subproblems of this size"
                ),
                backend: Some(backend),
            };
        }

        match self
            .preferred_backends
            .iter()
            .find(|name| available.iter().any(|candidate| &candidate == name))
        {
            Some(backend) => RoutingDecision {
                reason: format!(
                    "no benchmark evidence yet; trying the configured preference '{backend}'"
                ),
                backend: Some(backend.clone()),
            },
            None => RoutingDecision {
                backend: None,
                reason: "no preferred backend is registered, so the brain solves it classically"
                    .into(),
            },
        }
    }
}
