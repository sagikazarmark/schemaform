use schemaform::{
    CapabilitySeverity, CompileError, FindingVisibility, FindingVisibilityPolicy, FormDefinition,
    JsonPointer, SubmissionOutcome,
    definition::{DefinitionNodeKind, SemanticKind},
    form::{SubmissionBlocker, ValidationOutcomeView},
};
use serde_json::{Value, json};

fn compatible_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:compatible-all-of",
        "type": "object",
        "allOf": [
            {
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "title": "Name" }
                }
            },
            {
                "properties": {
                    "name": { "minLength": 3 }
                }
            }
        ]
    })
}

#[test]
fn compatible_all_of_compiles_one_deterministic_control_with_every_source_location() {
    let definition = FormDefinition::compile(compatible_schema())
        .expect("compatible allOf subschemas should compile");
    let repeated = FormDefinition::compile(compatible_schema())
        .expect("recompiling the composition should succeed");
    assert_eq!(definition.fingerprint(), repeated.fingerprint());

    let root = definition
        .node(definition.root())
        .expect("the generated root should exist");
    assert_eq!(
        root.schema_locations()
            .map(|location| (location.resource().as_str(), location.pointer().as_str()))
            .collect::<Vec<_>>(),
        [
            ("urn:schemaform:test:compatible-all-of", ""),
            ("urn:schemaform:test:compatible-all-of", "/allOf/0"),
            ("urn:schemaform:test:compatible-all-of", "/allOf/1"),
        ]
    );
    let name = root
        .children()
        .next()
        .and_then(|identity| definition.node(identity))
        .expect("the composed string control should exist");
    assert_eq!(name.kind(), DefinitionNodeKind::Control);
    assert_eq!(name.binding().map(JsonPointer::as_str), Some("/name"));
    assert_eq!(name.label(), "Name");
    assert!(name.is_required());
    assert_eq!(
        name.schema_locations()
            .map(|location| (location.resource().as_str(), location.pointer().as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "urn:schemaform:test:compatible-all-of",
                "/allOf/0/properties/name",
            ),
            (
                "urn:schemaform:test:compatible-all-of",
                "/allOf/1/properties/name",
            ),
        ]
    );
}

#[test]
fn compatible_all_of_edits_validates_and_submits_through_the_public_facade() {
    let definition = FormDefinition::compile(compatible_schema())
        .expect("compatible allOf subschemas should compile");
    let mut form = definition
        .form(json!({ "name": "Ada" }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::TouchedOrSubmission,
        ))
        .build()
        .expect("the composed form should be created");
    let name = form
        .node(form.view().root())
        .expect("the form root should exist")
        .children()
        .next()
        .expect("the composed control should be instantiated");

    form.user()
        .input_text(name, "Li")
        .expect("the composed control should accept input");
    assert_eq!(form.form_data(), &json!({ "name": "Li" }));
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| {
                finding.code() == "minLength"
                    && finding.instance_location().as_str() == "/name"
            })
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));

    form.user()
        .input_text(name, "Grace")
        .expect("the composed control should accept a correction");
    let prepared = form.prepare_submission();
    let snapshot = match prepared.outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot,
        SubmissionOutcome::Blocked(_) => panic!("the corrected composed form should submit"),
    };
    assert_eq!(snapshot.form_data(), &json!({ "name": "Grace" }));
}

#[test]
fn incompatible_all_of_kinds_are_a_located_capability_finding() {
    assert_all_of_ambiguity(
        json!([
            { "type": "string" },
            { "type": "integer" }
        ]),
        "incompatible-kind",
    );
}

#[test]
fn applicable_type_declarations_are_intersected_before_projecting_a_control() {
    for (branches, expected_kind, accepts_null) in [
        (
            json!([{ "type": ["string", "null"] }, { "type": "null" }]),
            SemanticKind::Null,
            true,
        ),
        (
            json!([{ "type": ["string", "null"] }, { "type": "string" }]),
            SemanticKind::String,
            false,
        ),
        (
            json!([{ "type": ["number", "null"] }, { "type": "integer" }]),
            SemanticKind::Integer,
            false,
        ),
        (
            json!([{ "type": "number" }, { "type": ["integer", "null"] }]),
            SemanticKind::Integer,
            false,
        ),
    ] {
        let definition = FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "allOf": branches } }
        }))
        .expect("a nonempty type intersection should compile");
        let value = definition
            .node(definition.root())
            .and_then(|root| root.children().next())
            .and_then(|identity| definition.node(identity))
            .expect("the intersected control should exist");

        assert_eq!(value.semantic_kind(), Some(expected_kind));
        assert_eq!(value.accepts_null(), accepts_null);
    }
}

#[test]
fn conflicting_all_of_titles_fall_back_with_a_located_nonblocking_warning() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:ambiguous-all-of",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "value": {
                "allOf": [
                    { "type": "string", "title": "Text" },
                    { "type": "string", "title": "Count" }
                ]
            }
        }
    }))
    .expect("conflicting presentation annotations must not block editing");
    let value = definition
        .node(definition.root())
        .and_then(|root| root.children().next())
        .and_then(|identity| definition.node(identity))
        .expect("the string control should remain represented");
    let finding = definition
        .capability_findings()
        .next()
        .expect("the annotation warning should exist");

    assert_eq!(value.kind(), DefinitionNodeKind::Control);
    assert_eq!(value.label(), "value");
    assert_eq!(finding.code(), "annotation.conflict");
    assert_eq!(finding.instance_location().as_str(), "/value");
    assert_eq!(
        finding.keyword_location().pointer().as_str(),
        "/properties/value/allOf/0/title"
    );
    assert_eq!(finding.severity(), CapabilitySeverity::Warning);
    assert_eq!(
        finding.parameters(),
        &json!({ "keyword": "title", "values": ["Count", "Text"] })
    );
}

#[test]
fn root_all_of_kind_conflict_is_explicit_in_strict_and_lenient_modes() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:ambiguous-root-all-of",
        "allOf": [
            { "type": "object" },
            { "type": "string" }
        ]
    });
    let compiler = || FormDefinition::compiler(schema.clone());
    let strict_report = match compiler().compile() {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("strict compilation returned the wrong error: {error}"),
        Ok(_) => panic!("strict compilation must reject the ambiguous root"),
    };
    let analysis = compiler()
        .analyze()
        .expect("lenient analysis should retain the ambiguous root");

    assert_eq!(&strict_report, analysis.capability_report());
    let finding = strict_report
        .findings()
        .next()
        .expect("the root capability finding should exist");
    assert_eq!(finding.code(), "applicator.all-of.ambiguous");
    assert_eq!(finding.instance_location().as_str(), "");
    assert_eq!(finding.keyword_location().pointer().as_str(), "/allOf");
    assert_eq!(
        finding.parameters(),
        &json!({ "branchCount": 2, "reason": "incompatible-kind" })
    );
    let unsupported = analysis
        .definition()
        .node(analysis.definition().root())
        .expect("the generated root should exist")
        .children()
        .next()
        .and_then(|identity| analysis.definition().node(identity))
        .expect("lenient analysis should retain an unsupported root region");
    assert_eq!(unsupported.kind(), DefinitionNodeKind::Unsupported);
    assert_eq!(unsupported.binding().map(JsonPointer::as_str), Some(""));
}

#[test]
fn root_all_of_reports_kind_and_title_conflicts_together() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:multiple-root-all-of-conflicts",
        "allOf": [
            { "type": "object", "title": "Object" },
            { "type": "string", "title": "Text" }
        ]
    });
    let analysis = FormDefinition::compiler(schema)
        .analyze()
        .expect("lenient analysis should retain every root conflict");
    let reasons = analysis
        .capability_report()
        .findings()
        .filter(|finding| finding.code() == "applicator.all-of.ambiguous")
        .map(|finding| finding.parameters()["reason"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(reasons, ["incompatible-kind"]);
    let title_warning = analysis
        .capability_report()
        .findings()
        .find(|finding| finding.code() == "annotation.conflict")
        .expect("the root title conflict should remain visible");
    assert_eq!(title_warning.severity(), CapabilitySeverity::Warning);
    assert!(
        analysis
            .capability_report()
            .findings()
            .any(|finding| finding.code() == "validation.type.ambiguous")
    );
}

#[test]
fn object_root_conflict_does_not_hide_property_blockers() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:root-and-property-conflicts",
        "type": "object",
        "allOf": [
            { "type": "object", "title": "First" },
            { "type": "object", "title": "Second" }
        ],
        "properties": {
            "value": {
                "oneOf": [
                    { "type": "string" },
                    { "type": "integer" }
                ]
            }
        }
    });
    let analysis = FormDefinition::compiler(schema)
        .analyze()
        .expect("lenient analysis should collect root and property blockers");
    let findings = analysis
        .capability_report()
        .findings()
        .filter(|finding| finding.is_blocking())
        .map(|finding| (finding.code(), finding.instance_location().as_str()))
        .collect::<Vec<_>>();

    assert_eq!(findings, [("applicator.one-of", "/value")]);
    assert!(analysis.capability_report().findings().any(|finding| {
        finding.code() == "annotation.conflict"
            && finding.instance_location().as_str().is_empty()
            && !finding.is_blocking()
    }));
}

#[test]
fn nested_all_of_conflict_reports_the_most_specific_composition() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:nested-ambiguous-all-of",
        "type": "object",
        "properties": {
            "value": {
                "allOf": [
                    {
                        "allOf": [
                            { "type": "string" },
                            { "type": "integer" }
                        ]
                    },
                    {}
                ]
            }
        }
    });
    let analysis = FormDefinition::compiler(schema)
        .analyze()
        .expect("lenient analysis should retain the nested conflict");
    let finding = analysis
        .capability_report()
        .findings()
        .find(|finding| finding.is_blocking())
        .expect("the nested capability finding should exist");

    assert_eq!(finding.code(), "applicator.all-of.ambiguous");
    assert_eq!(
        finding.keyword_location().pointer().as_str(),
        "/properties/value/allOf/0/allOf"
    );
    assert_eq!(
        finding.parameters(),
        &json!({ "branchCount": 2, "reason": "incompatible-kind" })
    );
}

#[test]
fn independent_nested_all_of_conflicts_are_all_reported() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:independent-ambiguous-all-of",
        "type": "object",
        "properties": {
            "value": {
                "allOf": [
                    {
                        "allOf": [
                            { "type": "string" },
                            { "type": "integer" }
                        ]
                    },
                    {
                        "allOf": [
                            { "type": "string" },
                            { "type": "object" }
                        ]
                    }
                ]
            }
        }
    });
    let analysis = FormDefinition::compiler(schema)
        .analyze()
        .expect("lenient analysis should retain both nested conflicts");
    let locations = analysis
        .capability_report()
        .findings()
        .filter(|finding| finding.is_blocking())
        .map(|finding| finding.keyword_location().pointer().as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        locations,
        [
            "/properties/value/allOf/0/allOf",
            "/properties/value/allOf/1/allOf",
        ]
    );
}

#[test]
fn empty_all_of_does_not_claim_an_independent_kind_conflict() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:unrelated-all-of",
        "type": "object",
        "properties": {
            "value": {
                "$ref": "#/$defs/string-value",
                "type": "integer",
                "allOf": [{}]
            }
        },
        "$defs": {
            "string-value": { "type": "string" }
        }
    });
    let analysis = FormDefinition::compiler(schema)
        .analyze()
        .expect("lenient analysis should retain the unsupported control");
    let finding = analysis
        .capability_report()
        .findings()
        .find(|finding| finding.is_blocking())
        .expect("the capability finding should exist");

    assert_eq!(finding.code(), "validation.type.ambiguous");
}

#[test]
fn number_and_integer_conjunction_projects_the_narrower_integer_kind() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "quantity": {
                "allOf": [
                    { "type": "number" },
                    { "type": "integer" }
                ]
            }
        }
    }))
    .expect("the compatible numeric kinds should compile");
    let quantity = definition
        .node(
            definition
                .node(definition.root())
                .expect("the root definition should exist")
                .children()
                .next()
                .expect("the quantity control should exist"),
        )
        .expect("the quantity definition should exist");

    assert_eq!(quantity.semantic_kind(), Some(SemanticKind::Integer));
}

#[test]
fn lenient_analysis_reports_every_blocker_on_one_unsupported_region() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:multiple-composition-blockers",
        "type": "object",
        "properties": {
            "value": {
                "oneOf": [true, false],
                "allOf": [
                    { "type": "string", "title": "Text" },
                    { "type": "integer", "title": "Count" }
                ]
            }
        }
    });
    let analysis = FormDefinition::compiler(schema)
        .analyze()
        .expect("lenient analysis should retain every blocker");
    let findings = analysis
        .capability_report()
        .findings()
        .filter(|finding| finding.is_blocking())
        .map(|finding| (finding.code(), finding.parameters().clone()))
        .collect::<Vec<_>>();

    assert_eq!(
        findings,
        [
            ("applicator.one-of", json!({ "branchCount": 2 })),
            (
                "applicator.all-of.ambiguous",
                json!({ "branchCount": 2, "reason": "incompatible-kind" }),
            ),
        ]
    );
    assert!(analysis.capability_report().findings().any(|finding| {
        finding.code() == "annotation.conflict"
            && finding.parameters()["keyword"] == "title"
            && !finding.is_blocking()
    }));
    let children = analysis
        .definition()
        .node(analysis.definition().root())
        .expect("the generated root should exist")
        .children()
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    assert_eq!(
        analysis
            .definition()
            .node(children[0])
            .expect("the unsupported region should exist")
            .kind(),
        DefinitionNodeKind::Unsupported
    );
}

fn assert_all_of_ambiguity(all_of: Value, reason: &str) {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:ambiguous-all-of",
        "type": "object",
        "properties": {
            "value": {
                "allOf": all_of
            }
        }
    });
    let compiler = || FormDefinition::compiler(schema.clone());
    let strict_report = match compiler().compile() {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("strict compilation returned the wrong error: {error}"),
        Ok(_) => panic!("strict compilation must reject ambiguous allOf semantics"),
    };
    let analysis = compiler()
        .analyze()
        .expect("lenient analysis should retain the ambiguous region");

    assert_eq!(&strict_report, analysis.capability_report());
    let finding = strict_report
        .findings()
        .find(|finding| finding.is_blocking())
        .expect("one capability finding should be reported");
    assert_eq!(
        strict_report
            .findings()
            .filter(|finding| finding.is_blocking())
            .count(),
        1
    );
    assert_eq!(finding.code(), "applicator.all-of.ambiguous");
    assert_eq!(finding.instance_location().as_str(), "/value");
    assert_eq!(
        finding.keyword_location().resource().as_str(),
        "urn:schemaform:test:ambiguous-all-of"
    );
    assert_eq!(
        finding.keyword_location().pointer().as_str(),
        "/properties/value/allOf"
    );
    assert_eq!(
        finding.parameters(),
        &json!({ "branchCount": 2, "reason": reason })
    );

    let value = analysis
        .definition()
        .node(analysis.definition().root())
        .expect("the generated root should exist")
        .children()
        .next()
        .and_then(|identity| analysis.definition().node(identity))
        .expect("the ambiguous region should be explicit");
    assert_eq!(value.kind(), DefinitionNodeKind::Unsupported);
    assert_eq!(value.binding().map(JsonPointer::as_str), Some("/value"));
    assert_eq!(value.label(), "value");
}
