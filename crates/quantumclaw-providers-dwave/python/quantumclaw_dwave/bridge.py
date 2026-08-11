"""JSON sidecar entry point for the QuantumClaw D-Wave provider.

Reads one request object on stdin and writes one response object on stdout::

    python -m quantumclaw_dwave.bridge < request.json

Stdout carries JSON and nothing else; diagnostics go to stderr. A failed run
still prints a JSON response with ``ok: false`` and exits non-zero, so the
caller always has a structured cause.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any

from .errors import INVALID_REQUEST, BridgeError
from .models import PROTOCOL_VERSION, parse_request
from .samplers import probe, run


def handle(payload: Any) -> dict[str, Any]:
    """Runs one request payload and returns the response payload."""

    try:
        request = parse_request(payload)
        result = run(request)
    except BridgeError as error:
        return {"ok": False, "protocol_version": PROTOCOL_VERSION, "error": error.to_payload()}
    return {"ok": True, "protocol_version": PROTOCOL_VERSION, "result": result}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--probe",
        action="store_true",
        help="report which Ocean components are installed and exit",
    )
    arguments = parser.parse_args(argv)

    if arguments.probe:
        report = {"ok": True, "protocol_version": PROTOCOL_VERSION, "result": probe()}
        json.dump(report, sys.stdout)
        sys.stdout.write("\n")
        return 0

    raw = sys.stdin.read()
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as exc:
        response = {
            "ok": False,
            "protocol_version": PROTOCOL_VERSION,
            "error": BridgeError(INVALID_REQUEST, "stdin did not contain valid JSON", exc).to_payload(),
        }
    else:
        response = handle(payload)

    json.dump(response, sys.stdout)
    sys.stdout.write("\n")
    sys.stdout.flush()
    return 0 if response["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
