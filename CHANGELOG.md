# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
`schemaform` and `schemaform-dioxus` share one version.

## [Unreleased]

### Breaking

- A custom `ControlRenderer` now owns its whole control region. The adapter
  renders exactly what `ControlRenderer::render` returns: it no longer appends
  help text or local findings after a custom control and no longer composes an
  `aria-describedby` value on the renderer's behalf. A renderer that rendered
  only its widget under 0.1 now loses help and findings until it renders them
  itself.
- `render::Accessibility` and the `accessibility()`, `label()`,
  `is_label_visible()`, and `help()` accessors on `ControlRenderContext` are
  removed. Every field has a home in the two new value objects on the context:
  `presentation()` returns a `NodePresentation` (`element_id`, localized
  `label`, `label_visible`, optional `help` with its element id, `findings`
  with stable ids, `invalid`, `described_by()`, `present_help()`,
  `present_findings()`) and
  `control()` returns `ControlFacets` (`kind`, `name`, `required`, `disabled`,
  `read_only`, `write_only`, `touched`, `dirty`, `nullable`, localized
  write-only replacement label and placeholder, write-only status text, and
  boolean value labels).
- The `invalid` rule is now the same for every node kind: a node is invalid
  exactly when any of its local findings is blocking. Validation findings and
  parse blockers always block; capability and external findings block when the
  core marks them blocking. Previously controls, built-in and custom alike,
  ignored blocking capability findings; a control with a blocking capability
  finding now exposes `aria-invalid="true"`.

  Migration for custom renderer authors:

  | 0.1 | 0.2 |
  | --- | --- |
  | `context.accessibility().control_id` | `context.presentation().element_id` |
  | `context.accessibility().described_by.join(" ")` | `context.presentation().described_by()` (already joined; `None` when empty) |
  | `context.accessibility().invalid` | `context.presentation().invalid` |
  | `context.accessibility().required` / `.disabled` / `.read_only` | `context.control().required` / `.disabled` / `.read_only` |
  | `context.label()` / `context.is_label_visible()` | `context.presentation().label` / `.label_visible` |
  | `context.help()` | `context.presentation().help` (`Option<Help { id, text }>`) |
  | `projection.binding` as the rendered `name` | `context.control().name` |
  | adapter-rendered help | `context.presentation().present_help()` or render `help.text` in an element with `id: help.id` |
  | adapter-rendered findings | `context.presentation().present_findings()` or render `presentation().findings` yourself, keeping each `stable_id` |

### Added

- `ControlRenderContext`, `NodeReader`, `ControlActions`, `NodePresentation`,
  and `ControlFacets` implement `PartialEq` (value equality for presentation
  data and facets, identity equality for the reader, actions, and the bound
  form a presentation renders findings through, pointer equality for prepared
  extensions), so a context can be passed as a prop to a child component
  without Dioxus memoization showing stale state.
- `ControlKind` is public: the widget family the adapter derives from a
  definition node (`String`, `Number`, `Integer`, `Boolean`, `Choice`,
  `Constant`; non-exhaustive).
- `NodeProjection::nullable` reports whether the bound scalar accepts JSON
  null.

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
- The built-in control, fixed-object group, homogeneous array, and unsupported
  region compute their label, help, findings, `invalid`, and `aria-describedby`
  through one shared node-presentation helper instead of four near-identical
  computations.
- The READMEs no longer describe the crates as unpublished and describe
  `SchemaForm::on_error` as the optional prop it is.

## [0.1.0] - 2026-08-03

Initial release of `schemaform` and `schemaform-dioxus`.

[Unreleased]: https://github.com/sagikazarmark/schemaform/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/sagikazarmark/schemaform/releases/tag/v0.1.0
