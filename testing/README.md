# Testing layout

The `testing/` tree contains test infrastructure and data shared across product
crates, browser targets, or release checks:

- `fixtures/business-schemas/` contains attributed, offline product-path data
  shared by the core and browser suites. It is not a mutation corpus.
- `browser/pack/` generates and verifies browser workloads and evidence.
- `browser/runner/` executes the real product path in WASM.
- `browser/empty-shell/` provides the artifact-size baseline.
- `browser/scripts/` contains the Node and Playwright drivers.
- `browser/workload-pack/` is generated, checked-in output. Treat its manifests
  and content-addressed fixtures as immutable release contracts.
- `toolchain/` keeps the Dioxus pin and `rust-version` identical across the
  root, `fuzz/` and `demo/` manifests, which cannot inherit them from one
  workspace.

Testing infrastructure that follows ecosystem conventions remains outside this
tree:

- `crates/schemaform-fuzz-harness/` is a workspace crate so native and browser
  tests can replay deterministic fuzz semantics without linking libFuzzer.
- `fuzz/` is the standalone `cargo-fuzz` package, with mutation targets, seed
  corpora, execution profiles, and retained regression evidence.

Generated output under `browser/artifacts/` and `../fuzz/artifacts/` is not
checked in.

The product crates keep ordinary unit and integration tests in Cargo's standard
locations.

Verify the checked-in browser contracts with:

```console
cargo run --locked -p browser-workload-pack -- check
```

Replay retained fuzz inputs and verify their evidence contract with:

```console
cargo test --locked -p schemaform-fuzz-harness
```

See `testing/browser/workload-pack/README.md` and
`testing/fixtures/business-schemas/README.md` for the subsystem-specific
contracts, and the repository [README](../README.md) for the CI checks and
manual release checks that consume them.
