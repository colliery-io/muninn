# muninn-narsil-vendor — upstream provenance

This crate contains a focused subset of source files vendored from
[narsil-mcp](https://github.com/postrv/narsil-mcp), pinned at upstream
commit:

    a4a2bd7a95f8ef9aab682bf1db97905d8f79b895

## Files vendored

| Local path                  | Upstream path             |
| --------------------------- | ------------------------- |
| `src/symbols.rs`            | `src/symbols.rs`          |
| `src/parser.rs`             | `src/parser.rs`           |
| `src/extract.rs`            | `src/extract.rs`          |
| `src/callgraph.rs`          | `src/callgraph.rs`        |
| `src/incremental.rs`        | `src/incremental.rs`      |

The files are copied substantially verbatim. Any local modifications
are kept minimal and called out in commit messages so future upstream
pulls remain mechanical.

## Why vendor rather than depend?

narsil-mcp is published as a single binary crate, not as a workspace
of consumable libraries. There is no `narsil-callgraph` or similar on
crates.io that we could `cargo add`. Vendoring the focused subset is
the only path to use this code as a library — and it lets us drop
narsil's heavyweight subsystems (RDF/SPARQL, MCP server, security
scanner, neural embeddings, LSP integration, frontend) that we don't
need.

## License

narsil-mcp is dual-licensed under MIT OR Apache-2.0. Both license
texts are included verbatim in this crate (`LICENSE-MIT` and
`LICENSE-APACHE`). muninn as a whole is Apache-2.0, which is
compatible with the dual-license terms.

## Updating

To pull a newer narsil revision:

1. `git -C /tmp/narsil-look fetch && git -C /tmp/narsil-look log <pin>..HEAD`
2. Review the diff for the five files listed above.
3. Copy updated files in; resolve any cascading API changes in
   `muninn-graph`'s adapter layer.
4. Update the pin commit hash in this file.
5. Commit with the new commit hash in the message.
