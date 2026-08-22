//! D-Wave Ocean provider for QuantumClaw.
//!
//! QuantumClaw stays solver-agnostic: this crate implements the same
//! [`quantumclaw_core::SolverBackend`] contract as every other backend, so
//! D-Wave is a configuration choice rather than an architectural commitment.
//!
//! Five lanes are available:
//!
//! | Registry name  | Execution                                    | Kind               |
//! |----------------|----------------------------------------------|--------------------|
//! | `dwave-sa`     | local classical simulated annealing           | `Classical`        |
//! | `dwave-sqa`    | local emulation of quantum annealing          | `QuantumInspired`  |
//! | `dwave-exact`  | local classical exhaustive search             | `Classical`        |
//! | `dwave-hybrid` | D-Wave Leap managed hybrid solver             | `QuantumHybrid`    |
//! | `dwave-qpu`    | quantum annealing hardware with embedding     | `QuantumAnnealing` |
//!
//! `dwave-sa` is **classical** simulated annealing operating on an
//! Ocean-compatible BQM. It is not a QPU simulator. `dwave-sqa` runs
//! `PathIntegralAnnealingSampler`, a local emulator of quantum annealing
//! dynamics; it is quantum-inspired, not a quantum device.
//!
//! Ocean itself is reached through a Python sidecar (see [`bridge`]), which
//! keeps the Ocean dependency tree out of every QuantumClaw installation that
//! does not want it.

pub mod backends;
pub mod bridge;
pub mod config;
pub mod error;
pub mod models;
pub mod result;

pub use backends::{
    DWaveExactSolverBackend, DWaveLeapHybridBackend, DWaveQpuBackend,
    DWaveSimulatedAnnealingBackend, DWaveSimulatedQuantumAnnealingBackend, NAME_EXACT, NAME_HYBRID,
    NAME_QPU, NAME_SIMULATED_ANNEALING, NAME_SIMULATED_QUANTUM_ANNEALING,
};
pub use bridge::{BridgeExecution, DWaveBridge};
pub use config::{
    DWaveConfig, ExactParams, HybridParams, LeapConfig, QpuParams, SimulatedAnnealingParams,
    SimulatedQuantumAnnealingParams,
};
pub use error::{DWaveError, Result};
pub use models::{BridgeBackend, ProbeReport};
pub use result::{DWaveRunMetadata, PROVIDER};

use quantumclaw_core::SolverRegistry;
use std::sync::Arc;

/// Registers every D-Wave backend under its short name.
///
/// After this call `--backend dwave-sa`, `dwave-sqa`, `dwave-exact`,
/// `dwave-hybrid`, and `dwave-qpu` all resolve. Registration does not contact
/// D-Wave and does not require Ocean to be installed; a missing dependency
/// surfaces at solve time with an actionable message.
pub fn register_backends(registry: &mut SolverRegistry, bridge: Arc<DWaveBridge>) {
    registry.register(Arc::new(DWaveSimulatedAnnealingBackend::new(
        bridge.clone(),
    )));
    registry.register(Arc::new(DWaveSimulatedQuantumAnnealingBackend::new(
        bridge.clone(),
    )));
    registry.register(Arc::new(DWaveExactSolverBackend::new(bridge.clone())));
    registry.register(Arc::new(
        DWaveLeapHybridBackend::new(bridge.clone()).with_leap(LeapConfig::from_env()),
    ));
    registry.register(Arc::new(
        DWaveQpuBackend::new(bridge).with_leap(LeapConfig::from_env()),
    ));
}

/// Registers every D-Wave backend using bridge settings from the environment.
pub fn register_backends_from_env(registry: &mut SolverRegistry) {
    register_backends(registry, Arc::new(DWaveBridge::from_env()));
}
