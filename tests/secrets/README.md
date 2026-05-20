# Test secrets

API keys for UAT and other test harnesses live in this directory as
sops-encrypted YAML. They are committed to git in ciphertext form;
each authorized developer can decrypt locally using their own AGE
private key. There is **no shared symmetric key** floating between
humans — onboarding is "you generate a keypair locally, you send me
your pubkey, I add it to the recipient list."

## One-time per developer setup

1. Install the tooling.

   macOS:
   ```sh
   brew install sops age
   ```

   Linux (Ubuntu/Debian):
   ```sh
   sudo apt install age
   # grab the latest sops release from https://github.com/getsops/sops/releases
   ```

2. Generate your AGE keypair.

   ```sh
   mkdir -p ~/.config/sops/age
   age-keygen -o ~/.config/sops/age/keys.txt
   ```

   The file contains both keys. Take the line that starts with
   `# public key: age1...` — that's the part you share.

3. Open a PR adding your pubkey to `.sops.yaml` at the repo root:

   ```yaml
   age:
     - age1existing0pubkey0from0maintainer  # existing recipient
     - age1your0new0pubkey0here              # you
   ```

   Get it reviewed + merged. The maintainer (or any current recipient)
   then runs `sops updatekeys tests/secrets/*.enc.yaml` and commits.

4. Once your pubkey is in the recipient list and the ciphertext has
   been refreshed, point sops at your private key and you're done:

   ```sh
   export SOPS_AGE_KEY_FILE=~/.config/sops/age/keys.txt
   ```

   Add that to your shell rc file so it persists.

## Editing a secret

```sh
sops edit tests/secrets/uat.enc.yaml
```

Or use the angreal wrapper which handles new-file creation for you:

```sh
angreal test secrets-edit
```

## Using secrets in tests

The `angreal test uat` task wraps the UAT test runner with
`sops exec-env tests/secrets/uat.enc.yaml`, which decrypts the bundle
and exposes every key as an environment variable to the wrapped
command. Tests read them like any other env var:

```rust
let api_key = std::env::var("OLLAMA_API_KEY").ok();
```

If `tests/secrets/uat.enc.yaml` doesn't exist or you haven't onboarded
sops yet, the task falls back to whatever's already in your shell
environment — fine for ad-hoc local runs:

```sh
OLLAMA_API_KEY=sk-... angreal test uat
```

## File format

`uat.enc.yaml` is a flat YAML map of `KEY: "value"`:

```yaml
OLLAMA_API_KEY: "sk-..."
GROQ_API_KEY: "gsk-..."
ANTHROPIC_API_KEY: "sk-ant-..."
```

The `encrypted_regex` in `.sops.yaml` matches `*_KEY` / `*_TOKEN` /
`*_SECRET` / `*_PASSWORD` — non-matching values (like metadata
comments) stay plaintext so diffs are readable.

## Bootstrapping the first file

This repo doesn't ship a pre-encrypted `uat.enc.yaml` because there's
no first recipient pubkey baked in. After your pubkey is in
`.sops.yaml`:

```sh
cd <repo root>
echo 'OLLAMA_API_KEY: "your-real-key"' > tests/secrets/uat.enc.yaml
sops --encrypt --in-place tests/secrets/uat.enc.yaml
git add tests/secrets/uat.enc.yaml
git commit -m "secrets: initial UAT key bundle"
```

Two things going on here:

1. **The file path matters for sops's creation_rule lookup.** sops
   resolves the rule from `.sops.yaml` against the *path you give it*.
   We write the plaintext directly at `tests/secrets/uat.enc.yaml`,
   which matches the `path_regex`, so the AGE recipient list resolves
   automatically. (Writing to `/tmp/foo.yaml` and piping doesn't
   match — you'd get `no matching creation rules found`.)

2. **`sops --encrypt --in-place` rewrites the file as ciphertext.**
   The plaintext you just wrote is replaced with sops's envelope
   format. Don't worry about cleaning up — the in-place op overwrites.

Common gotcha: **`sops edit` doesn't create files in sops 3.10+.** It
only opens already-encrypted files for editing. The recipe above is
the explicit create path. After the first run, all subsequent updates
use `sops edit tests/secrets/uat.enc.yaml` normally.

Or use the angreal wrapper which handles new-file creation for you:

```sh
angreal test secrets-edit
```

It detects a missing file, writes a stub, runs `sops --encrypt
--in-place`, and then drops into `sops edit` so you can replace the
placeholder with real keys.
