# schemaform_daisyui

A `schemaform-dioxus` control renderer that presents string, number, and
integer controls with the `dioxus-daisyui-components` registry's `Field`,
`FieldLabel`, `Input`, `FieldDescription`, and `FieldError` parts, and its
`Button` for presence operations. Every other node kind — booleans, choices,
constants, layouts, groups, tabs, arrays, the form shell — falls back to the
adapter's built-in renderer.

## Browser CSR only

This component targets the browser client-side rendering path of
`schemaform-dioxus`. It is not supported under SSR, hydration, or a desktop
WebView: the edit hooks it is built on resynchronise the DOM after the core
rejects input, and the registry's `Input` focuses its native element through
`MountedData`.

## Layout

The directory is laid out as a `dx components` member so it can later move to a
registry without changing shape:

- `component.json` declares the registry components it is built on (`field`,
  `input`, `button`, each pinned to the one registry revision the copies were
  taken from, so the three parts share a contract) and the Cargo crates it
  compiles against. `dioxus-field` is pinned to the version the registry's own
  manifests declare, so the copied widgets and this mapping share one `Binding`
  and one `FieldMetaValues`. The `schemaform` and `schemaform-dioxus` entries
  name the release that ships the headless edit hooks; this demo builds them
  from the workspace instead.
- `mod.rs`, `component.rs`, and `mapping.rs` are what an install copies.
- The registry components live beside this one under `src/components/`,
  copied verbatim from the pinned revision and committed. CI never runs
  `dx components add`.

## What it maps

`schemaform-dioxus` owns the correctness-critical editing behaviour through
`use_text_edit`; this component only maps that onto the `dioxus-field`
convention the registry speaks:

| schemaform-dioxus | dioxus-field |
| --- | --- |
| `TextEdit::value` | `Binding::read` |
| `TextEdit::input` | `Binding::write` (the change origin is ignored) |
| — | `Binding::commit` is a no-op: the core applies every keystroke |
| `TextEdit::blur` | `Binding::focus_exit` |
| `NodePresentation::element_id` / `ControlFacets::name` | `FieldMetaValues::id` / `name` |
| `ControlFacets::{required, disabled, touched, dirty}` | the same flags |
| `NodePresentation::invalid` | `FieldMetaValues::invalid = Some(..)` |
| blocking parse, validation, and external findings | `FieldMetaValues::errors` |

The binding's identity is the edit's hook-stable handles (its value signal and
its `input` and `blur` callbacks), so bindings built on later renders compare
equal and the registry widgets neither re-render per keystroke nor re-register
focus.

`FieldError` renders `errors` only while the field is invalid, so it receives
the findings that block submission and concern the entered value: parse
blockers, validation findings, and blocking external findings. Every other
finding — help text, capability findings, advisory external findings — is
rendered as a `FieldDescription` carrying its stable id, so every id the
adapter hands out resolves to an element. A blocking capability finding
therefore still marks the field invalid but is read as a description, since it
is about what the form can present rather than what the user typed.

A read-only node renders as a noninteractive `output` inside the same `Field`,
as the built-in does. Write-only controls use the password input type and the
localized replacement label and placeholder. IME composition start and end are
forwarded to the hook through the `Input`'s explicit attributes.

## Usage

```rust
use schemaform_dioxus::RenderConfiguration;

use crate::components::schemaform_daisyui;

let bound = RenderConfiguration::builder()
    .controls(schemaform_daisyui::controls())
    .build()
    .bind(&form)?;
```

`controls()` starts from `ControlRegistry::with_builtins()` and registers the
renderer through a matcher above the built-in priority that accepts exactly
string, number, and integer definition nodes.
