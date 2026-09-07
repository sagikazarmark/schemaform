use schemaform::{
    ExternalFinding, ExternalFindingBatch, FindingView, Form, FormDefinition, InstanceIdentity,
    JsonPointer, SubmissionOutcome,
    form::{FindingVisibility, FindingVisibilityPolicy, ParseBlockerKind, SubmissionBlocker},
};
use serde_json::{Value, json};

#[test]
fn range_violation_yields_the_out_of_range_value_and_one_validation_finding() {
    let mut form = quantity_definition(json!({ "type": "integer", "minimum": 1 }))
        .create_form(json!({ "quantity": 1 }))
        .expect("the form should be created");
    let quantity = node_with_binding(&form, "/quantity");
    form.user()
        .input_text(quantity, "0")
        .expect("the below-minimum integer should remain editable");

    let (_, advisory) = form.prepare_advisory_submission().into_parts();

    assert_eq!(advisory.form_data(), &json!({ "quantity": 0 }));
    assert_eq!(finding_kinds(&advisory), ["validation"]);
    assert!(advisory.findings().any(|finding| matches!(
        finding,
        SubmissionBlocker::Validation(finding)
            if finding.code() == "minimum"
                && finding.instance_location().as_str() == "/quantity"
                && finding.parameters() == &json!({ "limit": 1 })
    )));
}

#[test]
fn unparseable_integer_buffer_yields_data_without_the_member_and_one_parse_finding() {
    let mut form = quantity_definition(json!({ "type": "integer" }))
        .create_form(json!({}))
        .expect("the form should be created");
    let quantity = node_with_binding(&form, "/quantity");
    form.user()
        .input_text(quantity, "-")
        .expect("the incomplete integer should remain buffered");

    let (_, advisory) = form.prepare_advisory_submission().into_parts();

    assert_eq!(advisory.form_data(), &json!({}));
    assert_eq!(finding_kinds(&advisory), ["parse"]);
    assert!(advisory.findings().any(|finding| matches!(
        finding,
        SubmissionBlocker::Parse {
            target,
            kind: ParseBlockerKind::InvalidInteger,
        } if *target == quantity
    )));
    assert_eq!(
        form.node(quantity)
            .expect("the quantity control should exist")
            .edit_buffer(),
        Some("-"),
        "an unparseable buffer stays interaction state rather than being committed"
    );
}

#[test]
fn blocking_capability_finding_yields_data_and_that_finding() {
    let definition = one_of_definition();
    let mut form = definition
        .create_form(json!({ "contact": "ada@example.test", "name": "Ada" }))
        .expect("valid form data should instantiate the lenient definition");

    let (_, advisory) = form.prepare_advisory_submission().into_parts();

    assert_eq!(
        advisory.form_data(),
        &json!({ "contact": "ada@example.test", "name": "Ada" })
    );
    assert_eq!(finding_kinds(&advisory), ["capability"]);
    assert!(advisory.findings().any(|finding| matches!(
        finding,
        SubmissionBlocker::Capability(finding) if finding.code() == "applicator.one-of"
    )));
    assert_eq!(advisory.definition_fingerprint(), definition.fingerprint());
}

#[test]
fn valid_form_yields_data_and_no_findings_with_gated_preparation_semantics() {
    let definition = quantity_definition(json!({ "type": "integer" }));
    let mut form = definition
        .form(json!({ "quantity": 1 }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::SubmissionOnly,
            FindingVisibility::SubmissionOnly,
        ))
        .build()
        .expect("the form should be created");
    let quantity = node_with_binding(&form, "/quantity");
    form.user()
        .input_text(quantity, "1e3")
        .expect("the parseable spelling should update canonical data");
    assert_eq!(
        form.node(quantity)
            .expect("the quantity control should exist")
            .edit_buffer(),
        Some("1e3")
    );
    assert!(!form.view().submission_attempted());

    let preparation = form.prepare_advisory_submission();

    assert_ne!(
        preparation.transition().before_state_revision(),
        preparation.transition().after_state_revision(),
        "advisory preparation should expose its one state transition"
    );
    assert_eq!(
        preparation.transition().before_data_revision(),
        preparation.transition().after_data_revision(),
        "finalizing a parseable buffer does not change canonical data"
    );
    assert!(
        preparation
            .transition()
            .changed()
            .any(|identity| identity == quantity),
        "the finalized control should be reported as changed"
    );
    assert!(form.view().submission_attempted());
    assert_eq!(
        form.node(quantity)
            .expect("the quantity control should exist")
            .edit_buffer(),
        None,
        "a parseable buffer is finalized exactly as the gated path does"
    );

    let advisory = preparation.submission();
    assert_eq!(advisory.form_data(), &json!({ "quantity": 1000 }));
    assert_eq!(advisory.data_revision(), form.view().data_revision());
    assert_eq!(advisory.definition_fingerprint(), definition.fingerprint());
    assert_eq!(advisory.findings().count(), 0);

    let owned = advisory.clone();
    form.user()
        .input_text(quantity, "2")
        .expect("the form should remain editable after advisory preparation");
    assert_eq!(
        owned.form_data(),
        &json!({ "quantity": 1000 }),
        "later edits do not modify an owned advisory submission"
    );
}

#[test]
fn advisory_preparation_reveals_submission_only_findings_like_the_gated_path() {
    let mut form = quantity_definition(json!({ "type": "integer", "minimum": 1 }))
        .form(json!({ "quantity": 0 }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::SubmissionOnly,
            FindingVisibility::SubmissionOnly,
        ))
        .build()
        .expect("the form should be created");
    assert_eq!(form.view().visible_findings().count(), 0);

    let preparation = form.prepare_advisory_submission();

    assert_eq!(finding_kinds(preparation.submission()), ["validation"]);
    assert!(form.view().visible_findings().any(|finding| matches!(
        finding,
        FindingView::Validation { finding, .. } if finding.code() == "minimum"
    )));
    assert!(
        preparation
            .transition()
            .changed()
            .any(|identity| identity == node_with_binding(&form, "/quantity")),
        "revealing a finding should report the control as changed"
    );
}

#[test]
fn every_blocker_family_becomes_an_advisory_and_advisory_external_findings_are_excluded() {
    let mut form = every_family_form();

    let (transition, advisory) = form.prepare_advisory_submission().into_parts();

    assert_ne!(
        transition.before_state_revision(),
        transition.after_state_revision()
    );
    assert_eq!(
        advisory.form_data(),
        &json!({ "age": 1, "contact": "Ada", "name": "", "quantity": 1 }),
        "the parse-blocked control keeps its prior canonical value"
    );
    assert_eq!(
        finding_kinds(&advisory),
        [
            "parse",
            "validation",
            "validation",
            "capability",
            "external"
        ]
    );
    assert!(advisory.findings().any(|finding| matches!(
        finding,
        SubmissionBlocker::External { source, finding }
            if source == "server" && finding.code() == "server-rejected" && finding.is_blocking()
    )));
    assert!(
        !advisory
            .findings()
            .any(|finding| matches!(finding, SubmissionBlocker::External { finding, .. } if finding.code() == "advice")),
        "a non-blocking external finding was never a blocker and stays out of the advisory list"
    );
}

#[test]
fn the_gated_path_is_unchanged_by_the_advisory_path() {
    let mut gated_only = every_family_form();
    let mut advisory_then_gated = every_family_form();

    let expected = match gated_only.prepare_submission().into_parts().1 {
        SubmissionOutcome::Blocked(blockers) => blocker_summary(&gated_only, blockers.iter()),
        SubmissionOutcome::Ready(_) => panic!("every blocker family should prevent submission"),
    };

    let (_, advisory) = advisory_then_gated
        .prepare_advisory_submission()
        .into_parts();
    assert_eq!(
        blocker_summary(&advisory_then_gated, advisory.findings()),
        expected,
        "the advisory list is exactly what the gated path reports as blockers"
    );
    match advisory_then_gated.prepare_submission().into_parts().1 {
        SubmissionOutcome::Blocked(blockers) => assert_eq!(
            blocker_summary(&advisory_then_gated, blockers.iter()),
            expected,
            "a preceding advisory preparation leaves the gated outcome untouched"
        ),
        SubmissionOutcome::Ready(_) => panic!("every blocker family should prevent submission"),
    }
    assert_eq!(gated_only.form_data(), advisory_then_gated.form_data());
    assert_eq!(gated_only.form_data(), advisory.form_data());

    let definition = quantity_definition(json!({ "type": "integer" }));
    let mut ready = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the ready form should be created");
    let quantity = node_with_binding(&ready, "/quantity");
    ready
        .user()
        .input_text(quantity, "1e3")
        .expect("the parseable spelling should update canonical data");
    let (_, advisory) = ready.prepare_advisory_submission().into_parts();
    let snapshot = match ready.prepare_submission().into_parts().1 {
        SubmissionOutcome::Ready(snapshot) => snapshot,
        SubmissionOutcome::Blocked(_) => panic!("the valid form should be ready"),
    };
    assert_eq!(
        serde_json::to_string(advisory.form_data()).expect("the advisory data should serialize"),
        serde_json::to_string(snapshot.form_data()).expect("the snapshot should serialize"),
    );
    assert_eq!(advisory.data_revision(), snapshot.data_revision());
    assert_eq!(
        advisory.definition_fingerprint(),
        snapshot.definition_fingerprint()
    );
}

#[cfg(schemaform_test_validation_faults)]
#[test]
fn indeterminate_validation_becomes_an_advisory_rather_than_a_refusal() {
    let mut form = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "x-schemaform-test-validation-fault": true,
        "type": "object",
        "additionalProperties": false,
        "properties": { "quantity": { "type": "integer" } }
    }))
    .expect("the private validator fault fixture should compile")
    .create_form(json!({ "quantity": 1 }))
    .expect("the form should be created");

    let (_, advisory) = form.prepare_advisory_submission().into_parts();

    assert_eq!(advisory.form_data(), &json!({ "quantity": 1 }));
    assert_eq!(finding_kinds(&advisory), ["indeterminate"]);
    assert!(advisory.findings().any(|finding| matches!(
        finding,
        SubmissionBlocker::Indeterminate(reason) if reason.code() == "injected-validator-failure"
    )));
}

fn every_family_form() -> Form {
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["age", "contact", "name", "quantity"],
        "properties": {
            "age": { "type": "integer", "minimum": 18 },
            "contact": {
                "oneOf": [{ "type": "string" }, { "type": "integer" }]
            },
            "name": { "type": "string", "minLength": 3 },
            "quantity": { "type": "integer" }
        }
    }))
    .analyze()
    .expect("lenient analysis should preserve the unsupported region")
    .into_parts()
    .0;
    let mut form = definition
        .form(json!({ "age": 1, "contact": "Ada", "name": "", "quantity": 1 }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::SubmissionOnly,
            FindingVisibility::SubmissionOnly,
        ))
        .build()
        .expect("the form should be created");
    let quantity = node_with_binding(&form, "/quantity");
    form.user()
        .input_text(quantity, "-")
        .expect("the parse-blocked value should remain buffered");
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [
            ExternalFinding::advisory("advice", pointer("/name"), json!({})),
            ExternalFinding::blocking(
                "server-rejected",
                pointer("/missing"),
                json!({ "retry": false }),
            ),
        ],
    ))
    .expect("the current external batch should apply");
    form
}

fn quantity_definition(quantity: Value) -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": { "quantity": quantity }
    }))
    .expect("the data schema should compile")
}

fn one_of_definition() -> FormDefinition {
    FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["contact", "name"],
        "properties": {
            "contact": {
                "oneOf": [
                    { "type": "string" },
                    { "type": "integer", "minimum": 1 }
                ]
            },
            "name": { "type": "string" }
        }
    }))
    .analyze()
    .expect("lenient analysis should succeed")
    .into_parts()
    .0
}

fn pointer(pointer: &str) -> JsonPointer {
    JsonPointer::parse(pointer).expect("the test pointer should be valid")
}

fn finding_kinds(advisory: &schemaform::AdvisorySubmission) -> Vec<&'static str> {
    advisory.findings().map(blocker_kind).collect()
}

fn blocker_kind(blocker: &SubmissionBlocker) -> &'static str {
    match blocker {
        SubmissionBlocker::Parse { .. } => "parse",
        SubmissionBlocker::Validation(_) => "validation",
        SubmissionBlocker::ValidationFindingsTruncated { .. } => "validation-truncated",
        SubmissionBlocker::Indeterminate(_) => "indeterminate",
        SubmissionBlocker::Capability(_) => "capability",
        SubmissionBlocker::External { .. } => "external",
        _ => "unknown",
    }
}

fn blocker_summary<'a>(
    form: &Form,
    blockers: impl Iterator<Item = &'a SubmissionBlocker>,
) -> Vec<(&'static str, String, String)> {
    blockers
        .map(|blocker| {
            let (location, code) = match blocker {
                SubmissionBlocker::Parse { target, kind } => (
                    form.node(*target)
                        .and_then(|node| node.binding())
                        .map(|binding| binding.pointer().as_str().to_owned())
                        .expect("the parse target should be a bound control"),
                    format!("{kind:?}"),
                ),
                SubmissionBlocker::Validation(finding) => (
                    finding.instance_location().as_str().to_owned(),
                    finding.code().to_owned(),
                ),
                SubmissionBlocker::ValidationFindingsTruncated { retained } => {
                    (String::new(), retained.to_string())
                }
                SubmissionBlocker::Indeterminate(reason) => {
                    (String::new(), reason.code().to_owned())
                }
                SubmissionBlocker::Capability(finding) => (
                    finding.instance_location().as_str().to_owned(),
                    finding.code().to_owned(),
                ),
                SubmissionBlocker::External { source, finding } => (
                    finding.instance_location().as_str().to_owned(),
                    format!("{source}:{}", finding.code()),
                ),
                _ => (String::new(), "unknown".to_owned()),
            };
            (blocker_kind(blocker), location, code)
        })
        .collect()
}

fn node_with_binding(form: &Form, binding: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).expect("the form node should exist");
        if node
            .binding()
            .is_some_and(|current| current.pointer().as_str() == binding)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("the bound node should exist")
}
