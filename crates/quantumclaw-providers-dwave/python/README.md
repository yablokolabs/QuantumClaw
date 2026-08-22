# quantumclaw-dwave

The D-Wave Ocean bridge for [QuantumClaw](https://github.com/yablokolabs/QuantumClaw).

QuantumClaw's core is Rust and Ocean is Python, so the two meet at this small
JSON sidecar. The Rust provider spawns `python -m quantumclaw_dwave.bridge`,
writes one request on stdin, and reads one response on stdout.

```sh
# Local samplers only: classical simulated annealing, the quantum annealing
# emulator (PathIntegralAnnealingSampler), and exhaustive search.
# No cloud client, no credentials.
pip install "quantumclaw-dwave[local]"

# Everything, including the Leap cloud client, QPU sampler, and embedding tools.
pip install "quantumclaw-dwave[dwave]"
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
simulator. `PathIntegralAnnealingSampler` (`dwave-sqa`) is a **local emulator**
of quantum annealing dynamics — quantum-inspired, not a quantum device. See
`docs/providers/dwave.md` in the main repository.
