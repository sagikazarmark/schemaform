use schemaform::{
    CapabilitySeverity, FindingView, FormDefinition, JsonPointer, RetrievalUri, SubmissionOutcome,
    form::{SubmissionBlocker, ValidationOutcomeView},
};
use serde_json::{Value, json};

const ROOT_URI: &str = "urn:schemaform:test:open-object";

fn compiler(additional_properties: Option<bool>) -> schemaform::FormCompiler {
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Name" }
        }
    });
    if let Some(additional_properties) = additional_properties {
        schema
            .as_object_mut()
            .expect("the test schema should be an object")
            .insert(
                "additionalProperties".to_owned(),
                Value::Bool(additional_properties),
            );
    }
    FormDefinition::compiler(schema)
        .root_uri(RetrievalUri::parse(ROOT_URI).expect("the test retrieval URI should be valid"))
}

#[test]
fn open_fixed_objects_compile_with_deterministic_nonblocking_warnings() {
    for (additional_properties, expected_pointer, implicit) in [
        (Some(true), "/additionalProperties", false),
        (None, "", true),
    ] {
        let definition = compiler(additional_properties)
            .compile()
            .expect("an open fixed-object projection should compile in strict mode");
        let analysis = compiler(additional_properties)
            .analyze()
            .expect("lenient analysis should report the same warning");
        let findings = definition.capability_findings().collect::<Vec<_>>();

        assert_eq!(
            findings,
            analysis.capability_report().findings().collect::<Vec<_>>()
        );
        assert_eq!(findings.len(), 1);
        let finding = findings[0];
        assert_eq!(finding.code(), "applicator.additional-properties.open");
        assert_eq!(finding.instance_location().as_str(), "");
        assert_eq!(finding.keyword_location().resource().as_str(), ROOT_URI);
        assert_eq!(
            finding.keyword_location().pointer().as_str(),
            expected_pointer
        );
        assert_eq!(finding.parameters(), &json!({ "implicit": implicit }));
        assert_eq!(finding.severity(), CapabilitySeverity::Warning);
        assert!(!finding.is_blocking());
        assert!(!analysis.capability_report().is_blocking());

        let repeated = compiler(additional_properties)
            .compile()
            .expect("repeated open-object compilation should succeed");
        assert_eq!(
            definition.capability_findings().collect::<Vec<_>>(),
            repeated.capability_findings().collect::<Vec<_>>()
        );
        assert_eq!(definition.fingerprint(), repeated.fingerprint());
    }

    let closed = compiler(Some(false))
        .compile()
        .expect("a closed fixed object should compile");
    assert_eq!(closed.capability_findings().count(), 0);
}

#[test]
fn undeclared_form_data_survives_edits_lifecycle_operations_and_submission() {
    let definition = compiler(Some(true))
        .compile()
        .expect("the open fixed object should compile");
    let baseline: Value = serde_json::from_str(
        r#"{
            "name": "Ada",
            "metadata": {"source": "import", "verified": true},
            "tags": ["math", 7],
            "exact": 12345678901234567890.00000000000000000001
        }"#,
    )
    .expect("the arbitrary-precision baseline should parse");
    let mut form = definition
        .create_form(baseline.clone())
        .expect("the complete open-object form data should instantiate");
    let name = form
        .node(form.view().root())
        .expect("the form root should exist")
        .children()
        .next()
        .expect("the declared name control should exist");

    form.user()
        .input_text(name, "Grace")
        .expect("the declared property should remain editable");
    assert_eq!(form.form_data()["name"], "Grace");
    assert_eq!(form.form_data()["metadata"], baseline["metadata"]);
    assert_eq!(form.form_data()["tags"], baseline["tags"]);
    assert_eq!(form.form_data()["exact"], baseline["exact"]);

    form.transact(|draft| {
        draft.set(
            &JsonPointer::parse("/name").expect("the declared pointer should be valid"),
            json!("Katherine"),
        );
    })
    .expect("a host update should preserve undeclared siblings");
    assert_eq!(form.form_data()["metadata"], baseline["metadata"]);
    assert_eq!(form.form_data()["exact"], baseline["exact"]);

    form.reset();
    assert_eq!(form.form_data(), &baseline);

    let reinitialized = json!({
        "name": "Dorothy",
        "opaque": { "version": 2, "payload": [null, false] }
    });
    form.reinitialize(reinitialized.clone())
        .expect("reinitialization should accept complete open-object form data");
    form.user()
        .input_text(name, "Dorothy Vaughan")
        .expect("the declared property should remain editable after reinitialization");
    assert_eq!(form.form_data()["opaque"], reinitialized["opaque"]);
    form.reset();
    assert_eq!(form.form_data(), &reinitialized);

    let replaced = json!({
        "name": "Mary",
        "hostOwned": { "preserved": "byte-for-value" }
    });
    form.transact(|draft| draft.replace_all(replaced.clone()))
        .expect("host replacement should retain every supplied undeclared member");
    assert!(matches!(
        form.view().visible_findings().collect::<Vec<_>>().as_slice(),
        [FindingView::Capability { finding, .. }]
            if finding.code() == "applicator.additional-properties.open"
                && !finding.is_blocking()
    ));
    let preparation = form.prepare_submission();
    let snapshot = match preparation.outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot,
        SubmissionOutcome::Blocked(_) => panic!("a capability warning must not block submission"),
    };
    assert_eq!(snapshot.form_data(), &replaced);
}

#[test]
fn stock_validation_remains_authoritative_for_undeclared_members() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": true,
        "propertyNames": { "pattern": "^[a-z]+$" },
        "required": ["name"],
        "properties": {
            "name": { "type": "string" }
        }
    }))
    .expect("the open fixed-object projection should compile");
    let mut form = definition
        .create_form(json!({ "name": "Ada", "HostOwned": true }))
        .expect("schema-invalid open-object data should remain repairable");

    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "propertyNames")
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
                && !blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::Capability(_)
                ))
    ));

    form.transact(|draft| draft.replace_all(json!({ "name": "Ada", "hostowned": true })))
        .expect("the host should be able to repair an undeclared member");
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(_)
    ));
}
