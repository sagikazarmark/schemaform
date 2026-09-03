# schemaform-dioxus

[![crates.io](https://img.shields.io/crates/v/schemaform-dioxus?style=flat-square)](https://crates.io/crates/schemaform-dioxus)
[![docs.rs](https://img.shields.io/docsrs/schemaform-dioxus?style=flat-square)](https://docs.rs/schemaform-dioxus)

**Browser-CSR [Dioxus](https://dioxuslabs.com) adapter for runtime JSON Schema
forms.**

`schemaform-dioxus` renders a [`schemaform`](../schemaform/README.md)
`FormDefinition` as accessible, unstyled semantic HTML in a Dioxus browser
client-side-rendered application. It keeps Dioxus state out of the core engine
and provides explicit renderer, finding presenter, localization, and extension
seams.

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
  descriptors with stable ids, and `invalid`. `described_by()` joins the help
  and finding ids for `aria-describedby`; `present_help()` renders the help as
  the built-in does; `present_findings()` renders the findings through the
  configured local finding presenter so presenter swaps keep working without
  re-calling the renderer.
- `control()` is the control facets: `kind` (`String`, `Number`, `Integer`,
  `Boolean`, `Choice`, `Constant`), the control binding as `name`, `required`,
  `disabled`, `read_only`, `write_only`, `touched`, `dirty`, `nullable`, and the
  localized write-only replacement label and placeholder, write-only status
  text, and boolean value labels the built-in uses.
- `node()` is a target-scoped reactive reader, `actions()` the approved scalar
  actions for that node, and `extensions()` the prepared extension decorators.

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

An exact-widget renderer renders its whole region from the node-scoped context:

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
        let Some(projection) = context.node().read().ok().flatten() else {
            return rsx! {};
        };
        let actions = context.actions().clone();
        let presentation = context.presentation();
        let control = context.control();
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
                oninput: move |event| { let _ = actions.input_text(event.value()); }
            }
            if let Some(help) = &presentation.help {
                p { id: help.id.clone(), "{help.text}" }
            }
            {presentation.present_findings()}
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
| Handle and control operations | `HandleError` |
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
