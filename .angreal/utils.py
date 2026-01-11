"""Shared utilities for angreal tasks."""

import subprocess
from pathlib import Path
import angreal


def get_project_root():
    """Return the project root directory."""
    # get_root() returns path to .angreal/, we need its parent
    return Path(angreal.get_root()).parent


def run_cargo(args, check=True, capture=False):
    """Run a cargo command in the project root.

    Args:
        args: List of arguments to pass to cargo
        check: If True, raise on non-zero exit
        capture: If True, capture output; otherwise stream to console

    Returns:
        subprocess.CompletedProcess
    """
    cmd = ["cargo"] + args
    project_root = get_project_root()

    if capture:
        result = subprocess.run(
            cmd,
            cwd=project_root,
            capture_output=True,
            text=True
        )
    else:
        # Stream output directly to console
        result = subprocess.run(
            cmd,
            cwd=project_root,
            text=True
        )

    if check and result.returncode != 0:
        raise subprocess.CalledProcessError(
            result.returncode, cmd,
            result.stdout if capture else None,
            result.stderr if capture else None
        )

    return result
