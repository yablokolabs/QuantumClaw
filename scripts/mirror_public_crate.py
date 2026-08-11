#!/usr/bin/env python3
"""Regenerates the published `quantumclaw` crate from the internal crates.

QuantumClaw ships as a single public crate while every implementation crate
stays `publish = false`. The public crate therefore mirrors internal sources
with their imports rewritten. Doing that by hand drifts, so this script is the
source of truth and CI fails when running it produces a diff.

    python3 scripts/mirror_public_crate.py
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"
PUBLIC_SRC = CRATES / "quantumclaw" / "src"

# internal crate directory -> module name inside the public crate
MIRRORED = {
    "quantumclaw-core": "quantumclaw_core",
    "quantumclaw-ir": "quantumclaw_ir",
    "quantumclaw-optimization": "quantumclaw_optimization",
    "quantumclaw-memory": "quantumclaw_memory",
    "quantumclaw-planner": "quantumclaw_planner",
    "quantumclaw-policy": "quantumclaw_policy",
    "quantumclaw-runtime": "quantumclaw_runtime",
    "quantumclaw-skills": "quantumclaw_skills",
    "quantumclaw-tools": "quantumclaw_tools",
    "quantumclaw-observability": "quantumclaw_observability",
    "quantumclaw-solvers-classical": "quantumclaw_solvers_classical",
    "quantumclaw-solvers-qinspired": "quantumclaw_solvers_qinspired",
    "quantumclaw-solvers-future-qpu": "quantumclaw_solvers_future_qpu",
    "quantumclaw-providers-dwave": "quantumclaw_providers_dwave",
    "quantumclaw-brains": "quantumclaw_brains",
    "quantumclaw-brains-router": "quantumclaw_brains_router",
}

ALL_MODULES = sorted(MIRRORED.values())


def rewrite(source: str, module: str) -> str:
    """Rewrites crate-relative and sibling-crate paths for the public crate."""

    # `crate::x` inside an internal crate becomes `crate::<module>::x`.
    source = re.sub(r"\bcrate::(?!" + module + r"\b)", f"crate::{module}::", source)
    # A sibling crate path becomes a module path, unless it is already prefixed.
    for other in ALL_MODULES:
        source = re.sub(
            r"(?<!crate::)(?<!\w)" + other + r"::",
            f"crate::{other}::",
            source,
        )
    # `crate::<module>::` self-references collapse back to plain `crate::`.
    return source


def mirror_crate(directory: str, module: str) -> list[Path]:
    src = CRATES / directory / "src"
    written: list[Path] = []

    module_dir = PUBLIC_SRC / module
    if module_dir.exists():
        shutil.rmtree(module_dir)

    for path in sorted(src.rglob("*.rs")):
        relative = path.relative_to(src)
        if relative == Path("lib.rs"):
            target = PUBLIC_SRC / f"{module}.rs"
        else:
            target = module_dir / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(rewrite(path.read_text(), module))
        written.append(target)

    return written


def format_rust(paths: list[Path]) -> None:
    """Rewriting imports disturbs their order, so hand the result to rustfmt.

    Without this the mirror would be a permanent `cargo fmt --check` failure.
    """

    if not paths:
        return
    try:
        subprocess.run(
            ["rustfmt", "--edition", "2021", *[str(path) for path in paths]],
            check=True,
            capture_output=True,
        )
    except FileNotFoundError:
        print("rustfmt not found; run `cargo fmt --all` after mirroring", file=sys.stderr)
    except subprocess.CalledProcessError as error:
        print(error.stderr.decode(), file=sys.stderr)
        raise


def main() -> int:
    if not PUBLIC_SRC.exists():
        print(f"missing public crate source at {PUBLIC_SRC}", file=sys.stderr)
        return 1

    written: list[Path] = []
    for directory, module in MIRRORED.items():
        files = mirror_crate(directory, module)
        written.extend(files)
        print(f"{directory} -> {module} ({len(files)} file(s))")

    format_rust(written)
    print(f"mirrored {len(written)} file(s) into {PUBLIC_SRC.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
