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
  | no presence affordances for custom controls | render one element per `context.presentation().presence` affordance, keeping its `id`, and call `affordance.invoke.call(())` |
  | `let _ = actions.input_text(..)` (dropped errors) | `context.report(actions.input_text(..))` |

### Added

- `use_text_edit(&ControlRenderContext) -> TextEdit`, the first headless edit
  hook. Called inside a custom renderer's own child component, it returns the
  built-in text-editing behaviour: `value` (a `ReadSignal<String>` that is the
  IME composition buffer while composing, else the edit buffer, else the
  canonical text, and empty for a write-only control without an edit buffer),
  `input`, `composition_start`, `composition_end`, `blur`, and `read_only`.
  Input while composing is buffered locally without a core operation; a form
  reset or reinitialization discards an in-flight composition; a rejected write
  is reported to `SchemaForm::on_error` and the widget's DOM value is restored
  to the canonical text. `value` is derived through a memo that subscribes to
  the node, and the callbacks are hook-stable, so a widget that takes them as
  props does not re-render per keystroke. The built-in string, number, and
  integer controls, including the write-only password and read-only output
  paths, now render through a child component built on this hook and the public
  render context, with unchanged DOM output.
- `use_boolean_edit(&ControlRenderContext) -> BooleanEdit` and
  `use_choice_edit(&ControlRenderContext) -> ChoiceEdit` complete the headless
  hook set. `BooleanEdit` carries a tri-state `checked: ReadSignal<Option<bool>>`
  (`None` for null, missing, or incompatible data, and always for a write-only
  control), a `set: Callback<Option<bool>>` that sets null for `None` and, for
  `Some`, reads the operations the core allows at event time to choose set
  value or replace value, and `blur`. `ChoiceEdit` carries
  `selected: ReadSignal<Option<ChoiceIdentity>>`, `options: Vec<ChoiceOption>`
  (opaque `identity`, `label` localized through the configured `Localizer`,
  `is_null`, and `disabled` when the core would reject selecting the option
  right now), `select: Callback<Option<ChoiceIdentity>>` (the null option sets
  null; another option sets or replaces at event time; reselection, `None`, and
  an unknown identity run no core operation), and `blur`. Both report failures
  to `SchemaForm::on_error` and resynchronise the widget carrying the element
  id after a rejected write and, for write-only controls, after every write.
- `BuiltinControlRenderer`, the built-in scalar control, is a public
  `ControlRenderer` built on the three hooks and the public render context, so
  a host can register it under an exact widget symbol or at another priority.
- `ControlRegistry::empty()` creates a registry without the built-in renderer,
  and binding reports the additive `BindFinding::NoMatchingRenderer
  { definition_node }` for every control no registration accepts.
- `NodePresentation::presence` lists the presence affordances the core allows
  for a scalar control right now, and `Affordance` (`kind`, localized `label`,
  the DOM `id` the triggering element must carry, `invoke`) performs the
  operation and reports failures to `SchemaForm::on_error` itself. Set is
  offered only while the value is missing or null and a creation seed exists,
  replace only while the core allows replacement and a seed exists, set null
  and remove value whenever the core allows them. `AffordanceKind` is
  non-exhaustive (`Set`, `SetNull`, `RemoveValue`, `Replace` today) so later
  renderer seams can add collection and submit affordances. The built-in
  control renders its presence buttons from the same list, so a custom
  renderer receives exactly the operations the built-in would offer; those
  buttons now carry the affordance id.
- `ControlRenderContext::report(result)` routes a failed `ControlActions` call
  to `SchemaForm::on_error` and returns the success value as `Option`, so
  custom renderers no longer have to drop `HandleError` values.
- `ControlRenderContext`, `NodeReader`, `ControlActions`, `NodePresentation`,
  and `ControlFacets` implement `PartialEq` (value equality for presentation
  data and facets, identity equality for the reader, actions, and the bound
  form a presentation renders findings through, pointer equality for prepared
  extensions), so a context can be passed as a prop to a child component
  without Dioxus memoization showing stale state. `Affordance` compares by
  `kind`, `label`, and `id`; its `invoke` callback is hook-stable and excluded.
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
- `ControlRegistry::with_builtins()` now registers `BuiltinControlRenderer` at
  `BUILTIN_CONTROL_PRIORITY` like any other matcher registration, and the
  adapter has one control render path: the host computes the render context and
  hands it to the preflight-selected renderer, built-in or custom. The built-in
  boolean (checkbox and write-only replacement select), choice, and constant
  controls render through child components built on `use_boolean_edit`,
  `use_choice_edit`, and the public context, with unchanged DOM output. Two
  behaviours follow from the hooks: the built-in checkbox and write-only
  boolean select decide between set value and replace value from the operations
  the core allows when the event fires rather than from a render-time snapshot,
  and choice option labels pass through the configured `Localizer` as keyless
  messages whose fallback is the authored label, so the default localizer
  renders them unchanged.
- The READMEs no longer describe the crates as unpublished and describe
  `SchemaForm::on_error` as the optional prop it is.

## [0.1.0] - 2026-08-03

Initial release of `schemaform` and `schemaform-dioxus`.

[Unreleased]: https://github.com/sagikazarmark/schemaform/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/sagikazarmark/schemaform/releases/tag/v0.1.0
