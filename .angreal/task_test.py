"""Testing tasks."""

import angreal
from utils import run_cargo

# Create the test command group
test = angreal.command_group(name="test", about="Testing commands")


@test()
@angreal.command(
    name="unit",
    about="Run unit tests",
    tool=angreal.ToolDescription(
        """
        Run unit tests (lib tests within each crate).

        ## When to use
        - Fast feedback during development
        - Testing individual functions/modules
        """,
        risk_level="safe"
    )
)
def test_unit():
    """Run unit tests only."""
    print("Running unit tests...")
    try:
        run_cargo(["test", "--workspace", "--lib"])
        print("Unit tests passed!")
        return 0
    except Exception:
        print("Unit tests failed.")
        return 1


@test()
@angreal.command(
    name="integration",
    about="Run integration tests",
    tool=angreal.ToolDescription(
        """
        Run integration tests (tests/ directories in each crate + workspace).

        ## When to use
        - Testing crate APIs and cross-crate interactions
        - Before committing larger changes
        """,
        risk_level="safe"
    )
)
def test_integration():
    """Run integration tests."""
    print("Running integration tests...")
    try:
        run_cargo(["test", "--workspace", "--test", "*"])
        print("Integration tests passed!")
        return 0
    except Exception:
        print("Integration tests failed.")
        return 1


@test()
@angreal.command(
    name="all",
    about="Run all tests",
    tool=angreal.ToolDescription(
        """
        Run complete test suite (unit + integration).

        ## When to use
        - Before committing or pushing
        - Full validation
        """,
        risk_level="safe"
    )
)
def test_all():
    """Run all tests."""
    print("Running all tests...")
    try:
        run_cargo(["test", "--workspace"])
        print("All tests passed!")
        return 0
    except Exception:
        print("Tests failed.")
        return 1


@test()
@angreal.command(
    name="crate",
    about="Test a specific crate",
    tool=angreal.ToolDescription(
        """
        Run all tests for a specific crate.

        ## Examples
        ```
        angreal test crate -n muninn-graph
        angreal test crate -n muninn-llm
        ```
        """,
        risk_level="safe"
    )
)
@angreal.argument(
    name="name",
    short="n",
    long="name",
    required=True,
    help="Crate name to test"
)
def test_crate(name):
    """Test specific crate."""
    print(f"Testing {name}...")
    try:
        run_cargo(["test", "--package", name])
        print(f"{name} tests passed!")
        return 0
    except Exception:
        print(f"{name} tests failed.")
        return 1
