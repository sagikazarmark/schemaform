use schemaform::{
    AnalysisError, CompileError, FormDefinition, QualificationError, QualificationResource,
    RetrievalUri, SchemaResource,
};
use serde_json::{Value, json};

const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

fn uri(value: &str) -> RetrievalUri {
    RetrievalUri::parse(value).expect("the fixture URI should be absolute and fragment-free")
}

fn assert_location(
    location: &schemaform::QualificationLocation,
    resource: QualificationResource,
    retrieval_uri: &str,
    pointer: &str,
) {
    assert_eq!(location.resource(), resource);
    assert_eq!(location.retrieval_uri().as_str(), retrieval_uri);
    assert_eq!(location.pointer().as_str(), pointer);
}

#[test]
fn default_dialect_qualifies_a_standalone_boolean_schema() {
    let analysis = FormDefinition::compiler(json!(true))
        .default_dialect(schemaform::Dialect::Draft202012)
        .analyze()
        .expect("the default dialect should qualify every valid schema shape");
    let finding = analysis
        .capability_report()
        .findings()
        .next()
        .expect("an unconstrained boolean root is outside the editable profile");

    assert_eq!(finding.code(), "core.boolean.unconstrained");
    assert_eq!(finding.instance_location().as_str(), "");
    assert_eq!(finding.keyword_location().pointer().as_str(), "");
    assert!(
        analysis
            .definition()
            .node(analysis.definition().root())
            .expect("the analyzed root should exist")
            .schema_locations()
            .all(|location| location.pointer().as_str().is_empty())
    );
}

#[test]
fn default_dialect_applies_to_caller_supplied_resource_roots() {
    let definition = FormDefinition::compiler(json!({
        "type": "object",
        "properties": {
            "name": { "$ref": "https://schemas.example/name.json" }
        }
    }))
    .root_uri(uri("https://schemas.example/root.json"))
    .resource(SchemaResource::new(
        uri("https://schemas.example/name.json"),
        json!({ "type": "string" }),
    ))
    .default_dialect(schemaform::Dialect::Draft202012)
    .compile()
    .expect("the explicit default should qualify every supplied document root");

    assert!(definition.node(definition.root()).is_some());
}

#[test]
fn default_dialect_still_rejects_nested_dialect_switches() {
    let result = FormDefinition::compiler(json!({
        "type": "object",
        "properties": {
            "value": {
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "string"
            }
        }
    }))
    .default_dialect(schemaform::Dialect::Draft202012)
    .compile();
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("a default root dialect must not authorize nested dialect switches"),
    };

    assert!(matches!(
        error,
        CompileError::Qualification(QualificationError::NestedDialectSwitch { location, .. })
            if location.pointer().as_str() == "/properties/value/$schema"
    ));
}

#[test]
fn malformed_schemas_are_typed_located_and_atomic() {
    let compiler = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "unsupported": { "oneOf": [{ "type": "string" }, { "type": "integer" }] },
            "broken": { "type": "not-a-json-type" }
        }
    }))
    .root_uri(uri("https://schemas.example/root.json"));

    for error in [
        compiler
            .clone()
            .compile()
            .err()
            .expect("strict compilation should fail qualification"),
        match compiler
            .analyze()
            .err()
            .expect("lenient analysis should fail qualification")
        {
            AnalysisError::Qualification(error) => CompileError::Qualification(error),
            error => panic!("lenient analysis returned a non-qualification error: {error}"),
        },
    ] {
        let CompileError::Qualification(QualificationError::InvalidSchema { location }) = error
        else {
            panic!("malformed data schema returned a non-qualification error: {error}");
        };
        assert_location(
            &location,
            QualificationResource::Root,
            "https://schemas.example/root.json",
            "/properties/broken/type",
        );
    }

    let unsupported_pattern = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "name": { "type": "string", "pattern": "(a)\\1" }
        }
    }))
    .root_uri(uri("https://schemas.example/pattern.json"))
    .compile()
    .err()
    .expect("a pattern rejected by the selected validator should fail qualification");
    let CompileError::Qualification(QualificationError::InvalidSchema { location }) =
        unsupported_pattern
    else {
        panic!("validator construction returned a non-qualification error: {unsupported_pattern}");
    };
    assert_location(
        &location,
        QualificationResource::Root,
        "https://schemas.example/pattern.json",
        "/properties/name/pattern",
    );

    let reached_malformed = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$ref": "#/x-target",
        "x-target": { "type": "not-a-json-type" }
    }))
    .root_uri(uri("https://schemas.example/reached.json"))
    .compile()
    .err()
    .expect("a malformed reference-reached schema should fail qualification");
    let CompileError::Qualification(QualificationError::InvalidSchema { location }) =
        reached_malformed
    else {
        panic!("a malformed reference target returned the wrong error: {reached_malformed}");
    };
    assert_location(
        &location,
        QualificationResource::Root,
        "https://schemas.example/reached.json",
        "/x-target/type",
    );

    let referenced_pattern = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "name": { "$ref": "https://schemas.example/pattern-child.json" }
        }
    }))
    .root_uri(uri("https://schemas.example/pattern-root.json"))
    .resource(SchemaResource::new(
        uri("https://retrieval.example/pattern-child.json"),
        json!({
            "$schema": DRAFT_2020_12,
            "$id": "https://schemas.example/pattern-child.json",
            "type": "string",
            "pattern": "(a)\\1"
        }),
    ))
    .compile()
    .err()
    .expect("an invalid pattern in a referenced resource should fail qualification");
    let CompileError::Qualification(QualificationError::InvalidSchema { location }) =
        referenced_pattern
    else {
        panic!("a referenced invalid pattern returned the wrong error: {referenced_pattern}");
    };
    assert_location(
        &location,
        QualificationResource::Caller(0),
        "https://retrieval.example/pattern-child.json",
        "/pattern",
    );
}

#[test]
fn dialect_vocabulary_and_nested_switch_failures_are_distinct() {
    let missing = FormDefinition::compiler(json!({ "type": "object" }))
        .root_uri(uri("https://schemas.example/missing.json"))
        .compile()
        .err()
        .expect("a missing root dialect should fail qualification");
    let CompileError::Qualification(QualificationError::MissingDialect { location }) = missing
    else {
        panic!("a missing dialect returned the wrong error: {missing}");
    };
    assert_location(
        &location,
        QualificationResource::Root,
        "https://schemas.example/missing.json",
        "",
    );

    let unsupported = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object"
    }))
    .root_uri(uri("https://schemas.example/root.json"))
    .resource(SchemaResource::new(
        uri("https://schemas.example/legacy.json"),
        json!({ "$schema": "http://json-schema.org/draft-07/schema#" }),
    ))
    .compile()
    .err()
    .expect("an unsupported caller-resource dialect should fail qualification");
    let CompileError::Qualification(QualificationError::UnsupportedDialect { location, dialect }) =
        unsupported
    else {
        panic!("an unsupported dialect returned the wrong error: {unsupported}");
    };
    assert_eq!(dialect, "http://json-schema.org/draft-07/schema#");
    assert_location(
        &location,
        QualificationResource::Caller(0),
        "https://schemas.example/legacy.json",
        "/$schema",
    );

    let nested = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$defs": {
            "legacy": { "$schema": "http://json-schema.org/draft-07/schema#" }
        },
        "type": "object"
    }))
    .root_uri(uri("https://schemas.example/nested.json"))
    .compile()
    .err()
    .expect("a nested dialect switch should fail qualification");
    let CompileError::Qualification(QualificationError::NestedDialectSwitch { location, dialect }) =
        nested
    else {
        panic!("a nested dialect switch returned the wrong error: {nested}");
    };
    assert_eq!(dialect, "http://json-schema.org/draft-07/schema#");
    assert_location(
        &location,
        QualificationResource::Root,
        "https://schemas.example/nested.json",
        "/$defs/legacy/$schema",
    );

    let custom_meta_uri = "https://schemas.example/custom-meta.json";
    let vocabulary = "https://schemas.example/vocabulary/unsupported";
    let unsupported_vocabulary = FormDefinition::compiler(json!({
        "$schema": custom_meta_uri,
        "type": "object"
    }))
    .root_uri(uri("https://schemas.example/custom-root.json"))
    .resource(SchemaResource::new(
        uri(custom_meta_uri),
        json!({
            "$schema": DRAFT_2020_12,
            "$id": custom_meta_uri,
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                (vocabulary): true
            }
        }),
    ))
    .compile()
    .err()
    .expect("an unsupported required vocabulary should fail qualification");
    let CompileError::Qualification(QualificationError::UnsupportedRequiredVocabulary {
        location,
        vocabulary: actual,
    }) = unsupported_vocabulary
    else {
        panic!("an unsupported vocabulary returned the wrong error: {unsupported_vocabulary}");
    };
    assert_eq!(actual, vocabulary);
    assert_location(
        &location,
        QualificationResource::Caller(0),
        custom_meta_uri,
        "/$vocabulary/https:~1~1schemas.example~1vocabulary~1unsupported",
    );

    let pointer_meta_uri = "https://schemas.example/meta-container.json";
    let pointer_vocabulary = "https://schemas.example/vocabulary/pointer-unsupported";
    let pointer_vocabulary_error = FormDefinition::compiler(json!({
        "$schema": format!("{pointer_meta_uri}#/$defs/custom"),
        "type": "object"
    }))
    .root_uri(uri("https://schemas.example/pointer-root.json"))
    .resource(SchemaResource::new(
        uri(pointer_meta_uri),
        json!({
            "$schema": DRAFT_2020_12,
            "$defs": {
                "custom": {
                    "$vocabulary": {
                        "https://json-schema.org/draft/2020-12/vocab/core": true,
                        (pointer_vocabulary): true
                    }
                }
            }
        }),
    ))
    .compile()
    .err()
    .expect("a pointer-addressed meta-schema should qualify required vocabularies");
    assert!(matches!(
        pointer_vocabulary_error,
        CompileError::Qualification(
            QualificationError::UnsupportedRequiredVocabulary { vocabulary, .. }
        ) if vocabulary == pointer_vocabulary
    ));

    FormDefinition::compiler(json!({
        "type": "object",
        "properties": { "name": { "type": "string" } }
    }))
    .root_uri(uri("https://schemas.example/defaulted.json"))
    .default_dialect(schemaform::Dialect::Draft202012)
    .compile()
    .expect("an explicit Draft 2020-12 default should qualify a missing declaration");

    FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$vocabulary": { "https://schemas.example/ordinary-annotation": true },
        "type": "object",
        "properties": { "name": { "type": "string" } }
    }))
    .root_uri(uri("https://schemas.example/ordinary-vocabulary.json"))
    .compile()
    .expect("$vocabulary in an ordinary data schema is not an active declaration");
}

#[test]
fn duplicate_identities_and_anchors_report_both_source_locations() {
    let root_uri = "https://schemas.example/root.json";

    let invalid_identity = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$id": "https://schemas.example/root.json#fragment",
        "type": "object"
    }))
    .root_uri(uri(root_uri))
    .compile()
    .err()
    .expect("a fragmented canonical identity should fail qualification");
    let CompileError::Qualification(QualificationError::InvalidCanonicalIdentity {
        location,
        identity,
    }) = invalid_identity
    else {
        panic!("an invalid canonical identity returned the wrong error: {invalid_identity}");
    };
    assert_eq!(identity, "https://schemas.example/root.json#fragment");
    assert_location(&location, QualificationResource::Root, root_uri, "/$id");

    let built_in_collision = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object"
    }))
    .root_uri(uri(root_uri))
    .resource(SchemaResource::new(
        uri(DRAFT_2020_12),
        json!({ "$schema": DRAFT_2020_12 }),
    ))
    .compile()
    .err()
    .expect("caller resources should not shadow built-in Draft resources");
    let CompileError::Qualification(QualificationError::DuplicateCanonicalIdentity {
        identity,
        first_location,
        second_location,
    }) = built_in_collision
    else {
        panic!("a built-in identity collision returned the wrong error: {built_in_collision}");
    };
    assert_eq!(identity, DRAFT_2020_12);
    assert_location(
        &first_location,
        QualificationResource::BuiltIn,
        DRAFT_2020_12,
        "",
    );
    assert_location(
        &second_location,
        QualificationResource::Caller(0),
        DRAFT_2020_12,
        "",
    );

    let duplicate_uri = "https://retrieval.example/duplicate.json";
    let duplicate_retrieval = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object"
    }))
    .root_uri(uri(root_uri))
    .resource(SchemaResource::new(
        uri(duplicate_uri),
        json!({ "$schema": DRAFT_2020_12 }),
    ))
    .resource(SchemaResource::new(
        uri(duplicate_uri),
        json!({ "$schema": DRAFT_2020_12 }),
    ))
    .compile()
    .err()
    .expect("duplicate retrieval identities should fail qualification");
    let CompileError::Qualification(QualificationError::DuplicateRetrievalIdentity {
        identity,
        first_location,
        second_location,
    }) = duplicate_retrieval
    else {
        panic!("duplicate retrieval identities returned the wrong error: {duplicate_retrieval}");
    };
    assert_eq!(identity, duplicate_uri);
    assert_location(
        &first_location,
        QualificationResource::Caller(0),
        duplicate_uri,
        "",
    );
    assert_location(
        &second_location,
        QualificationResource::Caller(1),
        duplicate_uri,
        "",
    );

    let canonical_uri = "https://schemas.example/shared.json";
    let duplicate_canonical = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object"
    }))
    .root_uri(uri(root_uri))
    .resource(SchemaResource::new(
        uri("https://retrieval.example/first.json"),
        json!({ "$schema": DRAFT_2020_12, "$id": canonical_uri }),
    ))
    .resource(SchemaResource::new(
        uri("https://retrieval.example/second.json"),
        json!({ "$schema": DRAFT_2020_12, "$id": canonical_uri }),
    ))
    .compile()
    .err()
    .expect("duplicate canonical identities should fail qualification");
    let CompileError::Qualification(QualificationError::DuplicateCanonicalIdentity {
        identity,
        first_location,
        second_location,
    }) = duplicate_canonical
    else {
        panic!("duplicate canonical identities returned the wrong error: {duplicate_canonical}");
    };
    assert_eq!(identity, canonical_uri);
    assert_location(
        &first_location,
        QualificationResource::Caller(0),
        "https://retrieval.example/first.json",
        "/$id",
    );
    assert_location(
        &second_location,
        QualificationResource::Caller(1),
        "https://retrieval.example/second.json",
        "/$id",
    );

    for (first_keyword, second_keyword) in [
        ("$anchor", "$anchor"),
        ("$dynamicAnchor", "$dynamicAnchor"),
        ("$anchor", "$dynamicAnchor"),
    ] {
        let mut first = serde_json::Map::new();
        first.insert(first_keyword.to_owned(), json!("same"));
        let mut second = serde_json::Map::new();
        second.insert(second_keyword.to_owned(), json!("same"));
        let error = FormDefinition::compiler(json!({
            "$schema": DRAFT_2020_12,
            "$defs": {
                "first": Value::Object(first),
                "second": Value::Object(second)
            },
            "type": "object"
        }))
        .root_uri(uri(root_uri))
        .compile()
        .err()
        .expect("duplicate static or dynamic anchors should fail qualification");
        let CompileError::Qualification(QualificationError::DuplicateAnchorIdentity {
            resource_uri,
            anchor,
            first_location,
            second_location,
        }) = error
        else {
            panic!("duplicate anchors returned the wrong error: {error}");
        };
        assert_eq!(resource_uri, root_uri);
        assert_eq!(anchor, "same");
        assert_location(
            &first_location,
            QualificationResource::Root,
            root_uri,
            &format!("/$defs/first/{first_keyword}"),
        );
        assert_location(
            &second_location,
            QualificationResource::Root,
            root_uri,
            &format!("/$defs/second/{second_keyword}"),
        );
    }

    let opaque_duplicate = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$ref": "#/x-target",
        "x-target": {
            "$defs": {
                "first": { "$anchor": "same" },
                "second": { "$dynamicAnchor": "same" }
            }
        }
    }))
    .root_uri(uri(root_uri))
    .compile()
    .err()
    .expect("reference-reached duplicate anchors should fail qualification");
    assert!(matches!(
        opaque_duplicate,
        CompileError::Qualification(QualificationError::DuplicateAnchorIdentity { .. })
    ));
}

#[test]
fn unresolved_references_are_located_and_follow_caller_order() {
    let root_uri = "https://schemas.example/root.json";
    for (keyword, reference) in [
        ("$ref", "missing.json"),
        ("$ref", "#missing-anchor"),
        ("$ref", "#/$defs/missing"),
        ("$dynamicRef", "missing-dynamic.json#node"),
        ("$ref", "http://json-schema.org/draft-07/schema#"),
        ("$ref", "#%2Fx-target"),
    ] {
        let mut property = serde_json::Map::new();
        property.insert(keyword.to_owned(), Value::String(reference.to_owned()));
        let error = FormDefinition::compiler(json!({
            "$schema": DRAFT_2020_12,
            "type": "object",
            "properties": { "value": Value::Object(property) }
        }))
        .root_uri(uri(root_uri))
        .compile()
        .err()
        .unwrap_or_else(|| panic!("unresolved {keyword} target {reference} should fail"));
        let CompileError::Qualification(QualificationError::UnresolvedReference {
            location,
            reference: actual,
        }) = error
        else {
            panic!("an unresolved {keyword} returned the wrong error: {error}");
        };
        assert_eq!(actual, reference);
        assert_location(
            &location,
            QualificationResource::Root,
            root_uri,
            &format!("/properties/value/{keyword}"),
        );
    }

    for (first_name, first_missing) in [("first", "z-missing.json"), ("second", "a-missing.json")] {
        let resources = if first_name == "first" {
            [("first", "z-missing.json"), ("second", "a-missing.json")]
        } else {
            [("second", "a-missing.json"), ("first", "z-missing.json")]
        };
        for _ in 0..32 {
            let mut compiler = FormDefinition::compiler(json!({
                "$schema": DRAFT_2020_12,
                "type": "object"
            }))
            .root_uri(uri(root_uri));
            for (name, missing) in resources {
                compiler = compiler.resource(SchemaResource::new(
                    uri(&format!("https://schemas.example/{name}.json")),
                    json!({ "$schema": DRAFT_2020_12, "$ref": missing }),
                ));
            }
            let error = compiler
                .compile()
                .err()
                .expect("the first caller resource has an unresolved reference");
            let CompileError::Qualification(QualificationError::UnresolvedReference {
                location,
                reference,
            }) = error
            else {
                panic!("caller-order reference qualification returned the wrong error: {error}");
            };
            assert_eq!(reference, first_missing);
            assert_location(
                &location,
                QualificationResource::Caller(0),
                &format!("https://schemas.example/{first_name}.json"),
                "/$ref",
            );
        }
    }

    let transitive = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$ref": "#/x-target",
        "x-target": { "$ref": "transitive-missing.json" }
    }))
    .root_uri(uri(root_uri))
    .compile()
    .err()
    .expect("a reference-reached unresolved reference should fail qualification");
    let CompileError::Qualification(QualificationError::UnresolvedReference {
        location,
        reference,
    }) = transitive
    else {
        panic!("a transitive unresolved reference returned the wrong error: {transitive}");
    };
    assert_eq!(reference, "transitive-missing.json");
    assert_location(
        &location,
        QualificationResource::Root,
        root_uri,
        "/x-target/$ref",
    );

    let earlier_root_reference = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$ref": "first-missing.json"
    }))
    .root_uri(uri(root_uri))
    .resource(SchemaResource::new(
        uri("https://schemas.example/later.json"),
        json!({
            "$schema": DRAFT_2020_12,
            "$id": "https://schemas.example/later.json#fragment"
        }),
    ))
    .compile()
    .err()
    .expect("both the root and later caller resource fail qualification");
    assert!(matches!(
        earlier_root_reference,
        CompileError::Qualification(QualificationError::UnresolvedReference {
            reference,
            ..
        }) if reference == "first-missing.json"
    ));
}

#[test]
fn relative_root_id_is_applied_once_during_reference_qualification() {
    FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$id": "dir/root.json",
        "$defs": { "name": { "type": "string" } },
        "type": "object",
        "properties": { "name": { "$ref": "#/$defs/name" } }
    }))
    .root_uri(uri("https://schemas.example/root.json"))
    .compile()
    .expect("the root identity should be resolved exactly once against its retrieval URI");
}

#[test]
fn opaque_enclosing_resource_sets_reference_base() {
    let definition = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$ref": "#/x-resource/x-target",
        "x-resource": {
            "$id": "nested/",
            "x-target": { "$ref": "child.json" }
        }
    }))
    .root_uri(uri("https://schemas.example/root.json"))
    .resource(SchemaResource::new(
        uri("https://retrieval.example/child.json"),
        json!({
            "$schema": DRAFT_2020_12,
            "$id": "https://schemas.example/nested/child.json",
            "type": "object",
            "properties": { "name": { "type": "string" } }
        }),
    ))
    .compile()
    .expect("the opaque enclosing resource should establish the target reference base");
    assert!(definition.node(definition.root()).is_some());
}

#[test]
fn built_in_references_that_do_not_define_a_form_are_capability_failures() {
    let error = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$ref": DRAFT_2020_12
    }))
    .root_uri(uri("https://schemas.example/built-in-reference.json"))
    .compile()
    .err()
    .expect("the Draft meta-schema cannot be projected as a fixed-object form");
    assert!(matches!(error, CompileError::Capability(_)));
}

#[test]
fn reference_reached_dialect_switches_are_qualified() {
    let error = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$ref": "#/x-target",
        "x-target": { "$schema": "http://json-schema.org/draft-07/schema#" }
    }))
    .root_uri(uri("https://schemas.example/root.json"))
    .compile()
    .err()
    .expect("a reference-reached nested dialect switch should fail qualification");
    let CompileError::Qualification(QualificationError::NestedDialectSwitch { location, dialect }) =
        error
    else {
        panic!("a reference-reached dialect switch returned the wrong error: {error}");
    };
    assert_eq!(dialect, "http://json-schema.org/draft-07/schema#");
    assert_location(
        &location,
        QualificationResource::Root,
        "https://schemas.example/root.json",
        "/x-target/$schema",
    );
}
