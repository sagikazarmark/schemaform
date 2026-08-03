use schemaform::{
    CapabilitySeverity, FormDefinition, JsonPointer, SubmissionOutcome,
    definition::DefinitionNodeView, form::ValidationOutcomeView,
};
use serde_json::json;

#[test]
fn title_and_description_are_fallbacks_with_deterministic_conflict_warnings() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:annotations",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "email": {
                "type": "string",
                "title": "Email address",
                "description": "Where account notices are sent."
            },
            "nickname": {
                "allOf": [
                    {
                        "type": "string",
                        "title": "Public name",
                        "description": "Shown to other users."
                    },
                    {
                        "title": "Nickname",
                        "description": "A short display name."
                    }
                ]
            }
        }
    });
    let compile = || {
        FormDefinition::compile(schema.clone())
            .expect("presentation annotation conflicts must not block editing")
    };
    let definition = compile();
    let email = node_with_binding(&definition, "/email");
    let nickname = node_with_binding(&definition, "/nickname");

    assert_eq!(email.label(), "Email address");
    assert_eq!(email.help(), Some("Where account notices are sent."));
    assert_eq!(nickname.label(), "nickname");
    assert_eq!(nickname.help(), None);

    let findings = definition.capability_findings().collect::<Vec<_>>();
    assert_eq!(findings.len(), 2);
    assert_eq!(
        findings
            .iter()
            .map(|finding| (
                finding.code(),
                finding.instance_location().as_str(),
                finding.keyword_location().pointer().as_str(),
                finding.parameters().clone(),
                finding.severity(),
            ))
            .collect::<Vec<_>>(),
        [
            (
                "annotation.conflict",
                "/nickname",
                "/properties/nickname/allOf/0/description",
                json!({
                    "keyword": "description",
                    "values": ["A short display name.", "Shown to other users."]
                }),
                CapabilitySeverity::Warning,
            ),
            (
                "annotation.conflict",
                "/nickname",
                "/properties/nickname/allOf/0/title",
                json!({ "keyword": "title", "values": ["Nickname", "Public name"] }),
                CapabilitySeverity::Warning,
            ),
        ]
    );
    assert!(findings.iter().all(|finding| !finding.is_blocking()));
    assert_eq!(
        findings,
        compile().capability_findings().collect::<Vec<_>>(),
        "annotation warnings must be stable across compilations"
    );
}

#[test]
fn nonasserting_annotations_neither_mutate_data_nor_block_submission() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "value": {
                "type": "string",
                "format": "email",
                "default": "default@example.test",
                "deprecated": true,
                "examples": ["example@example.test"],
                "contentEncoding": "base64",
                "contentMediaType": "text/plain",
                "contentSchema": { "type": "string" }
            }
        }
    }))
    .expect("nonasserting annotations should not block compilation");
    let mut form = definition
        .create_form(json!({ "value": "not an email or base64" }))
        .expect("annotated form data should remain constructible");
    let value = node_with_binding(&definition, "/value");
    let annotations = value.data_schema_annotations();

    assert_eq!(annotations.formats().collect::<Vec<_>>(), ["email"]);
    assert_eq!(
        annotations.defaults().collect::<Vec<_>>(),
        [&json!("default@example.test")]
    );
    assert!(annotations.is_deprecated());
    assert_eq!(
        annotations.examples().collect::<Vec<_>>(),
        [&json!("example@example.test")]
    );
    assert_eq!(
        annotations.content_encodings().collect::<Vec<_>>(),
        ["base64"]
    );
    assert_eq!(
        annotations.content_media_types().collect::<Vec<_>>(),
        ["text/plain"]
    );
    assert_eq!(
        annotations.content_schemas().collect::<Vec<_>>(),
        [&json!({ "type": "string" })]
    );
    assert_eq!(value.creation_seed(), Some(&json!("default@example.test")));

    assert_eq!(
        form.form_data(),
        &json!({ "value": "not an email or base64" })
    );
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(snapshot)
            if snapshot.form_data() == &json!({ "value": "not an email or base64" })
    ));

    let absent = definition
        .create_form(json!({}))
        .expect("an absent optional annotated property should remain absent");
    assert_eq!(absent.form_data(), &json!({}));
}

#[test]
fn collected_annotation_values_are_independent_of_applicable_schema_order() {
    let first = json!({
        "type": "string",
        "default": "z",
        "examples": ["z", 2],
        "contentSchema": { "type": "integer" }
    });
    let second = json!({
        "default": "a",
        "examples": [1, "a"],
        "contentSchema": { "type": "string" }
    });
    let compile = |all_of| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "allOf": all_of } }
        }))
        .expect("ordered annotations should compile")
    };
    let forward = compile(json!([first.clone(), second.clone()]));
    let reverse = compile(json!([second, first]));

    assert_eq!(
        node_with_binding(&forward, "/value").data_schema_annotations(),
        node_with_binding(&reverse, "/value").data_schema_annotations()
    );
}

fn node_with_binding<'a>(definition: &'a FormDefinition, binding: &str) -> DefinitionNodeView<'a> {
    definition
        .node(definition.root())
        .expect("the generated root should exist")
        .children()
        .filter_map(|identity| definition.node(identity))
        .find(|node| node.binding().map(JsonPointer::as_str) == Some(binding))
        .expect("the generated bound node should exist")
}
