# schemaform_daisyui

A `schemaform-dioxus` control renderer that presents every control kind with
the `dioxus-daisyui-components` registry: strings, numbers, and integers with
`Field`, `FieldLabel`, `Input`, `FieldDescription`, and `FieldError`; booleans
with a native checkbox or the registry's `Checkbox`; choices with
`NativeSelect`, `RadioGroup`, or `Select`; constants as read-only output; and
presence operations as its `Button`. A whole form's controls are therefore
daisyUI-rendered, and only the structural nodes — layouts, groups, tabs,
arrays, the form shell — still come from the adapter's built-in renderer.

## Browser CSR only

This component targets the browser client-side rendering path of
`schemaform-dioxus`. It is not supported under SSR, hydration, or a desktop
WebView: the edit hooks it is built on resynchronise the DOM after the core
rejects input, and the registry's widgets focus their native elements through
`MountedData` and `document.eval`.

## Layout

The directory is laid out as a `dx components` member so it can later move to a
registry without changing shape:

- `component.json` declares the registry components it is built on (`field`,
  `input`, `checkbox`, `native_select`, `radio_group`, `select`, `button`, each
  pinned to the one registry revision the copies were taken from, so the parts
  share a contract) and the Cargo crates it compiles against. `dioxus-field` is
  pinned to the version the registry's own manifests declare, so the copied
  widgets and this mapping share one `Binding` and one `FieldMetaValues`. The
  `schemaform` and `schemaform-dioxus` entries name the release that ships the
  headless edit hooks; this demo builds them from the workspace instead.
- `mod.rs`, `component.rs`, `mapping.rs`, `parts.rs`, `text.rs`, `boolean.rs`,
  `choice.rs`, and `constant.rs` are what an install copies.
- The registry components live beside this one under `src/components/`,
  copied verbatim from the pinned revision and committed. CI never runs
  `dx components add`.

## What it renders

| Control kind | Widget |
| --- | --- |
| string, number, integer | `Input` (`type="password"` when write-only) |
| boolean, not nullable | native `input type="checkbox"` with daisyUI's `checkbox` class, driven by `use_boolean_edit` |
| boolean, nullable | registry `Checkbox`; JSON null is the indeterminate state |
| boolean, write-only | `NativeSelect<bool>` over the localized false/true labels, resting on the replacement placeholder |
| choice | `NativeSelect<ChoiceIdentity>` by default |
| choice with `"widget": "daisyui:radio"` | `RadioGroup` with one `RadioItem` per option |
| choice with `"widget": "daisyui:select"` | the compound `Select` with one `SelectOption` per option |
| constant | read-only `output` from presentation and facets |
| any read-only node | read-only `output` inside the same `Field` |

The null option of a nullable choice is an ordinary option in all three choice
widgets; selecting it sets null. A non-nullable boolean keeps the built-in's
native semantics on purpose: a native checkbox is what browsers, assistive
technology, and the existing tests expect. The nullable checkbox reaches null
through the set-null presence affordance, since a click from either boolean
state yields a boolean; the `CheckboxState` mapping lives in this component
only, never in the published crates.

Write-only booleans and choices never echo their value: the widget rests on the
replacement placeholder, the edit hook puts it back there after every write,
and the label is the localized replacement label. A constant is never an
editable widget with a metadata-only field context; it is output, with the
presence affordances that materialize or remove it.

## What it maps

`schemaform-dioxus` owns the correctness-critical editing behaviour through
`use_text_edit`, `use_boolean_edit`, and `use_choice_edit`; this component only
maps those onto the `dioxus-field` convention the registry speaks:

| schemaform-dioxus | dioxus-field |
| --- | --- |
| `TextEdit::value` / `input` / `blur` | `Binding<String>` read / write / focus exit |
| `BooleanEdit::checked` / `set` / `blur` | `Binding<Option<bool>>` read / write / focus exit |
| `BooleanEdit` with null ↔ `CheckboxState::Indeterminate` | `Binding<CheckboxState>` |
| `ChoiceEdit::selected` / `select` / `blur` | `Binding<Option<ChoiceIdentity>>` read / write / focus exit |
| `ChoiceEdit` with the identity as its `as_str` form | `Binding<String>` for `RadioGroup` |
| — | `Binding::commit` is a no-op for every kind: the core applies each edit as it happens |
| `NodePresentation::element_id` / `ControlFacets::name` | `FieldMetaValues::id` / `name` |
| `ControlFacets::{required, disabled, touched, dirty}` | the same flags |
| `NodePresentation::invalid` | `FieldMetaValues::invalid = Some(..)` |
| blocking parse, validation, and external findings | `FieldMetaValues::errors` |

Every binding's identity is its edit's hook-stable handles (the value signal
and the callbacks), so bindings built on later renders compare equal and the
registry widgets neither re-render per edit nor re-register focus. A native
`option`'s value string (`NativeSelectOption::form_value`), and a
`RadioItem`'s value, is the option's opaque `ChoiceIdentity` string: the same
string the edit hook writes back into the widget after a rejected write.

`FieldError` renders `errors` only while the field is invalid, so it receives
the findings that block submission and concern the entered value: parse
blockers, validation findings, and blocking external findings. Every other
finding — help text, capability findings, advisory external findings — is
rendered as a `FieldDescription` carrying its stable id, so every id the
adapter hands out resolves to an element. A blocking capability finding
therefore still marks the field invalid but is read as a description, since it
is about what the form can present rather than what the user typed.

Two registry behaviours to know about the compound `Select`: it reports focus
exit one task after its trigger loses focus, so touched state lands one task
later than with the native select; and it renders no hidden form participant,
so a `daisyui:select` choice carries the control binding as the trigger's
`name` but does not take part in native form submission. The native select and
the radio group's hidden radios do.

## Usage

```rust
use schemaform_dioxus::RenderConfiguration;

use crate::components::schemaform_daisyui;

let bound = RenderConfiguration::builder()
    .controls(schemaform_daisyui::controls())
    .build()
    .bind(&form)?;
```

`controls()` starts from `ControlRegistry::with_builtins()`, registers the
renderer through a matcher above the built-in priority that accepts every
definition node the adapter derives a control kind from, and registers one
renderer each for the exact widget symbols `daisyui:radio` and `daisyui:select`
(`RADIO_WIDGET` and `SELECT_WIDGET`). The widget symbols are named on a UI
schema control:

```json
{
  "type": "control",
  "value": {
    "binding": { "origin": "root", "pointer": "/billing_cycle" },
    "widget": "daisyui:radio"
  }
}
```

A widget symbol on a non-choice control changes nothing: the renderer
dispatches on the control kind first.
