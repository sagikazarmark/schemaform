# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
`schemaform` and `schemaform-dioxus` share one version.

## [Unreleased]

### Changed

- The minimum supported Rust version is now 1.92, declared as `rust-version` in
  the root workspace, `fuzz`, and `demo` manifests. The previous claim of 1.85
  was already false: the crates use let-chains, stable since Rust 1.88. 1.92 is
  the toolchain the upcoming renderer work requires through `dioxus-field`, and
  one number is easier to keep honest than two. CI now compiles every workspace
  target on a 1.92 toolchain (`schemaform:msrv`), so the declared version is
  enforced rather than asserted.
- The root and demo workspaces pin Dioxus `=0.7.10` for the umbrella,
  `dioxus-html`, and `dioxus-web` crates, matching the `dioxus-core` and
  `dioxus-signals` pins that had already moved to 0.7.10. Both lockfiles are
  regenerated; the demo lockfile additionally catches up with the adapter's
  current dependency requirements.

## [0.1.0] - 2026-08-03

Initial release of `schemaform` and `schemaform-dioxus`.

[Unreleased]: https://github.com/sagikazarmark/schemaform/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/sagikazarmark/schemaform/releases/tag/v0.1.0
