use schemaform::{
    FormDefinition, RetrievalUri, SubmissionOutcome,
    definition::SemanticKind,
    form::{ParseBlockerKind, SubmissionBlocker, ValidationOutcomeView},
};

#[test]
fn arbitrary_precision_decimal_edits_preserve_exact_state_through_submission() {
    let definition = FormDefinition::compiler(
        serde_json::from_str(
            r#"{
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "required": ["rate"],
                "properties": {
                    "rate": {
                        "type": "number",
                        "title": "Rate",
                        "minimum": 0.1000000000000000000000000000000000000001
                    }
                }
            }"#,
        )
        .expect("the trusted data schema should parse"),
    )
    .root_uri(
        RetrievalUri::parse("urn:schemaform:test:decimal-minimum")
            .expect("the root retrieval URI should be valid"),
    )
    .compile()
    .expect("the trusted data schema should compile");
    let baseline_data: serde_json::Value =
        serde_json::from_str(r#"{"rate":0.1000000000000000000000000000000000000001}"#)
            .expect("the arbitrary-precision baseline should parse");
    let mut form = definition
        .create_form(baseline_data.clone())
        .expect("the decimal form should be created");
    let rate = form
        .node(form.view().root())
        .expect("the form root should exist")
        .children()
        .next()
        .expect("the generated number control should be instantiated");

    let before = form
        .node(rate)
        .expect("the generated number control should exist");
    assert_eq!(
        before.definition().semantic_kind(),
        Some(SemanticKind::Number)
    );
    assert_eq!(
        before.display_text().as_deref(),
        Some("0.1000000000000000000000000000000000000001")
    );
    assert!(!before.is_dirty());

    form.user()
        .input_text(rate, "1.000000000000000000000000000000000000001e-1")
        .expect("an equivalent decimal spelling should parse");
    let equivalent = form
        .node(rate)
        .expect("the generated number control should exist");
    assert_eq!(
        equivalent.edit_buffer(),
        Some("1.000000000000000000000000000000000000001e-1")
    );
    assert_eq!(equivalent.parse_blocker(), None);
    assert!(!equivalent.is_dirty());
    assert_eq!(form.form_data(), &baseline_data);

    form.user()
        .input_text(rate, "1e-")
        .expect("the incomplete decimal should remain buffered");
    let blocked = form
        .node(rate)
        .expect("the generated number control should exist");
    assert_eq!(blocked.edit_buffer(), Some("1e-"));
    assert_eq!(
        blocked.parse_blocker(),
        Some(ParseBlockerKind::InvalidNumber)
    );
    assert!(!blocked.is_dirty());
    assert_eq!(form.form_data(), &baseline_data);
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Parse {
                    target,
                    kind: ParseBlockerKind::InvalidNumber,
                } if *target == rate
            ))
    ));
    form.user()
        .blur(rate)
        .expect("blurring an incomplete decimal should preserve its buffer");
    let blurred_incomplete = form
        .node(rate)
        .expect("the generated number control should exist");
    assert_eq!(blurred_incomplete.edit_buffer(), Some("1e-"));
    assert_eq!(
        blurred_incomplete.parse_blocker(),
        Some(ParseBlockerKind::InvalidNumber)
    );
    assert!(blurred_incomplete.is_touched());
    assert_eq!(form.form_data(), &baseline_data);

    form.user()
        .input_text(rate, "0.10000000000000000000000000000000000000009")
        .expect("the below-minimum decimal should remain editable");
    let invalid_data: serde_json::Value =
        serde_json::from_str(r#"{"rate":0.10000000000000000000000000000000000000009}"#)
            .expect("the below-minimum form data should parse");
    assert_eq!(form.form_data(), &invalid_data);
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, truncated: false }
            if findings.len() == 1
                && findings[0].code() == "minimum"
                && findings[0].instance_location().as_str() == "/rate"
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));

    form.user()
        .input_text(rate, "0.10000000000000000000000000000000000000011")
        .expect("the corrected arbitrary-precision decimal should parse");
    let corrected_data: serde_json::Value =
        serde_json::from_str(r#"{"rate":0.10000000000000000000000000000000000000011}"#)
            .expect("the corrected arbitrary-precision form data should parse");
    assert_eq!(form.form_data(), &corrected_data);
    let corrected = form
        .node(rate)
        .expect("the generated number control should exist");
    assert_eq!(corrected.parse_blocker(), None);
    assert!(corrected.is_dirty());
    assert!(corrected.is_touched());

    form.user()
        .blur(rate)
        .expect("blurring a valid decimal should finalize its buffer");
    let blurred = form
        .node(rate)
        .expect("the generated number control should exist");
    assert_eq!(blurred.edit_buffer(), None);
    assert!(blurred.is_touched());

    let submitted = form.prepare_submission();
    let snapshot = match submitted.outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot,
        SubmissionOutcome::Blocked(_) => panic!("the corrected decimal should be submittable"),
    };
    assert_eq!(snapshot.form_data(), &corrected_data);
    assert_eq!(
        serde_json::to_string(snapshot.form_data()).expect("the snapshot should serialize"),
        r#"{"rate":0.10000000000000000000000000000000000000011}"#
    );
}
