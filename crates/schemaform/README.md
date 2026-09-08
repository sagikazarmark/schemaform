# schemaform

[![crates.io](https://img.shields.io/crates/v/schemaform?style=flat-square)](https://crates.io/crates/schemaform)
[![docs.rs](https://img.shields.io/docsrs/schemaform?style=flat-square)](https://docs.rs/schemaform)

**Runtime JSON Schema form definitions, state, validation, and submission.**

`schemaform` is a synchronous, Dioxus-free form engine for form shapes
discovered at runtime from application-trusted JSON Schema Draft 2020-12 data
schemas. It compiles a reusable definition, owns canonical JSON form data and
edit state, validates every accepted data change, and prepares immutable
submission snapshots, or advisory submissions for hosts that decide validity
themselves.

The first release supports a non-null fixed-object root containing supported
scalars, nested fixed objects, and one homogeneous array per root-to-leaf
branch. Nullable support applies only to scalar controls; nullable fixed objects
and nullable arrays are capability-blocking. Optional container properties may
be absent but do not accept null. Unsupported editing semantics are reported
explicitly rather than guessed or silently omitted.

Finite scalar choices come from `enum`, `const`, or a constant choice: a `oneOf`
or `anyOf` whose every branch is one scalar `const` plus annotations (`title`,
`description`, `$comment`, `deprecated`, `examples`, and keywords outside the
Draft 2020-12 vocabularies), optionally with a `type` the constant satisfies. A
constant choice compiles to the same choice control as `enum` with each option
labeled by its branch `title`, carrying its branch `description`, in authored
branch order; `enum` options stay sorted by value as before. Every other `oneOf`
or `anyOf` — a branch with any further assertion, applicator or annotation such
as `default`, a boolean-schema branch, or a `oneOf` with duplicate constants —
remains capability-blocking, and the finding names the reason.

An array that asserts `uniqueItems: true` over a finite item choice (`enum`, or
a constant choice) is a multiple choice: distinct members drawn from a finite
set. It still compiles to a homogeneous array — same node, item identities,
bindings, findings and collection operations — and additionally reports
`DefinitionNodeView::is_multiple_choice` with the item options as the array
node's `choice_options`, so a presentation can offer one toggle per option.
`UserActions::toggle_choice` adds a member in option order (checking B then A
yields `[A, B]`) or removes every item carrying it, without being gated by
`minItems` or `maxItems`, which remain findings. A member the data holds that is
no option is incompatible data offered for replacement, not dropped or invented
as an option. Because a multiple choice presents no item of its own, the array
node also attaches findings located at its items and can be blurred like a
scalar control, so its findings follow its own interaction state. Without
`uniqueItems`, or without a finite item choice, the array is a list of rows as
before.

Use [`schemaform-dioxus`](../schemaform-dioxus/README.md) to render a compiled
definition in a Dioxus browser application.

## Install

```toml
[dependencies]
schemaform = "0.1"
```

## Quick Start

```rust
use schemaform::{FormDefinition, SubmissionOutcome, Transition};
use serde_json::json;

fn process_transition(transition: &Transition) {
    for identity in transition.changed() {
        eprintln!("form node changed: {identity:?}");
    }
    for identity in transition.removed() {
        eprintln!("form node removed: {identity:?}");
    }
}

let definition = FormDefinition::compile(json!({
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "additionalProperties": false,
    "required": ["name"],
    "properties": {
        "name": { "type": "string", "title": "Name", "minLength": 1 }
    }
}))?;

let mut form = definition.create_form(json!({ "name": "Ada" }))?;
let name = form
    .node(form.view().root())
    .and_then(|root| root.children().next())
    .expect("the generated name control should exist");

let transition = form.user().input_text(name, "Grace")?;
process_transition(&transition);

let (transition, outcome) = form.prepare_submission().into_parts();
process_transition(&transition);
match outcome {
    SubmissionOutcome::Ready(snapshot) => {
        assert_eq!(snapshot.form_data(), &json!({ "name": "Grace" }));
    }
    SubmissionOutcome::Blocked(blockers) => {
        eprintln!("blocked by {} finding(s)", blockers.iter().count());
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

For inputs received as bytes, use `json::parse_data_schema`,
`json::parse_form_data`, and `json::parse_ui_schema_v1` before compilation or
construction. APIs accepting an existing `serde_json::Value` still enforce
post-parse structural limits, but cannot retroactively bound parsing or prior
allocation.

Strict UI-schema wire failures retain both the exact JSON Pointer and an owned
human-readable reason in `JsonParseError::InvalidUiSchema`. The reason is a
diagnostic rather than a stable machine-readable category; compiler-owned
UI-schema failures use `definition::UiSchemaInputErrorKind` where a stable
category is required.

Use `FormDefinition::compiler` when supplying a complete in-memory resource
graph, an authored UI schema, a default dialect for known inputs, or custom
finite limits. Resource retrieval is denied; referenced resources must be
provided by the application.

## Outcomes And Errors

| Stage | Public result |
| --- | --- |
| Bounded JSON ingestion | `JsonParseError` |
| Data-schema and UI-schema compilation | `CompileError` |
| Form construction | `FormBuildError` |
| User operations | `form::UserOperationError` |
| Privileged host transactions | `form::TransactionError` and `form::HostCommitError` |
| Reinitialization and external findings | Dedicated typed errors in `form` |
| Gated submission | `SubmissionOutcome::Ready` or `SubmissionOutcome::Blocked` |
| Advisory submission | `AdvisorySubmission` with its findings |

Schema-invalid but structurally permitted form data remains constructible,
visible, and repairable. A blocked submission is an ordinary outcome, not an
operation error. `prepare_submission` finalizes parseable buffers, updates
finding visibility, validates, and returns all current blockers or one immutable
snapshot. Serialization and transport remain application responsibilities.
Validation findings expose stable keyword codes, instance and data-schema
locations, and code-specific structured parameters through `ValidationFinding`;
adapters and hosts own localized presentation text.

## Two Submission Paths

The form is the authority on validity only when the host wants it to be.

| | `prepare_submission` | `prepare_advisory_submission` |
| --- | --- | --- |
| Returns | `SubmissionOutcome::Ready(SubmissionSnapshot)` or `SubmissionOutcome::Blocked(SubmissionBlockers)` | `AdvisorySubmission`: form data plus every finding the gated path would have blocked on |
| Refuses on | Parse blockers, validation findings, indeterminate validation, blocking capability findings, blocking external findings | Nothing; the same findings are reported as advisories |
| Data when findings exist | None | The current form data, which may violate the data schema |
| Unparseable edit buffers | Block | Stay out of the data and are reported as parse findings |
| Wants it | A host that treats the form as the last word: the snapshot is valid by construction and safe to send | A host that decides validity itself: a draft that may be saved incomplete, a client whose server validates again, or a protocol probe that must send what the data schema forbids to observe the other side |

Both paths prepare identically: they mark submission attempted, finalize
parseable edit buffers, and return a `Transition` the caller must process
because submission-only findings may have become visible. An
`AdvisorySubmission` is a different type from `SubmissionSnapshot` and offers
no conversion into one, so consumers expecting validated data cannot receive
findings-laden data by accident. The engine never commits unparseable numeric
text as a string; what to send for a member that could not be parsed is the
host's decision about its own wire format.

```rust
use schemaform::FormDefinition;
use serde_json::json;

let definition = FormDefinition::compile(json!({
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "additionalProperties": false,
    "properties": {
        "quantity": { "type": "integer", "minimum": 1 }
    }
}))?;
let mut form = definition.create_form(json!({ "quantity": 0 }))?;

let (transition, advisory) = form.prepare_advisory_submission().into_parts();
assert!(!transition.is_empty());
assert_eq!(advisory.form_data(), &json!({ "quantity": 0 }));
assert_eq!(advisory.findings().count(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Trust Boundary

Data schemas must be application-trusted for evaluator work. The package
meta-validates schemas, denies implicit I/O, and applies finite structural
limits, but it does not provide CPU, deadline, regex-work, or total evaluation
fuel containment for hostile schemas. Form data, UI schemas, edit buffers,
findings, and library-owned state growth are structurally bounded.

`writeOnly` affects built-in presentation. It does not hide data from host code,
validation, submission snapshots, or custom adapters. Applications remain
responsible for secrets, persistence, transport, and authorization.

### `format` is an annotation

`format` is collected as an annotation and never asserted, as Draft 2020-12's
default vocabulary has it: a string that does not look like its `format` is
valid, and no finding is raised for it. The annotation reaches adapters through
`DataSchemaAnnotations::formats`. The Dioxus adapter's built-in string control
uses it to pick the browser widget — `email`, `url`, `date`, `datetime-local`,
`time` — and its README records the
[mapping and the `datetime-local` decision](../schemaform-dioxus/README.md#string-formats-and-browser-widgets).
That is presentation only: the data stays a string, and a host that asserts
`format` downstream should read that section before pairing the picker with an
RFC 3339 `date-time`.

## Feature Flags

The crate has no public Cargo features. Product behavior is unconditional;
repository qualification hooks are not Cargo features and cannot be enabled by
dependency feature unification or `--all-features`.

### `serde_json/arbitrary_precision` is enabled for the whole build

`schemaform` depends on `serde_json/arbitrary_precision` and on
`jsonschema/arbitrary-precision`, which enables the same `serde_json` feature.
Cargo unifies features per crate across a build, so every `serde_json` user in a
host that depends on `schemaform` gets `arbitrary_precision` — including the
code that parses the host's own wire protocol and never asked for it. There is
no feature to turn this off, and this is not an oversight: exact numbers are the
product's number model, not an option layered on top of it. Under stock
`serde_json`, `0.1000000000000000000000000000000000000001` becomes `0.1` and
`184467440737095516160` becomes `1.8446744073709552e20` before the engine ever
sees them. Everything the engine promises about numbers rests on the literal
surviving:

- Form data keeps the spelling the user typed or the host supplied, and
  `display_text` renders it back; a form library that silently rewrites what the
  user typed is not a lighter mode of this one.
- Dirty state, edit no-op detection, choice matching, and host transactions
  compare numbers by mathematical value, so `1e3`, `1000`, and `1000.0` are one
  value with three spellings.
- Integer edits accept up to 4096 canonical digits by default, and
  `minItems`/`maxItems` bounds beyond `u64` are enforced exactly.
- The validator compares `minimum`, `maximum`, `multipleOf`, `const`, and `enum`
  against the authored literal, and bound findings report that literal rather
  than a rounding of it.

#### What changes for the host's own decoding

With `arbitrary_precision`, `serde_json` hands a number to a type that asks for
"any value" as a `u64` or `i64` when the literal is an integer that fits, and
otherwise — a fraction, an exponent, or an integer beyond `u64` or below `i64`
— as a one-entry map carrying the literal. A plain struct field typed `f64` is
unaffected: it asks for a float and gets one. Anything that buffers before
deciding what it is — `#[serde(tag = "…")]`, `#[serde(untagged)]`,
`#[serde(flatten)]`, or any other path through `serde`'s content buffer —
receives the map and fails with `invalid type: map, expected f64` or
`data did not match any variant of untagged enum …`. Integer-only fields in
those same types keep working, which is why the failure is easy to miss.

Decoding through `serde_json::Value` first is only a partial escape. `Value`
emits a float for a stored literal only when its spelling is what `f64`
formatting would produce: `0.5` and `1.0` decode; `1.50`, `1e2`, and `2.5E0`
still fail.

#### Add a tripwire; if it fires, canonicalize

Put a test on your wire layer that decodes a representative message through the
real decode path. Use spellings `f64` formatting would not reproduce — they fail
on the direct path and through `Value` alike — so the test fires however the
wire layer decodes today, and it points at the wire layer rather than at the
form:

```rust
#[test]
fn wire_messages_decode_under_the_current_dependency_set() {
    // Fires when any dependency enables `serde_json/arbitrary_precision` and a
    // buffered wire type with a float field meets a non-canonical number.
    let message = r#"{"type":"number","minimum":1.50,"maximum":1e2}"#;
    wire::decode::<PropertySchema>(message)
        .expect("the wire layer should decode non-canonical numbers");
}
```

If the wire types are yours, `serde_json::Number` fields decode under either
configuration. If they are not, re-spell the numbers to what stock `serde_json`
would have produced before the typed decoder sees them. At the cost of parsing
twice, this restores stock behavior for every message stock `serde_json` would
have accepted, including integers beyond `u64` becoming floats:

```rust
use serde::de::DeserializeOwned;
use serde_json::{Number, Value};

/// Decodes a wire message the way stock `serde_json` would have.
pub fn decode_wire<T: DeserializeOwned>(text: &str) -> serde_json::Result<T> {
    let mut value: Value = serde_json::from_str(text)?;
    canonicalize_numbers(&mut value);
    serde_json::from_value(value)
}

/// Re-spells every number that is not a machine integer as the `f64` it rounds to.
fn canonicalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if !number.is_u64() && !number.is_i64() => {
            if let Some(float) = number.as_f64().and_then(Number::from_f64) {
                *number = float;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(canonicalize_numbers),
        Value::Object(members) => members.values_mut().for_each(canonicalize_numbers),
        _ => {}
    }
}
```

Canonicalize only what the host decodes for itself. When a message carries a
data schema or form data bound for `schemaform` — as a protocol that ships the
form's schema inside its own envelope does — take that subtree from the
`Value` before the pass and hand it over unchanged; a typed `f64` field could
not have carried the exact literal anyway, and the engine's exactness depends on
receiving it. The repository's own qualification tests hold this section to the
locked `serde_json`: they assert that the direct path fails, that the `Value`
detour is partial, and that the recipe above — compiled from the same file the
block is checked against — decodes every spelling listed here.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](../../LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
