# With Nix

From a checkout with flakes enabled:

```sh
nix build
./result/bin/palace --version
nix flake check
nix develop
nix fmt -- --check flake.nix
```

The default package installs `palace` and `palace-mcp`. The MCP server uses
stdio with its required `--kg` or `--palace` argument; its current CLI does not
implement `--help` (it exits 1 silently). The lock pins nixpkgs
26.05, Crane, rust-overlay and the RustSec advisory database. Rust 1.97.1 is
pinned in both `flake.nix` and `rust-toolchain.toml`. The development shell
includes clippy, rustfmt, rust-analyzer, rust-src, jq, sqlite, uv, gitleaks,
cargo-deny and Python 3. Nix does not install or configure MemPalace, hooks or
scheduled jobs; the existing installer and release workflow remain separate.

`nix flake check` runs workspace tests, clippy with `-D warnings`, rustfmt and
cargo-deny. Cargo dependencies are vendored; deny uses the pinned advisory
snapshot offline. That snapshot is given local Git metadata dated to its input
revision so cargo-deny can check database age. Registry yank information is not
available from the vendored sources; deny can warn about unavailable index
entries. Refresh the advisory input regularly.

Apple silicon macOS is built and tested. Intel macOS and both Linux architectures
are evaluated only; building them requires an appropriate builder.

## Test exclusions

The source filter includes Cargo files, `.cargo/`, `crates/` and embedded
`prompts/`. Private `fixtures/`, `expected/` and `crates/*/tests/fixture_*` are
excluded before building. Tests use `--no-default-features` to disable private
fixture-backed doctors (and owner defaults). No Rust assertions are changed.
The fixture tests depend on private oracle data; replacing it would change what
they validate. The test derivation additionally copies only `scripts/mempalace-check.sh` into
its source, allowing all pin tests to run without including other checkout scripts.
Python is provided for the synthetic model and MemPalace stand-ins; no real model
or MemPalace is invoked.

| Excluded test | Reason |
|---|---|
| `the_real_rollups_parse_as_before_and_keep_their_no_compress_entries` | Reads private `fixtures/diary`. |
| `the_assistants_oracle` | Embeds `expected/m5c/classify.json`. |
| `the_improve_fixture_pairs_and_parses` | Embeds private improve and rollup fixtures. |
| `expected_files_equal_the_oracle` | Re-derives the private graph/query oracle and reads `expected/m5c`. |
| `the_port_matches_the_oracle` | Reads the private expected graph/match files. |
| `the_graph_block_carries_the_hooks_lines` | Copies the private graph and reads expected relation lines. |
| `missing_files_print_nothing_unless_strict` | Its setup copies the private graph. |
| `a_held_database_exits_three_naming_it` | Copies the private graph database. |
| `a_held_kg_is_a_retryable_error` | Copies the private graph database. |

The fixture modules also contain already-ignored oracle-generation tests; those
modules are absent from the Nix source. In a private checkout, run the same test
selection in the development shell with:

```sh
nix develop -c cargo test --workspace --locked --no-default-features --jobs 8 -- \
  --skip the_real_rollups_parse_as_before_and_keep_their_no_compress_entries \
  --skip the_assistants_oracle \
  --skip the_improve_fixture_pairs_and_parses \
  --skip expected_files_equal_the_oracle \
  --skip the_port_matches_the_oracle \
  --skip the_graph_block_carries_the_hooks_lines \
  --skip missing_files_print_nothing_unless_strict \
  --skip a_held_database_exits_three_naming_it \
  --skip a_held_kg_is_a_retryable_error
```

## Updating and verifying inputs

`nix flake update` refreshes all inputs; `nix flake update advisory-db` refreshes
only advisories. Review and commit `flake.lock`, then rerun build and checks.
For a toolchain update, change the exact version in both toolchain files and
refresh rust-overlay if needed. Check all platform evaluations with
`nix flake show --all-systems` and
`nix eval .#packages.<system>.default.drvPath`.

To check reproducibility locally, run `nix build` twice, record `readlink result`,
then run `nix build --rebuild`. A successful rebuild means Nix found no differing
output; a cached second build alone does not establish reproducibility.
