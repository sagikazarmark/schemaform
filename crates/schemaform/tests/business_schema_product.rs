use std::collections::HashMap;

use schemaform::{
    CompileError, FormDefinition, SubmissionOutcome,
    definition::{DefinitionNodeKind, SemanticKind},
    form::SubmissionBlocker,
};
use serde_json::json;

#[path = "../../../testing/fixtures/business-schemas/product_cases.rs"]
mod product_cases;

#[test]
fn every_business_schema_fixture_has_its_expected_product_outcome() {
    let fixtures = product_cases::fixtures();
    assert_eq!(fixtures.len(), 20);

    for fixture in fixtures {
        let analysis = fixture
            .compiler()
            .analyze()
            .unwrap_or_else(|error| panic!("fixture {} should qualify: {error}", fixture.id));
        let actual_findings = finding_keys(analysis.capability_report().findings());
        assert_eq!(
            actual_findings, fixture.expected_findings,
            "fixture {} capability outcome drifted",
            fixture.id
        );

        if fixture.is_in_profile() {
            let definition = fixture.compiler().compile().unwrap_or_else(|error| {
                panic!("in-profile fixture {} should compile: {error}", fixture.id)
            });
            assert_eq!(
                finding_keys(definition.capability_findings()),
                fixture.expected_findings,
                "fixture {} strict warnings drifted",
                fixture.id
            );
            assert_expected_controls(&definition, &fixture);
            let mut form = definition.create_form(json!({})).unwrap_or_else(|error| {
                panic!("in-profile fixture {} should execute: {error}", fixture.id)
            });
            assert_eq!(form.form_data(), &json!({}));
            assert!(
                match form.prepare_submission().outcome() {
                    SubmissionOutcome::Ready(_) => true,
                    SubmissionOutcome::Blocked(blockers) => blockers
                        .iter()
                        .all(|blocker| !matches!(blocker, SubmissionBlocker::Capability(_))),
                },
                "in-profile fixture {} should have no capability submission blocker",
                fixture.id
            );
        } else {
            let strict = match fixture.compiler().compile() {
                Err(CompileError::Capability(report)) => report,
                Err(error) => panic!(
                    "out-of-profile fixture {} returned the wrong strict error: {error}",
                    fixture.id
                ),
                Ok(_) => panic!(
                    "out-of-profile fixture {} should fail strict compilation",
                    fixture.id
                ),
            };
            assert_eq!(strict, *analysis.capability_report());
            assert!(has_unsupported_node(analysis.definition()));

            let definition = analysis.into_parts().0;
            let mut form = definition.create_form(json!({})).unwrap_or_else(|error| {
                panic!(
                    "out-of-profile fixture {} should execute leniently: {error}",
                    fixture.id
                )
            });
            assert_eq!(form.form_data(), &json!({}));
            assert!(matches!(
                form.prepare_submission().outcome(),
                SubmissionOutcome::Blocked(blockers)
                    if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Capability(_)))
            ));
            assert_eq!(form.form_data(), &json!({}));
        }
    }
}

fn assert_expected_controls(
    definition: &FormDefinition,
    fixture: &product_cases::BusinessSchemaFixture,
) {
    let mut actual = HashMap::new();
    collect_controls(definition, definition.root(), None, &mut actual);
    for expected in &fixture.expected_controls {
        let expected_kind = match expected.kind.as_str() {
            "string" | "nullable-string" | "sensitive-string" => SemanticKind::String,
            "number" => SemanticKind::Number,
            "integer" => SemanticKind::Integer,
            "boolean" => SemanticKind::Boolean,
            "choice" => SemanticKind::Choice,
            "homogeneous-array" => SemanticKind::HomogeneousArray,
            kind => panic!(
                "fixture {} has unknown expected control kind {kind}",
                fixture.id
            ),
        };
        assert_eq!(
            actual.get(expected.binding.as_str()),
            Some(&expected_kind),
            "fixture {} should project control {} as {}",
            fixture.id,
            expected.binding,
            expected.kind
        );
    }
}

fn collect_controls(
    definition: &FormDefinition,
    identity: schemaform::DefinitionNodeId,
    template_prefix: Option<&str>,
    controls: &mut HashMap<String, SemanticKind>,
) {
    let node = definition
        .node(identity)
        .expect("definition nodes should remain addressable");
    let binding = node.binding().map(|binding| {
        template_prefix.map_or_else(
            || binding.as_str().to_owned(),
            |prefix| format!("{prefix}{}", binding.as_str()),
        )
    });
    if node.kind() == DefinitionNodeKind::Control
        && let (Some(binding), Some(kind)) = (&binding, node.semantic_kind())
    {
        assert!(
            controls.insert(binding.clone(), kind).is_none(),
            "definition controls should have unique projected bindings"
        );
    }
    let array_prefix = if node.semantic_kind() == Some(SemanticKind::HomogeneousArray) {
        Some(format!(
            "{}/0",
            binding.expect("array controls should have a binding")
        ))
    } else {
        template_prefix.map(str::to_owned)
    };
    for child in node.children() {
        collect_controls(definition, child, array_prefix.as_deref(), controls);
    }
}

fn finding_keys<'a>(
    findings: impl Iterator<Item = &'a schemaform::CapabilityFinding>,
) -> Vec<product_cases::ExpectedCapabilityFinding> {
    findings
        .map(|finding| product_cases::ExpectedCapabilityFinding {
            code: finding.code().to_owned(),
            instance_location: finding.instance_location().as_str().to_owned(),
            resource_uri: finding.keyword_location().resource().as_str().to_owned(),
            keyword_pointer: finding.keyword_location().pointer().as_str().to_owned(),
            parameters: finding.parameters().clone(),
            blocking: finding.is_blocking(),
        })
        .collect()
}

fn has_unsupported_node(definition: &FormDefinition) -> bool {
    let mut pending = vec![definition.root()];
    while let Some(identity) = pending.pop() {
        let node = definition
            .node(identity)
            .expect("definition nodes should remain addressable");
        if node.kind() == DefinitionNodeKind::Unsupported {
            return true;
        }
        pending.extend(node.children());
    }
    false
}
