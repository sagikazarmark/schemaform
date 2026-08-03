use std::error::Error as _;

use schemaform::{
    CompilationProfile, CompileError, Dialect, ExternalFinding, ExternalFindingBatch,
    FormDefinition, JsonParseError, JsonPointer, RetrievalUri, SchemaResource, SubmissionOutcome,
    json::{FormDataLimits, parse_data_schema, parse_form_data, parse_ui_schema_v1},
    ui::v1::{Auto, Binding, Element, UiSchema},
};
use serde_json::json;

#[test]
fn retrieval_uri_deserialization_enforces_constructor_invariants() {
    let valid: RetrievalUri = serde_json::from_str(r#""https://schemas.example/root.json""#)
        .expect("an absolute fragment-free retrieval URI should deserialize");
    assert_eq!(valid.as_str(), "https://schemas.example/root.json");

    for invalid in [
        r#""relative.json""#,
        r#""https://schemas.example/root.json#part""#,
    ] {
        assert!(serde_json::from_str::<RetrievalUri>(invalid).is_err());
    }
}

#[test]
fn layered_public_errors_expose_their_underlying_source() {
    let compile_error = match FormDefinition::compile(json!({
        "type": "object",
        "properties": {}
    })) {
        Err(error) => error,
        Ok(_) => panic!("a missing dialect should fail qualification"),
    };
    assert!(compile_error.source().is_some());

    let parse_error = parse_data_schema(
        br#"{}"#,
        &CompilationProfile::default().max_data_schema_bytes(1),
    )
    .expect_err("the source byte limit should fail before parsing");
    assert!(parse_error.source().is_some());

    let parse_error = parse_form_data(
        br#"{"name":"Ada"}"#,
        &FormDataLimits::default().max_bytes(1),
    )
    .expect_err("the form-data source byte limit should fail before parsing");
    assert_eq!(
        parse_error.to_string(),
        parse_error.source().unwrap().to_string(),
        "the parse error should display its structured resource-limit source"
    );
}

#[test]
fn bounded_json_parsers_preserve_owned_syntax_diagnostics() {
    let malformed = b"{\n  ]";
    let errors = [
        parse_data_schema(malformed, &CompilationProfile::default()).unwrap_err(),
        parse_form_data(malformed, &FormDataLimits::default()).unwrap_err(),
        parse_ui_schema_v1(malformed, &CompilationProfile::default()).unwrap_err(),
    ];

    for error in errors {
        let cloned = error.clone();
        let JsonParseError::Syntax(syntax) = &error else {
            panic!("expected a syntax diagnostic, got {error:?}");
        };
        assert_eq!(syntax.line(), 2);
        assert_eq!(syntax.column(), 3);
        assert_eq!(syntax.reason(), "key must be a string");
        assert_eq!(
            syntax.to_string(),
            "key must be a string at line 2 column 3"
        );
        assert_eq!(
            error.to_string(),
            "invalid JSON: key must be a string at line 2 column 3"
        );
        assert_eq!(
            error, cloned,
            "syntax diagnostics must preserve Clone and Eq"
        );
        assert!(error.source().is_some());
    }
}

#[test]
fn bounded_json_parsers_reject_trailing_string_escapes_without_panicking() {
    let malformed = br#""trailing\"#;
    let errors = [
        parse_data_schema(malformed, &CompilationProfile::default()).unwrap_err(),
        parse_form_data(malformed, &FormDataLimits::default()).unwrap_err(),
        parse_ui_schema_v1(malformed, &CompilationProfile::default()).unwrap_err(),
    ];

    for error in errors {
        assert!(
            matches!(error, JsonParseError::Syntax(_)),
            "expected a syntax diagnostic, got {error:?}"
        );
    }
}

#[test]
fn accepted_core_modules_are_available_through_the_product_facade() {
    let profile = CompilationProfile::standard();
    let schema = parse_data_schema(
        br#"{
            "type": "object",
            "required": ["name"],
            "properties": { "name": { "type": "string", "title": "Name" } }
        }"#,
        &profile,
    )
    .expect("the data schema should parse");
    let definition = FormDefinition::compiler(schema)
        .default_dialect(Dialect::Draft202012)
        .root_uri(
            RetrievalUri::parse("urn:schemaform:test")
                .expect("the retrieval URI should be absolute"),
        )
        .profile(profile.clone())
        .compile()
        .expect("the self-contained data schema should compile");

    let parsed_data = parse_form_data(br#"{"name":"Ada"}"#, &FormDataLimits::default())
        .expect("the form data should parse");
    let mut form = definition
        .form(parsed_data)
        .build()
        .expect("the form builder should create an owned form");
    let pointer = JsonPointer::parse("/name").expect("the control pointer should be valid");
    let transition = form
        .transact(|transaction| transaction.set(&pointer, json!("Grace")))
        .expect("the typed host transaction should commit");
    assert!(!transition.is_empty());

    let batch = ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::advisory("check-name", pointer, json!({}))],
    );
    form.apply_external_findings(batch)
        .expect("the current external finding batch should apply");
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(_)
    ));

    let resource = SchemaResource::new(
        RetrievalUri::parse("urn:schemaform:resource")
            .expect("the resource URI should be absolute"),
        json!({}),
    );
    assert_eq!(resource.uri().as_str(), "urn:schemaform:resource");
}

#[test]
fn explicit_and_defaulted_dialects_have_the_same_definition_fingerprint() {
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "name": { "type": "string" } }
    });
    let defaulted = FormDefinition::compiler(schema.clone())
        .default_dialect(Dialect::Draft202012)
        .compile()
        .expect("the default dialect should qualify the schema");
    let mut explicit = schema;
    explicit.as_object_mut().unwrap().insert(
        "$schema".to_owned(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    let explicit = FormDefinition::compile(explicit).expect("the explicit dialect should compile");

    assert_eq!(defaulted.fingerprint(), explicit.fingerprint());
}

#[test]
fn compile_errors_with_nonblocking_reports_format_without_panicking() {
    let analysis = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    }))
    .analyze()
    .expect("the supported schema should analyze");
    let (_, report) = analysis.into_parts();
    assert!(!report.is_blocking());

    let error = CompileError::Capability(report);
    assert_eq!(error.to_string(), "data schema cannot be represented");
}

#[test]
fn accepted_ui_schema_v1_types_are_serde_compatible() {
    let root_pointer = JsonPointer::parse("").expect("the root pointer should be valid");
    let ui_schema = UiSchema::new(Element::Auto(Auto::new(Binding::root(root_pointer))));
    let encoded = serde_json::to_vec(&ui_schema).expect("the UI schema should serialize");
    let decoded = parse_ui_schema_v1(&encoded, &CompilationProfile::default())
        .expect("the UI schema should parse through the public helper");
    assert_eq!(decoded, ui_schema);
}
