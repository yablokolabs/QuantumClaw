# quantumclaw-dwave

The D-Wave Ocean bridge for [QuantumClaw](https://github.com/yablokolabs/QuantumClaw).

QuantumClaw's core is Rust and Ocean is Python, so the two meet at this small
JSON sidecar. The Rust provider spawns `python -m quantumclaw_dwave.bridge`,
writes one request on stdin, and reads one response on stdout.

Not on PyPI yet — install from the repository:

```sh
# Local classical samplers only: simulated annealing and exhaustive search.
# No cloud client, no credentials.
pip install "quantumclaw-dwave[local] @ git+https://github.com/yablokolabs/QuantumClaw#subdirectory=crates/quantumclaw-providers-dwave/python"

# Everything, including the Leap cloud client, QPU sampler, and embedding tools.
pip install "quantumclaw-dwave[dwave] @ git+https://github.com/yablokolabs/QuantumClaw#subdirectory=crates/quantumclaw-providers-dwave/python"
```

Then tell QuantumClaw which interpreter has it:

```sh
export QUANTUMCLAW_DWAVE_PYTHON=$(which python)
```

Check what this host can run:

```sh
python -m quantumclaw_dwave.bridge --probe
```

`SimulatedAnnealingSampler` is a **classical** algorithm. It is not a QPU
simulator. See `docs/providers/dwave.md` in the main repository.
