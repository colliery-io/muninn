"""Benchmark tasks for muninn."""

import os
import pathlib
import subprocess

import angreal

bench = angreal.command_group(name="bench", about="Benchmarking commands")


def _repo_root() -> pathlib.Path:
    return pathlib.Path.cwd()


@bench()
@angreal.command(
    name="hook",
    about="Benchmark `muninn hook decide` end-to-end latency vs. NFR-001",
    tool=angreal.ToolDescription(
        """
        Measure `muninn hook decide` end-to-end wall-clock latency
        against the configured backend (default: Ollama Cloud +
        gemma4:31b from the tiered config). Reports p50 / p95 / p99
        as JSON to stdout and emits a NFR-001 verdict on stderr.

        Loads credentials from `tests/secrets/uat.enc.yaml` via
        `sops exec-env` when present; falls back to shell env vars.

        ## Examples
        ```
        angreal bench hook                # 100 iters, 1 warmup
        angreal bench hook --iters 25     # quicker, less stable p99
        ```

        Build the release binary first for representative numbers:
        ```
        cargo build --release -p muninn
        ```
        """,
        risk_level="safe",
    ),
)
@angreal.argument(
    name="iters",
    long="iters",
    required=False,
    help="Number of measured iterations (default 100)",
)
@angreal.argument(
    name="warmup",
    long="warmup",
    required=False,
    help="Number of warmup iterations excluded from stats (default 1)",
)
def bench_hook(iters=None, warmup=None):
    """Run the hook latency benchmark."""
    repo_root = _repo_root()
    secrets_file = repo_root / "tests" / "secrets" / "uat.enc.yaml"

    cargo_cmd = ["cargo", "run", "--release", "-p", "muninn",
                 "--example", "bench_hook", "--"]
    if iters:
        cargo_cmd.extend(["--iters", str(iters)])
    if warmup:
        cargo_cmd.extend(["--warmup", str(warmup)])

    if secrets_file.exists():
        print(
            f"  Using sops-encrypted secrets from "
            f"{secrets_file.relative_to(repo_root)}"
        )
        cmd = ["sops", "exec-env", str(secrets_file), " ".join(cargo_cmd)]
    else:
        print("  No sops bundle — falling back to shell env vars")
        cmd = cargo_cmd

    try:
        subprocess.run(cmd, env=os.environ.copy(), check=True)
        return 0
    except subprocess.CalledProcessError:
        print("Benchmark exited non-zero (likely binary-locate failure).")
        return 1
