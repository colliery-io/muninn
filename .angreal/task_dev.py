"""Development workflow tasks."""

import angreal
from utils import run_cargo

# Create the dev command group
dev = angreal.command_group(name="dev", about="Development utilities")


@dev()
@angreal.command(
    name="check",
    about="Type-check the workspace",
    tool=angreal.ToolDescription(
        """
        Run cargo check on the entire workspace.

        ## When to use
        - Fast feedback on type errors during development
        - Before committing to catch obvious issues

        ## Output
        Returns 0 if check passes, non-zero otherwise.
        """,
        risk_level="read_only"
    )
)
def dev_check():
    """Run cargo check on the workspace."""
    print("Checking workspace...")
    try:
        run_cargo(["check", "--workspace", "--all-targets"])
        print("Check passed!")
        return 0
    except Exception as e:
        print(f"Check failed: {e}")
        return 1


@dev()
@angreal.command(
    name="fmt",
    about="Format code with rustfmt",
    tool=angreal.ToolDescription(
        """
        Format all Rust code in the workspace.

        ## Examples
        ```
        angreal dev fmt           # Format all code
        angreal dev fmt --check   # Check only
        ```
        """,
        risk_level="safe"
    )
)
@angreal.argument(
    name="check_only",
    long="check",
    is_flag=True,
    takes_value=False,
    help="Check formatting without making changes"
)
def dev_fmt(check_only=False):
    """Format Rust code."""
    args = ["fmt", "--all"]
    if check_only:
        args.append("--check")
        print("Checking formatting...")
    else:
        print("Formatting code...")

    try:
        run_cargo(args)
        print("Formatting OK!")
        return 0
    except Exception:
        if check_only:
            print("Formatting issues found. Run 'angreal dev fmt' to fix.")
        return 1


@dev()
@angreal.command(
    name="lint",
    about="Run clippy lints",
    tool=angreal.ToolDescription(
        """
        Run clippy for common mistakes and style issues.

        ## Examples
        ```
        angreal dev lint          # Run clippy
        angreal dev lint --fix    # Auto-fix where possible
        ```
        """,
        risk_level="read_only"
    )
)
@angreal.argument(
    name="fix",
    long="fix",
    is_flag=True,
    takes_value=False,
    help="Auto-fix issues where possible"
)
def dev_lint(fix=False):
    """Run clippy lints."""
    args = ["clippy", "--workspace", "--all-targets"]
    if fix:
        args.extend(["--fix", "--allow-dirty"])
        print("Running clippy with auto-fix...")
    else:
        print("Running clippy...")

    try:
        run_cargo(args)
        print("Clippy passed!")
        return 0
    except Exception:
        print("Clippy found issues.")
        return 1


@dev()
@angreal.command(
    name="clean",
    about="Clean build artifacts",
    tool=angreal.ToolDescription(
        """
        Remove all build artifacts from target directory.

        ## When to use
        - To free disk space
        - To force a full rebuild
        """,
        risk_level="destructive"
    )
)
def dev_clean():
    """Clean build artifacts."""
    print("Cleaning build artifacts...")
    try:
        run_cargo(["clean"])
        print("Clean complete!")
        return 0
    except Exception as e:
        print(f"Clean failed: {e}")
        return 1
