"""Testing tasks."""

import os
import pathlib
import subprocess
import sys

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


# ─────────────────────────────────────────────────────────────────────
# UAT (User Acceptance Testing) against real LLM backends.
#
# These tests exercise the hook + MCP + daemon paths against actual
# Ollama Cloud (or whatever provider the user configures) so we catch
# wiring breakage that mocked tests miss. They're gated on `#[ignore]`
# so normal `cargo test` doesn't try to spend the user's API budget.
#
# Secrets live in `tests/secrets/uat.enc.yaml` (sops + AGE encrypted).
# See `tests/secrets/README.md` for onboarding.
# ─────────────────────────────────────────────────────────────────────


def _repo_root() -> pathlib.Path:
    # angreal sets CWD to the repo root before invoking tasks.
    return pathlib.Path.cwd()


@test()
@angreal.command(
    name="uat",
    about="Run UAT tests against a real LLM backend (sops-decrypted secrets)",
    tool=angreal.ToolDescription(
        """
        Run ignored UAT tests against a real LLM backend.

        Decrypts `tests/secrets/uat.enc.yaml` via `sops exec-env` and
        invokes the workspace's ignored tests. Falls back to whatever
        env vars are in the shell when the encrypted file is absent
        (legacy path for developers who haven't onboarded sops yet).

        ## When to use
        - Validating end-to-end behavior of the hook + MCP + daemon path
        - Verifying provider/model configuration works against the live
          catalog before shipping

        ## Examples
        ```
        angreal test uat                          # default provider (ollama)
        angreal test uat -n routing               # filter by name substring
        angreal test uat --provider groq          # run against Groq
        angreal test uat --provider all           # run for each provider with a key present
        ```

        Provider selection sets `MUNINN_UAT_PROVIDER` (read by the
        test helpers in `crates/muninn/tests/common/mod.rs`). Pass
        `--provider all` to iterate every provider whose API key
        is present in the decrypted secrets bundle — handy for
        cross-backend smoke tests in CI / release prep.

        Requires `sops` + `age` installed and `SOPS_AGE_KEY_FILE` set
        if you want to use the encrypted bundle. See
        `tests/secrets/README.md`.
        """,
        risk_level="safe",
    ),
)
@angreal.argument(
    name="name",
    short="n",
    long="name",
    required=False,
    help="Filter ignored tests by name substring (cargo's standard test filter)",
)
@angreal.argument(
    name="provider",
    long="provider",
    required=False,
    help="LLM provider to target: ollama|groq|anthropic|all (default: ollama)",
)
def test_uat(name=None, provider=None):
    """Run UAT tests against a real LLM."""
    # angreal passes None when --provider is omitted; honor the
    # documented default here, not at the def-site signature.
    if provider is None:
        provider = "ollama"

    repo_root = _repo_root()
    secrets_file = repo_root / "tests" / "secrets" / "uat.enc.yaml"

    # UAT test crates live in dedicated test binaries named `uat`. We
    # target them explicitly rather than `--workspace -- --ignored` so
    # we don't accidentally pull in pre-existing `#[ignore]`'d tests
    # (e.g. muninn-graph's network-dependent doc indexers, which fail
    # for unrelated reasons in this run).
    #
    # Extend this list when adding new UAT targets — e.g. an MCP
    # subprocess UAT in `crates/muninn-rlm/tests/uat.rs`.
    uat_targets = [
        ("muninn", "uat"),
        ("muninn", "daemon_lifecycle"),
        ("muninn", "mcp_protocol"),
        ("muninn", "routing"),
        ("muninn", "user_prompt_submit"),
    ]
    cargo_cmd = ["cargo", "test"]
    for pkg, target in uat_targets:
        cargo_cmd.extend(["-p", pkg, "--test", target])
    cargo_cmd.extend(["--", "--ignored", "--nocapture"])
    if name:
        cargo_cmd.insert(-2, name)  # before the `--`

    # Map provider name -> env var the tests will check at runtime
    # to gate on credential presence (`provider_env_var` in
    # crates/muninn/tests/common/mod.rs).
    provider_env_keys = {
        "ollama": "OLLAMA_API_KEY",
        "groq": "GROQ_API_KEY",
        "anthropic": "ANTHROPIC_API_KEY",
    }

    def run_once(provider_name: str) -> int:
        """Run the cargo command with MUNINN_UAT_PROVIDER set."""
        env = os.environ.copy()
        env["MUNINN_UAT_PROVIDER"] = provider_name
        print(f"\n  ─── UAT pass: provider={provider_name} ───")
        if secrets_file.exists():
            # `sops exec-env` decrypts the file, exports every key as
            # an env var, then execs the inner command. We pass the
            # already-merged env via MUNINN_UAT_PROVIDER inside the
            # exec'd shell; sops's exec-env layers its decrypted keys
            # on top.
            print(
                f"  Using sops-encrypted secrets from "
                f"{secrets_file.relative_to(repo_root)}"
            )
            cmd = ["sops", "exec-env", str(secrets_file), " ".join(cargo_cmd)]
        else:
            print(
                f"  No {secrets_file.relative_to(repo_root)} — falling back "
                f"to shell env vars (e.g. {provider_env_keys.get(provider_name, 'OLLAMA_API_KEY')})"
            )
            cmd = cargo_cmd
        try:
            subprocess.run(cmd, env=env, check=True)
            print(f"  UAT pass for provider={provider_name}: OK")
            return 0
        except subprocess.CalledProcessError:
            print(f"  UAT pass for provider={provider_name}: FAILED")
            return 1

    if provider == "all":
        # Run once per provider whose API key is actually present.
        # Decrypt the bundle once to know what's available; if sops
        # isn't set up, fall back to inspecting the shell env.
        available = []
        if secrets_file.exists():
            try:
                out = subprocess.run(
                    ["sops", "-d", str(secrets_file)],
                    capture_output=True,
                    text=True,
                    check=True,
                )
                for p, var in provider_env_keys.items():
                    if f"{var}:" in out.stdout:
                        available.append(p)
            except subprocess.CalledProcessError as e:
                print(f"  Could not decrypt secrets bundle: {e}")
                return 1
        else:
            for p, var in provider_env_keys.items():
                if os.environ.get(var):
                    available.append(p)
        if not available:
            print("  No provider credentials available — nothing to run.")
            return 1
        print(f"  Providers to exercise: {', '.join(available)}")
        rc = 0
        for p in available:
            rc |= run_once(p)
        if rc == 0:
            print("\nAll UAT passes succeeded.")
        else:
            print("\nOne or more UAT passes failed.")
        return rc

    if provider not in provider_env_keys:
        print(
            f"  Unknown --provider '{provider}'. "
            f"Expected one of: {', '.join(provider_env_keys)}, all"
        )
        return 1
    return run_once(provider)


@test()
@angreal.command(
    name="secrets-edit",
    about="Open the UAT secrets bundle in $EDITOR via sops",
    tool=angreal.ToolDescription(
        """
        Edit `tests/secrets/uat.enc.yaml` in place via sops.

        For brand-new bundles (file doesn't exist yet) this writes a
        one-line stub, encrypts it in place to all current recipients
        in `.sops.yaml`, and then opens the encrypted file for editing.
        For existing bundles it just runs `sops edit` directly.

        Requires `SOPS_AGE_KEY_FILE` to point at your AGE private key.
        See `tests/secrets/README.md` for onboarding.
        """,
        risk_level="safe",
    ),
)
@angreal.argument(
    name="file",
    long="file",
    help="Encrypted file under tests/secrets/ (default: uat.enc.yaml)",
    default="uat.enc.yaml",
)
def test_secrets_edit(file=None):
    """Edit a sops-encrypted secrets bundle in place."""
    # angreal passes None when --file is omitted (the decorator
    # `default=` doesn't get threaded into the call), so honor the
    # documented default here rather than at the def-site signature.
    if file is None:
        file = "uat.enc.yaml"
    repo_root = _repo_root()
    target = repo_root / "tests" / "secrets" / file
    rel = target.relative_to(repo_root)

    # sops 3.10+ `edit` only operates on existing encrypted files. For
    # a new bundle: write a one-line placeholder at the target path
    # (which matches .sops.yaml's path_regex so the creation_rule
    # resolves), then encrypt-in-place. After that the file has sops
    # metadata and `edit` works for all subsequent updates.
    if not target.exists():
        print(f"  Creating new encrypted bundle at {rel}")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text('OLLAMA_API_KEY: "REPLACE_ME"\n')
        try:
            subprocess.run(
                ["sops", "--encrypt", "--in-place", str(target)],
                check=True,
            )
        except subprocess.CalledProcessError as e:
            # If encryption failed, scrub the plaintext placeholder so
            # we don't leave it lying around.
            try:
                target.unlink()
            except OSError:
                pass
            print(f"  sops encrypt failed; placeholder removed. Error: {e}")
            return 1

    try:
        subprocess.run(["sops", "edit", str(target)], check=True)
        return 0
    except subprocess.CalledProcessError as e:
        print(f"  sops edit failed: {e}")
        return 1
    except FileNotFoundError:
        print(
            "  `sops` not found on PATH. Install it: `brew install sops age` "
            "(macOS) or see tests/secrets/README.md."
        )
        return 1
