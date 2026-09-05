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
  (`None` for null and for a write-only control, `Some(false)` for missing or
  incompatible data as the built-in checkbox shows it), a
  `set: Callback<Option<bool>>` that sets null for `None` and, for
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
  for a node right now, and `Affordance` (`kind`, localized `label`, the DOM
  `id` the triggering element must carry, optional `accessible_name`, `invoke`)
  performs the operation and reports failures to `SchemaForm::on_error` itself.
  For a scalar control, set is offered only while the value is missing or null
  and a creation seed exists, replace only while the core allows replacement
  and a seed exists, set null and remove value whenever the core allows them;
  for a homogeneous array, materialize, replace, and remove value follow the
  same seed rules and additionally announce and focus the array when invoked.
  `AffordanceKind` is non-exhaustive (`Set`, `SetNull`, `RemoveValue`,
  `Replace`, `Materialize`, `Append`, `InsertBefore`, `MoveUp`, `MoveDown`,
  `RemoveItem`, and `Submit`). `accessible_name` is `Some` only when the
  accessible name must differ from the visible label: the four item affordances
  carry the positional variant (`Insert Tags item before position 2`), every
  other affordance carries `None`. The built-in control and array render their
  buttons from the same lists, so a custom renderer receives exactly the
  operations the built-in would offer; the built-in presence buttons now carry
  the affordance id, including the array's container presence buttons
  (`{element_id}-materialize`, `-replace-value`, `-remove-value`), which had
  none.
- `NodePresentation::incompatible_value` is the serialized current value while
  the node cannot edit it but the core allows replacement, as the built-in shows
  beside its replace button: `Some` for a scalar control whose value is
  incompatible (or null where null is not accepted) while text input is
  rejected, and for a container whose value is replaceable, unless the node is
  write-only. The built-in controls and array read it from the presentation.
- `ShellRenderer`, the first structure renderer seam, lets a host replace the
  form shell: where the finding summary sits, how the body is framed, and what
  triggers submission. `shell(ShellContext { form_id, summary, body, submit })`
  returns the *contents* of the `<form>` element; the adapter keeps the form
  element itself (`novalidate`, submit handling, `tabindex`, the error-handler
  context), the finding-summary region wrapper with its id, role, label, and
  focusability, and the submission rules. `summary` and `body` arrive as
  pre-keyed elements and must be placed; `submit` is an
  `AffordanceKind::Submit` affordance with the localized submit label and the
  id `{form_id}-submit` whose `invoke` finalizes edit buffers, calls
  `on_submit` for a ready snapshot, focuses the summary for a blocked outcome,
  and reports adapter failures to `on_error`. A shell may place it as a
  `type="submit"` button or as any element that calls `invoke`; a blocked
  submit still focuses the summary and a ready submit still yields a
  submission snapshot either way. `BuiltinShell` is the public built-in
  (summary, body, then a `type="submit"` button carrying the affordance id).
- `CollectionRenderer` lets a host replace the chrome of a homogeneous array
  and of its items (rows, add/insert/remove/move buttons, empty state) while the
  adapter keeps ownership of item identity, keying, focus after a mutation, and
  live-region announcements. `collection(CollectionContext { presentation,
  item_label, count, items, append, announcement, extensions })` renders the
  array: `presentation` carries the container presence affordances and
  `incompatible_value`, `item_label` is the localized singular item noun,
  `count` the number of items (the only way to render an empty state, since
  `items` is one opaque pre-keyed element), `append` the `Append` affordance
  (`{element_id}-append`) while appending is allowed, and `announcement` the
  adapter-owned live region, which must be placed. `collection_item(
  CollectionItemContext { row_id, position, count, item_label, children,
  insert_before, move_up, move_down, remove })` is called from an adapter-owned
  keyed per-item host (key = instance identity) inside a wrapper `div` carrying
  `row_id` and `data-array-item`; `children` is the instantiated item template
  and must be placed; each item affordance is `Some` exactly while the core
  allows it, `move_up`/`move_down` additionally `None` for the first/last item.
  Item affordance ids are the item root's id followed by `-insert-before`,
  `-move-up`, `-move-down`, `-remove`; the adapter reserves `row_id`, the item
  root's id, and the affordance ids, and renderers use `row_id` only as a prefix
  for their own ids. Invoking an affordance performs the operation, reports
  failures to `on_error`, announces, and moves focus. `collection` and
  `collection_item` are not called together (the collection re-renders after
  every announcement while item hosts memoize on their props), so a renderer
  that needs per-item state renders a child component; both contexts are
  `PartialEq`. `BuiltinCollection` is the public built-in and reproduces the
  previous array DOM, so every existing array behaviour is unchanged under it.
- `StructureRenderers` bundles one renderer per structural slot with private
  per-trait storage; `Default` is every built-in, `with_shell(impl
  ShellRenderer)` replaces the shell, and `with_collection(impl
  CollectionRenderer)` replaces the collection.
  `RenderConfigurationBuilder::structure` installs a bundle. There is
  deliberately no supertrait over the slots, so
  adding a slot later is additive for every existing renderer. Structure
  renderers are fixed at `RenderConfiguration::bind` and are not
  signal-swappable like presenters and the localizer:
  `rebind_presentation` leaves them alone, and changing a structure renderer
  means rebinding the form. The built-in submit button now carries the id
  `{form_id}-submit`.
- `ControlRenderContext::report(result)` routes a failed `ControlActions` call
  to `SchemaForm::on_error` and returns the success value as `Option`, so
  custom renderers no longer have to drop `HandleError` values.
- `ControlRenderContext`, `NodeReader`, `ControlActions`, `NodePresentation`,
  and `ControlFacets` implement `PartialEq` (value equality for presentation
  data and facets, identity equality for the reader, actions, and the bound
  form a presentation renders findings through, pointer equality for prepared
  extensions), so a context can be passed as a prop to a child component
  without Dioxus memoization showing stale state. `Affordance` compares by
  `kind`, `label`, `id`, and `accessible_name`; its `invoke` callback is
  hook-stable and excluded. `ShellContext`, `CollectionContext`, and
  `CollectionItemContext` are `PartialEq` for the same reason.
- `ControlKind` is public: the widget family the adapter derives from a
  definition node (`String`, `Number`, `Integer`, `Boolean`, `Choice`,
  `Constant`; non-exhaustive).
- `NodeProjection::nullable` reports whether the bound scalar accepts JSON
  null.
- `Affordance::present()` renders an affordance exactly as the built-ins do: a
  `button[type="button"]` carrying the affordance id, the built-in's `data-*`
  marker for its kind, `aria-label` from `accessible_name`, and `invoke` on
  `onclick`. The built-in control, array, and item render their buttons through
  it, so a custom renderer that wants the built-in's button no longer copies it.
- `NodeProjection::display_text()` is the text a control shows for a node it
  is not editing — the retained edit buffer or canonical spelling, else the
  current data spelled as JSON, and nothing for a write-only value without an
  edit buffer — so a custom constant or read-only renderer no longer restates
  the write-only rule.
- The crate root re-exports every public type of the `render` and `handle`
  modules a renderer author needs (`BUILTIN_CONTROL_PRIORITY`, `BooleanLabels`,
  `WriteOnlyReplacement`, `FindingCollectionContext`, `FindingDescriptor`,
  `FindingKind`, `FindingPresentation`, `TargetFocusAction`,
  `MessageDescriptor`, `RenderConfigurationBuilder`,
  `CollectionItemProjection`, `FindingProjection`, `FormProjection`); the
  module paths remain valid.

### Fixed

- `SchemaForm::on_error` is optional in the props builder, as its documentation
  has always said; omitting it drops adapter failures instead of failing to
  compile.
- Collection item hosts memoize on their props, as the `CollectionRenderer`
  contract promises. The adapter instantiated an item's bound subtree through
  tracked reads of every node in the item, so an edit inside any item
  re-rendered its host (and the finding summary, which walks the same subtrees)
  on every keystroke; the reads are now untracked, since an item's structure
  changes only with its position and count, which are already the host's
  props. The collection itself still re-renders when the array node changes,
  which the core reports whenever the array's data or findings change.
- A visible finding of a family the adapter does not project is left out of
  the summary projection instead of panicking inside a mutation. Debug builds
  still assert, so the lockstep release of the two crates catches a new core
  finding family in the adapter's tests.

### Changed

- `schemaform-dioxus` declares `wasm-bindgen`, `web-sys`, `js-sys`, and
  `wasm-bindgen-futures` as minimum requirements rather than exact pins, so
  the crate resolves alongside whatever version of that stack a consumer's
  other dependencies already lock. The versions this workspace builds and
  tests with are unchanged: they are the lockfile's, which CI holds with
  `--locked`. `jsonschema`, `referencing`, and `serde_json` stay exact, as one
  qualified validator release; a consumer whose lockfile already holds a newer
  `serde_json` needs `cargo update -p serde_json --precise 1.0.151` once.
- The renderer contract documents that `Affordance::invoke` is owned by the
  scope that computed it, so invoking an affordance retained past its node's
  removal panics inside Dioxus, and that the `Element` fields of
  `ShellContext`, `CollectionContext`, and `CollectionItemContext` compare by
  pointer: passing one of those contexts to a child component gives structure
  renderers a place for hooks, not memoization; only `ControlRenderContext`
  compares by value.
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
- The finding summary and the form body are separate adapter components so a
  shell can place them independently. The summary component alone subscribes
  to the summary projection; the body re-renders only when the bound form
  changes, whereas previously a summary change re-diffed the root node list as
  well. Each node's own subscription is unchanged.
- The homogeneous array renders through the collection seam: the adapter
  computes a `CollectionContext`, hosts each item in its own keyed scope with
  hook-stable affordance callbacks, and hands both to the configured
  `CollectionRenderer` (`BuiltinCollection` by default). Focus after an insert,
  append, or remove now targets the item's root element first (the item itself
  if focusable, else the first focusable element inside it) and only then falls
  back to the first focusable element inside the row wrapper. Under the
  built-in, which renders children before buttons, this lands where it always
  did; under a renderer that puts its buttons before the children it lands on
  the item's control rather than on the first button.

## [0.1.0] - 2026-08-03

Initial release of `schemaform` and `schemaform-dioxus`.

[Unreleased]: https://github.com/sagikazarmark/schemaform/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/sagikazarmark/schemaform/releases/tag/v0.1.0
