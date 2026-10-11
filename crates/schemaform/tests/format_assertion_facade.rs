//! Public-facade tests for opt-in `format` assertion.
//!
//! `format` is an annotation unless the host asserts it at compile time; an asserted format that a
//! value does not match is an ordinary validation finding at that value.

use schemaform::{
    AssertedFormat, Form, FormDefinition, InstanceIdentity, SubmissionOutcome,
    form::SubmissionBlocker,
};
use serde_json::{Value, json};

fn data_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "when": { "type": "string", "format": "date-time" },
            "email": { "type": "string", "format": "email" },
            "id": { "type": "string", "format": "uuid" }
        }
    })
}

/// What a `datetime-local` widget writes, an address with no domain, and a non-UUID.
fn nonconforming_data() -> Value {
    json!({ "when": "2025-01-31T09:30", "email": "ada", "id": "not-a-uuid" })
}

fn asserting_definition() -> FormDefinition {
    FormDefinition::compiler(data_schema())
        .assert_formats([AssertedFormat::DateTime, AssertedFormat::Email])
        .compile()
        .expect("the asserting definition should compile")
}

fn format_findings(form: &Form, binding: &str) -> Vec<(String, Value)> {
    form.node(node_with_binding(form, binding))
        .expect("the control should exist")
        .validation_findings()
        .map(|finding| (finding.code().to_owned(), finding.parameters().clone()))
        .collect()
}

#[test]
fn formats_are_annotations_unless_asserted() {
    let definition = FormDefinition::compile(data_schema()).expect("the schema should compile");
    let mut form = definition
        .create_form(nonconforming_data())
        .expect("the form should be created");

    assert!(matches!(
        form.prepare_submission().into_parts().1,
        SubmissionOutcome::Ready(_)
    ));
}

#[test]
fn an_asserted_format_a_value_does_not_match_is_a_finding_at_that_value() {
    let mut form = asserting_definition()
        .create_form(nonconforming_data())
        .expect("schema-invalid data should still be constructible");
    form.prepare_advisory_submission();

    assert_eq!(
        format_findings(&form, "/when"),
        vec![("format".to_owned(), json!({ "format": "date-time" }))]
    );
    assert_eq!(
        format_findings(&form, "/email"),
        vec![("format".to_owned(), json!({ "format": "email" }))]
    );
    // `uuid` was not asserted, so it stays an annotation.
    assert!(format_findings(&form, "/id").is_empty());
}

#[test]
fn asserted_format_findings_block_a_gated_submission_and_travel_with_an_advisory_one() {
    let mut form = asserting_definition()
        .create_form(nonconforming_data())
        .expect("schema-invalid data should still be constructible");

    let (_, advisory) = form.prepare_advisory_submission().into_parts();
    let mut advisory_formats = advisory
        .findings()
        .filter_map(|blocker| match blocker {
            SubmissionBlocker::Validation(finding) if finding.code() == "format" => {
                Some(finding.instance_location().as_str().to_owned())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    advisory_formats.sort();
    assert_eq!(advisory_formats, ["/email", "/when"]);

    let SubmissionOutcome::Blocked(blockers) = form.prepare_submission().into_parts().1 else {
        panic!("a value that does not match an asserted format should block submission");
    };
    assert_eq!(blockers.iter().count(), 2);

    let mut conforming = asserting_definition()
        .create_form(json!({
            "when": "2025-01-31T09:30:00+01:00",
            "email": "ada@example.test",
            "id": "not-a-uuid"
        }))
        .expect("the conforming form should be created");
    assert!(matches!(
        conforming.prepare_submission().into_parts().1,
        SubmissionOutcome::Ready(_)
    ));
}

#[test]
fn the_asserted_formats_are_part_of_the_definition_fingerprint() {
    let annotating = FormDefinition::compile(data_schema()).expect("the schema should compile");
    let asserting = asserting_definition();
    let reordered = FormDefinition::compiler(data_schema())
        .assert_formats([AssertedFormat::Email])
        .assert_formats([AssertedFormat::DateTime, AssertedFormat::Email])
        .compile()
        .expect("the reordered definition should compile");

    assert_ne!(annotating.fingerprint(), asserting.fingerprint());
    assert_eq!(asserting.fingerprint(), reordered.fingerprint());
}

fn node_with_binding(form: &Form, binding: &str) -> InstanceIdentity {
    let root = form.view().root();
    form.node(root)
        .expect("the form root should exist")
        .children()
        .find(|identity| {
            form.node(*identity)
                .and_then(|node| node.binding().map(|binding| binding.pointer().clone()))
                .is_some_and(|pointer| pointer.as_str() == binding)
        })
        .expect("the bound control should exist")
}
