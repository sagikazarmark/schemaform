use schemaform::{
    CompileError, FindingView, FormDefinition, JsonPointer, RetrievalUri, SubmissionOutcome,
    definition::DefinitionNodeKind,
    form::{AllowedOperations, SubmissionBlocker, UserOperationError, ValidationOutcomeView},
};
use serde_json::{Value, json};

fn schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["contact", "name"],
        "properties": {
            "contact": {
                "title": "Contact",
                "oneOf": [
                    { "type": "string" },
                    { "type": "integer", "minimum": 1 }
                ]
            },
            "name": { "type": "string", "title": "Name" }
        }
    })
}

fn compiler() -> schemaform::FormCompiler {
    FormDefinition::compiler(schema()).root_uri(
        RetrievalUri::parse("urn:schemaform:test:unsupported-one-of")
            .expect("the root retrieval URI should be valid"),
    )
}

#[test]
fn strict_and_lenient_compilation_report_the_same_located_one_of_capability() {
    let strict_report = match compiler().compile() {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("strict compilation returned the wrong error: {error}"),
        Ok(_) => panic!("strict compilation must reject the unsupported region"),
    };
    let analysis = compiler()
        .analyze()
        .expect("lenient analysis should retain an explicit unsupported region");

    assert_eq!(&strict_report, analysis.capability_report());
    let finding = strict_report
        .findings()
        .next()
        .expect("one capability finding should be reported");
    assert_eq!(strict_report.findings().count(), 1);
    assert_eq!(finding.code(), "applicator.one-of");
    assert_eq!(finding.instance_location().as_str(), "/contact");
    assert_eq!(
        finding.keyword_location().resource().as_str(),
        "urn:schemaform:test:unsupported-one-of"
    );
    assert_eq!(
        finding.keyword_location().pointer().as_str(),
        "/properties/contact/oneOf"
    );
    assert_eq!(finding.parameters(), &json!({ "branchCount": 2 }));
    assert!(finding.is_blocking());

    let definition = analysis.definition();
    assert_eq!(
        definition.capability_findings().collect::<Vec<_>>(),
        strict_report.findings().collect::<Vec<_>>()
    );
    let children = definition
        .node(definition.root())
        .expect("the definition root should exist")
        .children()
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    let contact = definition
        .node(children[0])
        .expect("the unsupported region should exist");
    assert_eq!(contact.kind(), DefinitionNodeKind::Unsupported);
    assert_eq!(contact.binding().map(JsonPointer::as_str), Some("/contact"));
    assert_eq!(contact.label(), "Contact");
    assert_eq!(contact.semantic_kind(), None);
    let name = definition
        .node(children[1])
        .expect("the supported control should exist");
    assert_eq!(name.kind(), DefinitionNodeKind::Control);
    assert_eq!(name.binding().map(JsonPointer::as_str), Some("/name"));

    let repeated = compiler()
        .analyze()
        .expect("repeated lenient analysis should succeed");
    assert_eq!(analysis.capability_report(), repeated.capability_report());
    assert_eq!(
        definition.fingerprint(),
        repeated.definition().fingerprint()
    );
}

#[test]
fn unsupported_region_is_visible_and_blocks_submission_without_skipping_validation() {
    let definition = compiler()
        .analyze()
        .expect("lenient analysis should succeed")
        .into_parts()
        .0;
    let mut form = definition
        .create_form(json!({ "contact": "ada@example.test", "name": "Ada" }))
        .expect("valid form data should instantiate the lenient definition");
    let contact = form
        .node(form.view().root())
        .expect("the form root should exist")
        .children()
        .next()
        .expect("the unsupported region should be instantiated");

    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    assert_eq!(
        form.node(contact)
            .expect("the unsupported region should exist")
            .allowed_operations(),
        AllowedOperations::default()
    );
    assert!(matches!(
        form.user().input_text(contact, "replacement"),
        Err(UserOperationError::OperationNotAllowed)
    ));
    let view = form.view();
    let visible = view.visible_findings().collect::<Vec<_>>();
    assert!(matches!(
        visible.as_slice(),
        [FindingView::Capability { finding, .. }] if finding.code() == "applicator.one-of"
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Capability(finding)
                    if finding.code() == "applicator.one-of"
            ))
    ));

    form.transact(|draft| {
        draft.set(
            &JsonPointer::parse("/contact").expect("the pointer should be valid"),
            json!(true),
        );
    })
    .expect("the host should be able to preserve schema-invalid form data");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "oneOf")
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
                && blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::Capability(finding)
                        if finding.code() == "applicator.one-of"
                ))
    ));
}

#[test]
fn lenient_analysis_retains_one_of_findings_beside_other_unsupported_regions() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "schemas/root",
        "type": "object",
        "properties": {
            "contact": {
                "$id": "contact",
                "oneOf": [{ "type": "string" }, { "type": "integer" }]
            },
            "metadata": true
        }
    });
    let compiler = || {
        FormDefinition::compiler(schema.clone()).root_uri(
            RetrievalUri::parse("https://retrieval.example.test/base/root.json")
                .expect("the root retrieval URI should be valid"),
        )
    };
    let strict_report = match compiler().compile() {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("strict compilation returned the wrong error: {error}"),
        Ok(_) => panic!("strict compilation must report every unsupported region"),
    };
    let analysis = compiler()
        .analyze()
        .expect("other unsupported properties must not discard the lenient definition");

    assert_eq!(&strict_report, analysis.capability_report());
    let findings = strict_report
        .findings()
        .filter(|finding| finding.is_blocking())
        .collect::<Vec<_>>();
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].code(), "applicator.one-of");
    assert_eq!(
        findings[0].keyword_location().resource().as_str(),
        "https://retrieval.example.test/base/schemas/contact"
    );
    assert_eq!(findings[0].keyword_location().pointer().as_str(), "/oneOf");
    assert_eq!(findings[1].code(), "core.boolean.unconstrained");
    assert_eq!(findings[1].instance_location().as_str(), "/metadata");
    assert_eq!(
        analysis
            .definition()
            .node(analysis.definition().root())
            .expect("the root should exist")
            .children()
            .filter(|id| {
                analysis
                    .definition()
                    .node(*id)
                    .is_some_and(|node| node.kind() == DefinitionNodeKind::Unsupported)
            })
            .count(),
        2
    );
}
