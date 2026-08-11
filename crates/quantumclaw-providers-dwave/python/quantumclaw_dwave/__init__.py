"""QuantumClaw bridge to the D-Wave Ocean SDK.

This package is the only place where QuantumClaw touches Ocean. The Rust
provider crate spawns :mod:`quantumclaw_dwave.bridge` and exchanges JSON with
it, which keeps Ocean an optional dependency of the Python side alone.
"""

from .errors import BridgeError
from .models import PROTOCOL_VERSION, BqmSpec, BridgeRequest, parse_request

__all__ = ["BridgeError", "BqmSpec", "BridgeRequest", "PROTOCOL_VERSION", "parse_request"]
__version__ = "0.1.0"
