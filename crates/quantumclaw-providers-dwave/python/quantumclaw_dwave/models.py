"""Request parsing and BQM construction for the QuantumClaw D-Wave bridge."""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any

from .errors import INSTALL_HINT, INVALID_BQM, INVALID_REQUEST, OCEAN_MISSING, BridgeError

PROTOCOL_VERSION = 1

BACKENDS = ("simulated_annealing", "exact", "hybrid", "qpu")


@dataclass
class BqmSpec:
    """A binary quadratic model in minimization form over binary variables."""

    variables: list[str]
    linear: dict[str, float]
    quadratic: dict[tuple[str, str], float]
    offset: float = 0.0

    @property
    def num_variables(self) -> int:
        return len(self.variables)

    @property
    def num_interactions(self) -> int:
        return len(self.quadratic)


@dataclass
class BridgeRequest:
    backend: str
    bqm: BqmSpec
    problem_id: str = "problem"
    parameters: dict[str, Any] = field(default_factory=dict)
    options: dict[str, Any] = field(default_factory=dict)


def _finite(value: Any, context: str) -> float:
    try:
        number = float(value)
    except (TypeError, ValueError) as exc:
        raise BridgeError(INVALID_BQM, f"{context} must be a number, got {value!r}", exc) from exc
    if not math.isfinite(number):
        raise BridgeError(INVALID_BQM, f"{context} must be finite, got {number!r}")
    return number


def parse_bqm(payload: Any) -> BqmSpec:
    if not isinstance(payload, dict):
        raise BridgeError(INVALID_BQM, "bqm must be an object")

    raw_variables = payload.get("variables")
    if not isinstance(raw_variables, list) or not raw_variables:
        raise BridgeError(INVALID_BQM, "bqm.variables must be a non-empty list")

    variables: list[str] = []
    for entry in raw_variables:
        if not isinstance(entry, str) or not entry:
            raise BridgeError(INVALID_BQM, f"bqm.variables entries must be non-empty strings, got {entry!r}")
        if entry in variables:
            raise BridgeError(INVALID_BQM, f"bqm.variables contains a duplicate: {entry!r}")
        variables.append(entry)

    known = set(variables)
    linear: dict[str, float] = {name: 0.0 for name in variables}
    for entry in payload.get("linear", []):
        if not (isinstance(entry, (list, tuple)) and len(entry) == 2):
            raise BridgeError(INVALID_BQM, f"each linear term must be [variable, coefficient], got {entry!r}")
        name, coefficient = entry
        if name not in known:
            raise BridgeError(INVALID_BQM, f"linear term references undeclared variable {name!r}")
        linear[name] += _finite(coefficient, f"linear coefficient for {name!r}")

    quadratic: dict[tuple[str, str], float] = {}
    for entry in payload.get("quadratic", []):
        if not (isinstance(entry, (list, tuple)) and len(entry) == 3):
            raise BridgeError(
                INVALID_BQM, f"each quadratic term must be [first, second, coefficient], got {entry!r}"
            )
        first, second, coefficient = entry
        for name in (first, second):
            if name not in known:
                raise BridgeError(INVALID_BQM, f"quadratic term references undeclared variable {name!r}")
        if first == second:
            raise BridgeError(INVALID_BQM, f"quadratic term uses the same variable twice: {first!r}")
        key = (first, second) if first <= second else (second, first)
        quadratic[key] = quadratic.get(key, 0.0) + _finite(
            coefficient, f"quadratic coefficient for {first!r},{second!r}"
        )

    return BqmSpec(
        variables=variables,
        linear=linear,
        quadratic=quadratic,
        offset=_finite(payload.get("offset", 0.0), "bqm.offset"),
    )


def parse_request(payload: Any) -> BridgeRequest:
    if not isinstance(payload, dict):
        raise BridgeError(INVALID_REQUEST, "request must be a JSON object")

    version = payload.get("protocol_version", PROTOCOL_VERSION)
    if version != PROTOCOL_VERSION:
        raise BridgeError(
            INVALID_REQUEST,
            f"unsupported protocol version {version!r}; this bridge speaks version {PROTOCOL_VERSION}",
        )

    backend = payload.get("backend")
    if backend not in BACKENDS:
        raise BridgeError(
            INVALID_REQUEST,
            f"unknown backend {backend!r}; expected one of {', '.join(BACKENDS)}",
        )

    parameters = payload.get("parameters") or {}
    options = payload.get("options") or {}
    if not isinstance(parameters, dict) or not isinstance(options, dict):
        raise BridgeError(INVALID_REQUEST, "parameters and options must be objects")

    return BridgeRequest(
        backend=backend,
        bqm=parse_bqm(payload.get("bqm")),
        problem_id=str(payload.get("problem_id", "problem")),
        parameters=parameters,
        options=options,
    )


def build_dimod_bqm(spec: BqmSpec):
    """Builds an Ocean ``BinaryQuadraticModel`` with ``Vartype.BINARY``."""

    try:
        import dimod
    except ImportError as exc:  # pragma: no cover - exercised only without Ocean
        raise BridgeError(OCEAN_MISSING, INSTALL_HINT, exc) from exc

    bqm = dimod.BinaryQuadraticModel(dimod.BINARY)
    for name in spec.variables:
        bqm.add_variable(name, spec.linear.get(name, 0.0))
    for (first, second), coefficient in spec.quadratic.items():
        bqm.add_quadratic(first, second, coefficient)
    bqm.offset = spec.offset
    return bqm
