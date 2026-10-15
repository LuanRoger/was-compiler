# Workspace validation and Bacon

Run commands from the repository root. Rust dependencies must be cached; use
`--offline`. Native compiler/CLI checks are the workspace defaults. Browser tests
are owned by `was-compiler-browser` and require explicit package selection or
`--workspace`.

## Editor analysis

The shared Rust subprocess helper lives at `tests/support/process.rs`, outside the
three Cargo package directories. Zed's [workspace settings](../.zed/settings.json)
add root `tests/` to rust-analyzer's `vfs.extraIncludes` so valid cross-package
`#[path]` modules are loaded by the editor too. Keep this inclusion when changing
workspace/editor configuration; Cargo can pass while rust-analyzer reports
`unresolved module` if that directory is absent from its virtual filesystem.

For this editor-only boundary, verify diagnostics in compiler unit/integration
and CLI/browser integration targets as well as running Cargo checks. Cargo alone
does not reproduce missing VFS files. The seven reported unresolved-module
diagnostics were reproduced before the inclusion and cleared after it, without
changing the Rust module paths or suppressing diagnostics.

## Interactive jobs

The root [bacon.toml](../bacon.toml) is the development interface. Bacon 3.22.0 was
used to verify this configuration. It is a developer tool, not a compiler
dependency; install prerequisites separately rather than having validation fetch
them. The configured jobs pass offline flags, and `CARGO_NET_OFFLINE=true` also
covers Cargo jobs inherited from Bacon defaults.

```text
bacon
bacon test
bacon clippy
bacon fast
bacon full
bacon bench
bacon bench-smoke
```

`check` is the default. Ordinary check/test/Clippy jobs watch workspace sources,
shared test assets, runtime helpers, and examples. Generated `pkg/` and `target/`
are excluded. Browser jobs, runtime execution, benchmarks, and validation chains
are manual: source-change reruns are disabled. Select the chain's **entry job**
again to restart validation, not whichever final job is currently displayed.
Bacon still watches configuration and retains its UI after a manual job finishes;
`--headless` is not a one-shot mode either. Stop Bacon yourself when done.

Chains use success actions with no cycles; errors, warnings, or failures stop
progression. Separate chain nodes keep standalone jobs standalone. Running an
individual browser-test job assumes `pkg/` is fresh; use `full` to enforce rebuild
ordering. Editing sources while a manual chain is running invalidates its release
evidence: restart from its entry job on stable sources.

## Finite direct commands

These are authoritative for bounded agent execution and release evidence. A
successful last job does not prove its predecessors ran. No shell wrapper is
needed. Every configured job maps to a command below; `fast-*` and `full-*` nodes
use the corresponding commands in the listed order.

| Standalone job | Direct command |
| --- | --- |
| `check` | `cargo check --offline --all-targets` |
| `test` | `cargo test --offline` |
| `clippy` | `cargo clippy --offline --all-targets -- -D warnings` |
| `core-wasm` | `cargo build --offline -p was-compiler --lib --target wasm32-unknown-unknown` |
| `architecture` | `cargo test --offline -p was-compiler --test workspace_contract` |
| `browser-build` | `wasm-pack build crates/browser --target web --out-dir ../../pkg --out-name was_compiler --mode no-install --no-opt --offline` |
| `browser-test` | `cargo test --offline -p was-compiler-browser --tests -- --include-ignored` |
| `runtime` | `cargo test --offline -- --include-ignored` |
| `browser-clippy` | `cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings` |
| `bench` | `cargo bench --offline -p was-compiler --bench compiler` |
| `bench-smoke` | `cargo bench --offline -p was-compiler --bench compiler -- --test` |

### Fast chain

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo build --offline -p was-compiler --lib --target wasm32-unknown-unknown
cargo test --offline -p was-compiler --test workspace_contract
```

The native library build is included in tests/Clippy. Architecture checks also
run in the default test suite; their separate final step makes dependency-isolation
and downstream-consumer evidence easy to locate. The downstream fixture is a
standalone Cargo workspace with its own committed lockfile; its subprocess uses
`target/downstream-contract` to avoid the parent Cargo target lock.

### Full chain

Run the fast sequence above, then:

```sh
wasm-pack build crates/browser --target web --out-dir ../../pkg --out-name was_compiler --mode no-install --no-opt --offline
cargo test --offline -p was-compiler-browser --tests -- --include-ignored
cargo test --offline -- --include-ignored
cargo clippy --offline -p was-compiler-browser --target wasm32-unknown-unknown --lib -- -D warnings
```

Prerequisites: Node.js 24+, `wasm32-unknown-unknown`, wasm-pack, compatible installed
wasm-bindgen tooling, and cached Rust dependencies. `--mode no-install` prevents
tooling installation; `--no-opt` skips external wasm-opt. The browser test target
runs `node tests/browser.cjs pkg` through the existing bounded subprocess helper,
plus the feature-specific native/browser parity suites. The next default-member
run covers all native ignored runtime cases. Alternatively, after the browser
rebuild, `cargo test --offline --workspace -- --include-ignored` covers all three
packages together. Ordinary `cargo test --offline` skips ignored cases and does
not select the browser package.

Benchmarks remain separate from correctness checks; see [measurement boundaries
and baseline](benchmarks.md). There is no hosted CI or performance-threshold gate.

For modified Rust files, use `rustfmt --edition 2024 --config skip_children=true`
with explicit paths, then `git --no-pager diff --check`. Review status/diff; preserve
existing fixtures and `target/wp7-inference`. Report missing prerequisites and
unverified gates rather than fetching tools or claiming partial runs passed.
