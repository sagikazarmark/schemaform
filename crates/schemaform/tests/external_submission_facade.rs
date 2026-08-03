use std::num::NonZeroUsize;

use schemaform::{
    ExternalFinding, ExternalFindingBatch, FindingView, Form, FormDefinition, InstanceIdentity,
    JsonPointer, SubmissionOutcome,
    form::{
        ExternalFindingError, ExternalFindingLimits, FindingVisibility, FindingVisibilityPolicy,
        SubmissionBlocker,
    },
};
use serde_json::json;

#[test]
fn external_batches_replace_by_source_only_for_the_current_data_revision() {
    let mut form = form_with_visibility(FindingVisibilityPolicy::new(
        FindingVisibility::Immediate,
        FindingVisibility::Immediate,
    ));
    let name = node_with_binding(&form, "/name");
    let quantity = node_with_binding(&form, "/quantity");
    let revision = form.view().data_revision();
    let original = external(
        "server-rejected",
        "/quantity",
        true,
        json!({ "attempt": 1 }),
    );
    let replacement = external(
        "server-retry-required",
        "/quantity",
        true,
        json!({ "attempt": 2 }),
    );
    let policy = external(
        "policy-rejected",
        "/missing",
        false,
        json!({ "rule": "approval" }),
    );
    let audit = external("review-name", "/name", false, json!({}));

    let server_transition = form
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            revision,
            [original.clone()],
        ))
        .expect("the current server batch should apply");
    assert_eq!(server_transition.changed().collect::<Vec<_>>(), [quantity]);
    let policy_transition = form
        .apply_external_findings(ExternalFindingBatch::new("policy", revision, [policy]))
        .expect("a second source should apply independently");
    assert_eq!(
        policy_transition.changed().collect::<Vec<_>>(),
        [form.view().root()],
        "an unmatched finding should attach to and update the root"
    );
    form.apply_external_findings(ExternalFindingBatch::new("audit", revision, [audit]))
        .expect("an exactly attached advisory should apply independently");
    form.apply_external_findings(ExternalFindingBatch::new("server", revision, [replacement]))
        .expect("a source should replace its previous batch");

    assert_eq!(
        external_summary(&form),
        [
            (
                "audit".to_owned(),
                "/name".to_owned(),
                "review-name".to_owned(),
                false,
                json!({}),
            ),
            (
                "policy".to_owned(),
                "/missing".to_owned(),
                "policy-rejected".to_owned(),
                false,
                json!({ "rule": "approval" }),
            ),
            (
                "server".to_owned(),
                "/quantity".to_owned(),
                "server-retry-required".to_owned(),
                true,
                json!({ "attempt": 2 }),
            ),
        ]
    );
    assert_eq!(
        form.node(quantity)
            .expect("the quantity should exist")
            .visible_findings()
            .filter(|finding| matches!(finding, FindingView::External { .. }))
            .count(),
        1,
        "only an exactly attached finding should be local"
    );

    let state_before_stale = form.view().state_revision();
    let stale = form
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            another_form_revision(),
            [original],
        ))
        .expect_err("a revision from another form must be rejected");
    assert!(matches!(stale, ExternalFindingError::StaleRevision { .. }));
    assert_eq!(form.view().state_revision(), state_before_stale);
    assert_eq!(external_summary(&form).len(), 3);
    assert!(form.view().visible_findings().any(|finding| matches!(
        finding,
        FindingView::External {
            target,
            source: "policy",
            ..
        } if target == form.view().root()
    )));
    assert!(
        form.node(form.view().root())
            .expect("the root should exist")
            .visible_findings()
            .any(|finding| matches!(
                finding,
                FindingView::External {
                    source: "policy",
                    ..
                }
            ))
    );

    let transition = form
        .user()
        .input_text(quantity, "2")
        .expect("a canonical quantity edit should succeed");
    assert!(
        transition.changed().any(|identity| identity == name),
        "a canonical change should invalidate local findings on other controls"
    );
    assert!(external_summary(&form).is_empty());
    assert!(matches!(
        form.apply_external_findings(ExternalFindingBatch::new(
            "server",
            revision,
            [external("stale", "/quantity", true, json!({}))],
        )),
        Err(ExternalFindingError::StaleRevision { .. })
    ));
}

#[test]
fn validation_and_external_visibility_are_independent_while_parse_blockers_are_immediate() {
    let mut form = form_with_visibility(FindingVisibilityPolicy::new(
        FindingVisibility::SubmissionOnly,
        FindingVisibility::Immediate,
    ));
    let name = node_with_binding(&form, "/name");
    let quantity = node_with_binding(&form, "/quantity");
    form.user()
        .input_text(name, "")
        .expect("schema-invalid text should remain editable");
    form.user()
        .input_text(quantity, "-")
        .expect("incomplete numeric text should remain buffered");
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [external("name-taken", "/name", true, json!({}))],
    ))
    .expect("the current external finding should apply");

    assert_eq!(visible_kinds(&form), ["external", "parse"]);

    form.set_finding_visibility(FindingVisibilityPolicy::new(
        FindingVisibility::Immediate,
        FindingVisibility::SubmissionOnly,
    ));
    assert_eq!(visible_kinds(&form), ["validation", "parse"]);

    form.set_finding_visibility(FindingVisibilityPolicy::new(
        FindingVisibility::TouchedOrSubmission,
        FindingVisibility::TouchedOrSubmission,
    ));
    assert_eq!(visible_kinds(&form), ["parse"]);
    form.user()
        .blur(name)
        .expect("blurring the name should mark its exact target touched");
    assert_eq!(visible_kinds(&form), ["validation", "external", "parse"]);

    let preparation = form.prepare_submission();
    let blockers = match preparation.outcome() {
        SubmissionOutcome::Blocked(blockers) => blockers,
        SubmissionOutcome::Ready(_) => panic!("hidden findings and a parse blocker must block"),
    };
    assert_eq!(
        blocker_kinds(blockers.iter()),
        ["parse", "validation", "external"]
    );
    assert_eq!(visible_kinds(&form), ["validation", "external", "parse"]);
}

#[test]
fn visibility_transitions_report_only_nodes_whose_observable_state_changed() {
    let mut form = form_with_visibility(FindingVisibilityPolicy::new(
        FindingVisibility::SubmissionOnly,
        FindingVisibility::SubmissionOnly,
    ));
    let root = form.view().root();
    let revision = form.view().data_revision();
    form.apply_external_findings(ExternalFindingBatch::new(
        "policy",
        revision,
        [external("unmatched", "/missing", true, json!({}))],
    ))
    .unwrap();

    let transition = form.set_finding_visibility(FindingVisibilityPolicy::new(
        FindingVisibility::SubmissionOnly,
        FindingVisibility::Immediate,
    ));

    assert_eq!(transition.changed().collect::<Vec<_>>(), [root]);
}

#[test]
fn submission_returns_one_transition_and_every_structured_blocker_or_a_minimal_snapshot() {
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
            external("advice", "/name", false, json!({})),
            external(
                "server-rejected",
                "/missing",
                true,
                json!({ "retry": false }),
            ),
        ],
    ))
    .expect("the current external batch should apply");

    let preparation = form.prepare_submission();
    assert_ne!(
        preparation.transition().before_state_revision(),
        preparation.transition().after_state_revision(),
        "submission should expose its one state transition"
    );
    let blockers = match preparation.outcome() {
        SubmissionOutcome::Blocked(blockers) => blockers,
        SubmissionOutcome::Ready(_) => panic!("every blocker family should prevent submission"),
    };
    assert_eq!(
        blocker_kinds(blockers.iter()),
        [
            "parse",
            "validation",
            "validation",
            "capability",
            "external"
        ]
    );
    let validation = blockers
        .iter()
        .filter_map(|blocker| match blocker {
            SubmissionBlocker::Validation(finding) => Some((
                finding.instance_location().as_str(),
                finding.code(),
                finding.parameters(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(validation.len(), 2);
    assert!(validation.contains(&("/age", "minimum", &json!({ "limit": 18 }))));
    assert!(
        validation
            .iter()
            .any(|(location, code, _)| { *location == "/name" && *code == "minLength" })
    );
    assert!(blockers.iter().any(|blocker| matches!(
        blocker,
        SubmissionBlocker::External { source, finding }
            if source == "server"
                && finding.code() == "server-rejected"
                && finding.parameters() == &json!({ "retry": false })
    )));

    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": { "quantity": { "type": "integer" } }
    }))
    .expect("the submission schema should compile");
    let fingerprint = definition.fingerprint();
    let mut ready = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the ready form should be created");
    let quantity = node_with_binding(&ready, "/quantity");
    ready
        .user()
        .input_text(quantity, "1e3")
        .expect("the parseable spelling should update canonical data");
    let prepared = ready.prepare_submission();
    assert!(ready.node(quantity).unwrap().edit_buffer().is_none());
    let snapshot = match prepared.outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the valid form should be ready"),
    };
    assert_eq!(snapshot.form_data(), &json!({ "quantity": 1000 }));
    assert_eq!(snapshot.data_revision(), ready.view().data_revision());
    assert_eq!(snapshot.definition_fingerprint(), fingerprint);
}

#[test]
fn over_limit_external_batches_are_rejected_without_partial_mutation() {
    let definition = test_definition();
    let mut form = definition
        .form(json!({ "name": "Ada", "quantity": 1 }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(2).unwrap(),
            NonZeroUsize::new(1024).unwrap(),
        ))
        .build()
        .expect("the form should be created");
    let revision = form.view().data_revision();
    let state_revision = form.view().state_revision();
    let findings = (0..3)
        .map(|index| {
            ExternalFinding::advisory(
                format!("finding-{index}"),
                JsonPointer::parse("/quantity").expect("the quantity pointer should be valid"),
                json!({ "index": index }),
            )
        })
        .collect::<Vec<_>>();

    let error = form
        .apply_external_findings(ExternalFindingBatch::new("server", revision, findings))
        .expect_err("the release-candidate external-finding limit should be enforced");

    assert!(matches!(
        error,
        ExternalFindingError::ResourceLimit(limit)
            if limit.dimension() == "active_external_findings"
                && limit.maximum() == 2
                && limit.observed() == 3
    ));
    assert_eq!(form.view().data_revision(), revision);
    assert_eq!(form.view().state_revision(), state_revision);
    assert!(external_summary(&form).is_empty());

    let mut byte_limited = definition
        .form(json!({ "name": "Ada", "quantity": 1 }))
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(10).unwrap(),
            NonZeroUsize::new(8).unwrap(),
        ))
        .build()
        .expect("the byte-limited form should be created");
    let byte_revision = byte_limited.view().data_revision();
    assert!(matches!(
        byte_limited.apply_external_findings(ExternalFindingBatch::new(
            "server",
            byte_revision,
            [external("too-large", "/quantity", false, json!({}))],
        )),
        Err(ExternalFindingError::ResourceLimit(limit))
            if limit.dimension() == "active_external_finding_bytes"
                && limit.maximum() == 8
                && limit.observed() > 8
    ));
}

fn form_with_visibility(policy: FindingVisibilityPolicy) -> Form {
    test_definition()
        .form(json!({ "name": "Ada", "quantity": 1 }))
        .finding_visibility(policy)
        .build()
        .expect("the form should be created")
}

fn test_definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "quantity"],
        "properties": {
            "name": { "type": "string", "minLength": 1 },
            "quantity": { "type": "integer" }
        }
    }))
    .expect("the data schema should compile")
}

fn external(
    code: &str,
    pointer: &str,
    blocking: bool,
    parameters: serde_json::Value,
) -> ExternalFinding {
    let pointer = JsonPointer::parse(pointer).expect("the test pointer should be valid");
    if blocking {
        ExternalFinding::blocking(code, pointer, parameters)
    } else {
        ExternalFinding::advisory(code, pointer, parameters)
    }
}

fn external_summary(form: &Form) -> Vec<(String, String, String, bool, serde_json::Value)> {
    form.view()
        .visible_findings()
        .filter_map(|finding| match finding {
            FindingView::External {
                source, finding, ..
            } => Some((
                source.to_owned(),
                finding.instance_location().as_str().to_owned(),
                finding.code().to_owned(),
                finding.is_blocking(),
                finding.parameters().clone(),
            )),
            _ => None,
        })
        .collect()
}

fn visible_kinds(form: &Form) -> Vec<&'static str> {
    form.view()
        .visible_findings()
        .map(|finding| match finding {
            FindingView::Validation { .. } => "validation",
            FindingView::ValidationFindingsTruncated { .. } => "validation-truncated",
            FindingView::Indeterminate { .. } => "indeterminate",
            FindingView::Capability { .. } => "capability",
            FindingView::External { .. } => "external",
            FindingView::Parse { .. } => "parse",
            _ => "unknown",
        })
        .collect()
}

fn blocker_kinds<'a>(blockers: impl Iterator<Item = &'a SubmissionBlocker>) -> Vec<&'static str> {
    blockers
        .map(|blocker| match blocker {
            SubmissionBlocker::Parse { .. } => "parse",
            SubmissionBlocker::Validation(_) => "validation",
            SubmissionBlocker::ValidationFindingsTruncated { .. } => "validation-truncated",
            SubmissionBlocker::Indeterminate(_) => "indeterminate",
            SubmissionBlocker::Capability(_) => "capability",
            SubmissionBlocker::External { .. } => "external",
            _ => "unknown",
        })
        .collect()
}

fn another_form_revision() -> schemaform::DataRevision {
    form_with_visibility(FindingVisibilityPolicy::default())
        .view()
        .data_revision()
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
