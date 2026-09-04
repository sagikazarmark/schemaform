# schemaform-dioxus

[![crates.io](https://img.shields.io/crates/v/schemaform-dioxus?style=flat-square)](https://crates.io/crates/schemaform-dioxus)
[![docs.rs](https://img.shields.io/docsrs/schemaform-dioxus?style=flat-square)](https://docs.rs/schemaform-dioxus)

**Browser-CSR [Dioxus](https://dioxuslabs.com) adapter for runtime JSON Schema
forms.**

`schemaform-dioxus` renders a [`schemaform`](../schemaform/README.md)
`FormDefinition` as accessible, unstyled semantic HTML in a Dioxus browser
client-side-rendered application. It keeps Dioxus state out of the core engine
and provides explicit control renderer, structure renderer, finding presenter,
localization, and extension seams.

## Install

Both crates are published on crates.io and share one version. Add the same
version of each:

```toml
[dependencies]
schemaform = "0.1"
schemaform-dioxus = "0.1"
```

See [CHANGELOG.md](../../CHANGELOG.md) for what changed in each release,
including the migration note for custom renderer authors in the unreleased
0.2 section.

## Quick Start

```rust,no_run
use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{SchemaForm, RenderConfiguration, use_form};
use serde_json::json;

#[component]
fn App() -> Element {
    let definition = use_hook(|| FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Your name" }
        }
    })).expect("the trusted data schema should compile"));
    let form = use_form(definition, json!({ "name": "Ada" }))
        .expect("the form should be created");
    let form_to_bind = form.clone();
    let bound = use_hook(move || RenderConfiguration::default()
        .bind(&form_to_bind)
        .expect("the built-in renderer should bind"));

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                println!("{}", snapshot.form_data());
            },
            on_error: move |error| eprintln!("form operation failed: {error}"),
        }
    }
}
# fn main() {}
```

`use_form` constructs one browser-local `FormHandle`.
`RenderConfiguration::bind` performs definition-stable renderer and extension
preflight before mounting. `SchemaForm` calls `on_submit` only for a ready
snapshot; blocked submission updates finding presentation and focus instead.
Adapter operation failures are reported through the optional `on_error`
callback and are dropped when it is not set.

Handle operations are fallible because host callbacks can re-enter a handle.
Borrow conflicts return `HandleError::BorrowConflict` or
`HandleTransactionError` rather than panicking. Core transaction closure and
commit failures remain available through the transaction error variant. A
handle retained after the Dioxus scope that created it is unmounted returns
`HandleError::Disposed` before reading or mutating core state; render preflight
reports the corresponding `BindFinding::Disposed`.

## Customization And Localization

A custom `ControlRenderer` owns the entire control region: label, widget, help
text, and local findings. The adapter renders exactly what the renderer
returns and contributes nothing after it. The `ControlRenderContext` hands the
renderer everything it needs, pre-localized:

- `presentation()` is the node presentation: the `element_id` the primary
  element must carry, the localized `label` and whether it is `label_visible`,
  optional `help` with the element id it must carry, local `findings` as
  descriptors with stable ids, `invalid`, and `presence`: the presence
  affordances the core allows right now. `described_by()` joins the help and
  finding ids for `aria-describedby`; `present_help()` renders the help as the
  built-in does; `present_findings()` renders the findings through the
  configured local finding presenter so presenter swaps keep working without
  re-calling the renderer.
- Each `Affordance` in `presence` is a pre-localized, pre-authorized action:
  its `kind` (`Set`, `SetNull`, `RemoveValue`, `Replace`), localized `label`,
  the `id` the triggering element must carry, an optional `accessible_name` to
  use as `aria-label` when it is `Some`, and `invoke`, a callback that
  performs the core operation and reports failures to `on_error` itself. The
  list holds exactly the operations the built-in would offer: set only while
  the value is missing or null and a creation seed exists, replace only while
  the core allows replacement and a seed exists, set null and remove value
  whenever the core allows them. Renderers place affordances; they do not
  reconstruct the rules.
- `control()` is the control facets: `kind` (`String`, `Number`, `Integer`,
  `Boolean`, `Choice`, `Constant`), the control binding as `name`, `required`,
  `disabled`, `read_only`, `write_only`, `touched`, `dirty`, `nullable`, and the
  localized write-only replacement label and placeholder, write-only status
  text, and boolean value labels the built-in uses.
- `node()` is a target-scoped reactive reader, `actions()` the approved scalar
  actions for that node, and `extensions()` the prepared extension decorators.
- `report(result)` routes a failed `actions()` call to `SchemaForm::on_error`
  and returns the success value as `Option`, so a renderer never has to drop a
  `HandleError` such as a borrow conflict or a rejected write.

Every id referenced by the presentation names an element the renderer is
responsible for emitting; finding-summary focus and label association rely on
the primary element carrying `element_id`. The context is `PartialEq`, so it
can be passed as a prop to a child component.

Custom renderers do not receive the complete form handle or unrestricted core
mutation authority. Collection actions are obtainable through the node reader,
and the core rejects them for nodes that are not arrays. Custom renderers,
presenters, localizers, and extension handlers are trusted host code and may
capture authority independently.

Homogeneous array composition remains adapter-owned. Array-level exact widget
requests are preserved by stable UI-schema v1 parsing and compilation but fail
render binding with `BindFinding::UnsupportedCollectionWidget`, even if
registered. Matchers are not evaluated for array nodes. Inline item templates
are still preflighted in full, and eligible controls within them retain exact
or matcher-selected custom renderers and prepared extensions.

`ControlRenderContext` does not expose raw data schemas. Control-specific
authored configuration may be interpreted by an extension handler while it
prepares its decorator, but is not exposed as an open-ended renderer-options
object.

An exact-widget renderer renders its whole region from the node-scoped context,
places the presence affordances the adapter computed, and reports operation
failures. Render-time reads are the one place a renderer legitimately falls back
instead of reporting: a node that cannot be read is about to be unmounted, so
the renderer renders nothing rather than raising an error on every frame. The
example below wires `oninput` directly to `actions().input_text`; a text
control that should behave exactly like the built-in, including IME
composition, uses `use_text_edit` from the [Headless edit hooks](#headless-edit-hooks)
section instead.

```rust
use std::sync::Arc;
use dioxus::prelude::{Element, rsx};
use schemaform::WidgetSymbol;
use schemaform_dioxus::{
    ControlRegistry, ControlRenderContext, ControlRenderer,
};

struct TextRenderer;

impl ControlRenderer for TextRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        // A node that cannot be read right now is unavailable; render nothing for it.
        let Some(projection) = context.node().read().ok().flatten() else {
            return rsx! {};
        };
        let actions = context.actions().clone();
        let reporter = context.clone();
        let presentation = context.presentation();
        let control = context.control();
        let presence = presentation.presence.clone();
        rsx! {
            label { r#for: presentation.element_id.clone(), "{presentation.label}" }
            input {
                id: presentation.element_id.clone(),
                name: control.name.clone(),
                value: projection.value.unwrap_or_default(),
                required: control.required,
                disabled: control.disabled,
                readonly: control.read_only,
                "aria-invalid": presentation.invalid,
                "aria-describedby": presentation.described_by(),
                oninput: move |event| {
                    reporter.report(actions.input_text(event.value()));
                }
            }
            if let Some(help) = &presentation.help {
                p { id: help.id.clone(), "{help.text}" }
            }
            {presentation.present_findings()}
            for affordance in presence {
                button {
                    key: "{affordance.id}",
                    id: affordance.id.clone(),
                    r#type: "button",
                    onclick: move |_| affordance.invoke.call(()),
                    "{affordance.label}"
                }
            }
        }
    }
}

let controls = ControlRegistry::with_builtins().widget(
    WidgetSymbol::parse("company:text")?,
    Arc::new(TextRenderer),
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

A `render::BoundForm` is a single-mount render plan. Its clones share DOM
identity and must not be mounted concurrently. Bind the `FormHandle` separately
for each concurrent view.

### Headless edit hooks

The hard parts of a text control are not in the widget: buffering an IME
composition until it ends, discarding a half-typed composition when the form is
reset or reinitialized, and putting the canonical text back into the DOM after
the core rejects a keystroke. `use_text_edit` owns all of that so a custom
renderer only places a widget. Call it inside the renderer's own child
component with the `ControlRenderContext` it received; `ControlRenderer::render`
itself is not a hook-safe call site. The built-in string, number, and integer
controls render through the same hook and the same public context.

`use_text_edit(&context)` returns a `TextEdit`:

- `value: ReadSignal<String>` is the text the widget should show right now: the
  composition buffer while composing, else the edit buffer, else the canonical
  text, and empty for a write-only control without an edit buffer. It is
  derived through a memo that subscribes to the node, so the first render after
  a transition already shows the new text without the component itself reading
  the node.
- `input: Callback<String>` applies the widget's text. While composing it
  buffers locally and runs no core operation; otherwise it calls
  `ControlActions::input_text`, reports a failure to `on_error`, and
  resynchronises the widget's DOM value to the canonical text.
- `composition_start` and `composition_end` bracket an IME composition; a
  composition that began before a reset or reinitialization is discarded.
- `blur` finishes any composition and marks the control touched.
- `read_only` is true while the node is read-only or the core does not accept
  text input right now, matching `control().read_only` for text controls.

The callbacks keep their identity across renders and `value` is a signal, so a
widget that takes them as props does not re-render per keystroke.

```rust
use dioxus::prelude::*;
use schemaform_dioxus::{ControlRenderContext, ControlRenderer, use_text_edit};

struct PlainTextRenderer;

impl ControlRenderer for PlainTextRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        // Hooks belong in the renderer's own component, not in `render` itself.
        rsx! { PlainTextControl { context } }
    }
}

#[component]
fn PlainTextControl(context: ControlRenderContext) -> Element {
    let edit = use_text_edit(&context);
    let presentation = context.presentation();
    let control = context.control();
    rsx! {
        label { r#for: presentation.element_id.clone(), "{presentation.label}" }
        input {
            id: presentation.element_id.clone(),
            name: control.name.clone(),
            value: edit.value,
            readonly: edit.read_only,
            required: control.required,
            "aria-invalid": presentation.invalid,
            "aria-describedby": presentation.described_by(),
            oninput: move |event| edit.input.call(event.value()),
            oncompositionstart: move |_| edit.composition_start.call(()),
            oncompositionend: move |_| edit.composition_end.call(()),
            onblur: move |_| edit.blur.call(()),
        }
        {presentation.present_help()}
        {presentation.present_findings()}
    }
}
```

Boolean and choice controls have their own hooks with the same shape: hook-stable
callbacks plus a read signal derived through a memo over the node.

`use_boolean_edit(&context)` returns a `BooleanEdit`:

- `checked: ReadSignal<Option<bool>>` is the tri-state to display: `Some(true)`
  or `Some(false)` while the current data is a JSON boolean and `None` while it
  is null. A missing or incompatible value reads as `Some(false)`, as the
  built-in checkbox shows it unchecked; a write-only control always reads as
  `None` and never echoes its value.
- `set: Callback<Option<bool>>` applies the widget's state. `None` sets null;
  `Some` reads the operations the core allows at event time and replaces the
  value when replacement is allowed (incompatible data, or a write-only
  control), otherwise sets it. A failure is reported to `on_error` and the
  widget carrying the element id is resynchronised to `checked` (a checkbox's
  `checked` property, or a `select`'s `value` as `"true"`, `"false"`, or `""`);
  a write-only control is resynchronised after every call.
- `blur` marks the control touched.

`use_choice_edit(&context)` returns a `ChoiceEdit`:

- `selected: ReadSignal<Option<ChoiceIdentity>>` is the option to show as
  selected, `None` while no option matches the current data and always for a
  write-only control.
- `options: Vec<ChoiceOption>` lists the options in the core's compiled order
  (the null option first), each with an opaque `identity`, a `label` localized
  through the configured `Localizer`, `is_null`, and `disabled`, which is true
  when selecting the option right now would be rejected by the core (the null
  option while set null is not allowed; another option while neither set nor
  replace is allowed). The current option is never disabled.
- `select: Callback<Option<ChoiceIdentity>>` applies a selection. The null
  option sets null; another option sets or replaces the value as `set` does
  above; reselecting the current option, `None`, and an unknown identity run no
  core operation. Whenever no operation changed the value, and after every call
  for a write-only control, the widget's `value` property is restored to the
  selected identity (or `""`).
- `blur` marks the control touched.

A widget maps its DOM value back to an identity by looking it up in `options`
with `ChoiceIdentity::as_str`. Constant controls have no hook: render read-only
output from `presentation()` and `control()`.

The built-in scalar control is itself a `ControlRenderer` built on these hooks
and the public context. `ControlRegistry::with_builtins()` registers
`BuiltinControlRenderer` at `render::BUILTIN_CONTROL_PRIORITY` with a matcher
for every supported semantic kind, so renderer resolution has no built-in
special case: the highest matching priority wins, a tie is
`BindFinding::AmbiguousMatcher`, and a registry created with
`ControlRegistry::empty()` reports `BindFinding::NoMatchingRenderer` for any
control no registration accepts. A host can also register
`BuiltinControlRenderer` under an exact widget symbol or at another priority.

### Structure renderers

Non-control presentation goes through one small trait per structural node kind,
composed in a `StructureRenderers` bundle whose unset slots are the built-ins.
A package implements only the traits it changes and exports a populated bundle;
a host composes slots from several packages with the `with_*` setters and
installs the result with `RenderConfigurationBuilder::structure`. There is no
supertrait over the slots, so a new slot in a later release is additive for
every existing implementation.

Structure renderers are fixed when the form is bound. Unlike presenters and the
localizer they are not signal-swappable: their output is the parent template of
every node, so swapping one would remount every child scope.
`RenderConfiguration::rebind_presentation` leaves them alone; changing a
structure renderer means calling `RenderConfiguration::bind` again.

The first slot is the **form shell**. `ShellRenderer::shell` receives a
`ShellContext` and returns the *contents* of the `<form>` element:

- `form_id` is the id of the adapter-owned `<form>`, which keeps `novalidate`,
  the submit handling, `tabindex="-1"`, and the error-handler context.
- `summary` is the finding summary inside its adapter-owned wrapper
  (`{form_id}-summary`, `role="region"`, a localized `aria-label`,
  `tabindex="-1"`). A blocked submission focuses it. It must be placed.
- `body` is every root-level node in definition order, pre-keyed. It must be
  placed.
- `submit` is an `Affordance` of kind `Submit` with the localized submit label
  and the id `{form_id}-submit`. `invoke` finalizes edit buffers and prepares
  submission: a ready snapshot reaches `on_submit`, a blocked outcome focuses
  the summary, and an adapter failure reaches `on_error`. Place it either as a
  `type="submit"` button, which submits through the form element, or as any
  element that calls `invoke`; not both on one element.

`BuiltinShell` is the public built-in: summary, body, then a `type="submit"`
button carrying the affordance's id and label.

```rust
use dioxus::prelude::*;
use schemaform_dioxus::{
    RenderConfiguration, ShellContext, ShellRenderer, StructureRenderers,
};

struct CardShell;

impl ShellRenderer for CardShell {
    fn shell(&self, context: ShellContext) -> Element {
        let submit = context.submit;
        rsx! {
            div { class: "card-body", {context.body} }
            div { class: "card-alerts", {context.summary} }
            div { class: "card-actions",
                button {
                    id: submit.id.clone(),
                    class: "btn btn-primary",
                    r#type: "button",
                    onclick: move |_| submit.invoke.call(()),
                    "{submit.label}"
                }
            }
        }
    }
}

let configuration = RenderConfiguration::builder()
    .structure(StructureRenderers::default().with_shell(CardShell))
    .build();
```

`ShellRenderer::shell` runs during rendering and is not a hook-safe call site;
a shell that needs hooks renders a child component and passes the context as
props (`ShellContext` is `PartialEq`).

`Localizer` receives a `render::MessageDescriptor` with an optional stable key,
an English fallback, and structured parameters. Authored UI-schema text, schema
labels and help, findings, submit and presence controls, array actions and live
announcements, and write-only replacement chrome all pass through this
interface. Text is rendered as escaped plain text.

## Errors And Platform Boundary

| Stage | Public result |
| --- | --- |
| Form construction | `schemaform::FormBuildError` |
| Render preflight | `render::BindError` with structured `render::BindFinding` values |
| Handle and control operations | `HandleError`; a custom renderer routes it to `on_error` with `ControlRenderContext::report` |
| Host transactions | `HandleTransactionError` |
| Submission | `SchemaForm::on_submit` for a ready snapshot; optional `SchemaForm::on_error` for adapter failures |

This package supports browser CSR. SSR, hydration, desktop/WebView execution,
transport, authentication, retries, and pending/success lifecycle are outside
its scope. It inherits the core trust boundary: data schemas must be
application-trusted for evaluator work, referenced resources are supplied in
memory, and structural limits are not hostile-schema execution containment. It
also inherits the core capability boundary: nullable support is limited to
scalar controls, while nullable fixed objects and arrays are capability-blocking.

## Feature Flags

The crate has no public Cargo features. Repository qualification hooks are
enabled through a `--cfg` and cannot be activated through dependency feature
unification or `--all-features`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](../../LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
