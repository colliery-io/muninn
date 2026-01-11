"""CI/validation tasks."""

import angreal
from utils import run_cargo


@angreal.command(
    name="ci",
    about="Run full CI validation",
    tool=angreal.ToolDescription(
        """
        Run the complete CI pipeline: format check, lint, build, test.

        ## When to use
        - Before pushing to verify CI will pass
        - As a pre-commit validation

        ## Steps
        1. cargo fmt --check
        2. cargo clippy
        3. cargo build --workspace
        4. cargo test --workspace

        ## Output
        Returns 0 only if ALL checks pass.
        """,
        risk_level="read_only"
    )
)
def ci():
    """Run full CI validation."""
    steps = [
        ("Checking formatting", ["fmt", "--all", "--check"]),
        ("Running clippy", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
        ("Building workspace", ["build", "--workspace", "--all-targets"]),
        ("Running tests", ["test", "--workspace"]),
    ]

    for name, args in steps:
        print(f"\n=== {name} ===")
        try:
            run_cargo(args)
        except Exception:
            print(f"\nCI failed at: {name}")
            return 1

    print("\n=== CI passed! ===")
    return 0
