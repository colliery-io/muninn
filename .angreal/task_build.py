"""Build tasks."""

import angreal
from utils import run_cargo

# Create the build command group
build = angreal.command_group(name="build", about="Build commands")


@build()
@angreal.command(
    name="debug",
    about="Build debug artifacts",
    tool=angreal.ToolDescription(
        """
        Build the workspace in debug mode.

        ## When to use
        - During development for fast compilation
        - For debugging with symbols
        """,
        risk_level="safe"
    )
)
def build_debug():
    """Build debug."""
    print("Building workspace (debug)...")
    try:
        run_cargo(["build", "--workspace"])
        print("Build complete!")
        return 0
    except Exception:
        print("Build failed.")
        return 1


@build()
@angreal.command(
    name="release",
    about="Build release artifacts",
    tool=angreal.ToolDescription(
        """
        Build the workspace in release mode with optimizations.

        ## When to use
        - Before releasing
        - For performance testing
        """,
        risk_level="safe"
    )
)
def build_release():
    """Build release."""
    print("Building workspace (release)...")
    try:
        run_cargo(["build", "--workspace", "--release"])
        print("Build complete!")
        return 0
    except Exception:
        print("Build failed.")
        return 1
