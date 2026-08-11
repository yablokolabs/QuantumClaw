"""Behavioral tests for the QuantumClaw D-Wave bridge.

Tests that need Ocean skip when it is absent. Set
``QUANTUMCLAW_DWAVE_REQUIRE=1`` to turn those skips into failures, which is how
CI proves the Ocean lane actually ran.
"""

from __future__ import annotations

import itertools
import json
import os
import subprocess
import sys

import pytest

from quantumclaw_dwave.bridge import handle, main

REQUIRE_OCEAN = os.environ.get("QUANTUMCLAW_DWAVE_REQUIRE") == "1"

try:  # pragma: no cover - import probing
    import dimod  # noqa: F401
    import dwave.samplers  # noqa: F401

    OCEAN_AVAILABLE = True
except ImportError:  # pragma: no cover - import probing
    OCEAN_AVAILABLE = False

needs_ocean = pytest.mark.skipif(
    not OCEAN_AVAILABLE and not REQUIRE_OCEAN,
    reason="Ocean local samplers are not installed",
)


def request(backend: str, **overrides):
    """A QUBO whose unique minimum is a=1, b=0, c=1 with energy -7."""

    payload = {
        "protocol_version": 1,
        "backend": backend,
        "problem_id": "known-optimum",
        "bqm": {
            "variables": ["a", "b", "c"],
            "linear": [["a", -4.0], ["b", -1.0], ["c", -3.0]],
            "quadratic": [["a", "b", 6.0], ["b", "c", 6.0]],
            "offset": 0.0,
        },
        "parameters": {"num_reads": 50, "seed": 7},
    }
    payload.update(overrides)
    return payload


def brute_force_optimum(payload):
    bqm = payload["bqm"]
    variables = bqm["variables"]
    best = None
    for bits in itertools.product([0, 1], repeat=len(variables)):
        assignment = dict(zip(variables, bits))
        energy = sum(coefficient * assignment[name] for name, coefficient in bqm["linear"])
        energy += sum(
            coefficient * assignment[first] * assignment[second]
            for first, second, coefficient in bqm["quadratic"]
        )
        energy += bqm.get("offset", 0.0)
        if best is None or energy < best[1]:
            best = (assignment, energy)
    return best


@needs_ocean
def test_exact_backend_returns_the_brute_force_optimum():
    payload = request("exact")
    response = handle(payload)

    assert response["ok"], response
    expected_assignment, expected_energy = brute_force_optimum(payload)
    assert response["result"]["best"]["sample"] == expected_assignment
    assert response["result"]["best"]["energy"] == pytest.approx(expected_energy)
    assert response["result"]["num_variables"] == 3


@needs_ocean
def test_simulated_annealing_finds_the_same_optimum_as_exhaustive_search():
    response = handle(request("simulated_annealing"))

    assert response["ok"], response
    expected_assignment, expected_energy = brute_force_optimum(request("exact"))
    assert response["result"]["best"]["sample"] == expected_assignment
    assert response["result"]["best"]["energy"] == pytest.approx(expected_energy)
    assert response["result"]["solver_runtime_ms"] >= 0.0


@needs_ocean
def test_simulated_annealing_is_reproducible_for_a_fixed_seed():
    first = handle(request("simulated_annealing", parameters={"num_reads": 20, "seed": 1234}))
    second = handle(request("simulated_annealing", parameters={"num_reads": 20, "seed": 1234}))

    assert first["result"]["best"] == second["result"]["best"]


@needs_ocean
def test_exact_backend_refuses_a_problem_beyond_its_variable_limit():
    payload = request("exact")
    payload["bqm"] = {
        "variables": [f"x{index}" for index in range(6)],
        "linear": [[f"x{index}", -1.0] for index in range(6)],
        "quadratic": [],
    }
    payload["options"] = {"max_variables": 4}

    response = handle(payload)

    assert not response["ok"]
    assert response["error"]["code"] == "problem_too_large"
    assert "4" in response["error"]["message"]


@needs_ocean
def test_beta_range_must_have_two_entries():
    response = handle(
        request("simulated_annealing", parameters={"num_reads": 10, "beta_range": [0.1]})
    )

    assert not response["ok"]
    assert response["error"]["code"] == "invalid_configuration"


def test_a_quadratic_term_on_an_undeclared_variable_is_rejected():
    payload = request("simulated_annealing")
    payload["bqm"]["quadratic"].append(["a", "ghost", 1.0])

    response = handle(payload)

    assert not response["ok"]
    assert response["error"]["code"] == "invalid_bqm"
    assert "ghost" in response["error"]["message"]


def test_an_unknown_backend_name_is_rejected_with_the_supported_names():
    response = handle(request("annealer-9000"))

    assert not response["ok"]
    assert response["error"]["code"] == "invalid_request"
    assert "simulated_annealing" in response["error"]["message"]


def test_a_future_protocol_version_is_refused():
    response = handle(request("exact", protocol_version=99))

    assert not response["ok"]
    assert response["error"]["code"] == "invalid_request"


def test_cloud_backends_report_missing_credentials_rather_than_hanging(monkeypatch):
    pytest.importorskip("dwave.system")
    monkeypatch.delenv("DWAVE_API_TOKEN", raising=False)
    monkeypatch.setenv("DWAVE_CONFIG_FILE", os.devnull)

    for backend in ("hybrid", "qpu"):
        response = handle(request(backend))
        assert not response["ok"]
        assert response["error"]["code"] in {"missing_credentials", "authentication_failed"}


def test_the_process_prints_only_json_on_stdout_and_exits_non_zero_on_error(tmp_path):
    payload = json.dumps(request("annealer-9000"))
    completed = subprocess.run(
        [sys.executable, "-m", "quantumclaw_dwave.bridge"],
        input=payload,
        capture_output=True,
        text=True,
        check=False,
    )

    assert completed.returncode == 1
    body = json.loads(completed.stdout)
    assert body["ok"] is False
    assert body["error"]["code"] == "invalid_request"


def test_probe_reports_backend_availability(capsys):
    exit_code = main(["--probe"])

    captured = json.loads(capsys.readouterr().out)
    assert exit_code == 0
    assert captured["ok"] is True
    assert set(captured["result"]["backends"]) == {
        "simulated_annealing",
        "exact",
        "hybrid",
        "qpu",
    }


def test_malformed_stdin_produces_a_structured_error():
    completed = subprocess.run(
        [sys.executable, "-m", "quantumclaw_dwave.bridge"],
        input="not json",
        capture_output=True,
        text=True,
        check=False,
    )

    assert completed.returncode == 1
    assert json.loads(completed.stdout)["error"]["code"] == "invalid_request"
