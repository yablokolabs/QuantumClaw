"""Typed error codes shared with the QuantumClaw Rust provider.

Every failure crossing the bridge carries one of these codes so the Rust side
can map it onto a typed error instead of parsing free text.
"""

from __future__ import annotations

from typing import Any

OCEAN_MISSING = "ocean_missing"
INVALID_REQUEST = "invalid_request"
INVALID_BQM = "invalid_bqm"
INVALID_CONFIGURATION = "invalid_configuration"
MISSING_CREDENTIALS = "missing_credentials"
AUTHENTICATION_FAILED = "authentication_failed"
SOLVER_UNAVAILABLE = "solver_unavailable"
EMBEDDING_FAILED = "embedding_failed"
TIMEOUT = "timeout"
NO_FEASIBLE_RESULT = "no_feasible_result"
PROBLEM_TOO_LARGE = "problem_too_large"
SAMPLER_FAILED = "sampler_failed"


class BridgeError(Exception):
    """An error that carries a stable code across the bridge."""

    def __init__(self, code: str, message: str, cause: BaseException | str | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        if cause is None:
            self.cause = None
        elif isinstance(cause, str):
            self.cause = cause
        else:
            self.cause = f"{type(cause).__name__}: {cause}"

    def to_payload(self) -> dict[str, Any]:
        payload: dict[str, Any] = {"code": self.code, "message": self.message}
        if self.cause is not None:
            payload["cause"] = self.cause
        return payload
