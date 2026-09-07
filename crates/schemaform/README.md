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

## Feature Flags

The first release has no public Cargo features. Product behavior is
unconditional; repository qualification hooks are not Cargo features and
cannot be enabled by dependency feature unification or `--all-features`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](../../LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
