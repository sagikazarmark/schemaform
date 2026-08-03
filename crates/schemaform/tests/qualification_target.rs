use schemaform::{
    CompileError, FormDefinition, QualificationError, QualificationResource, RetrievalUri,
    SchemaResource,
};
use serde_json::json;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

fn uri(value: &str) -> RetrievalUri {
    RetrievalUri::parse(value).expect("the fixture URI should be absolute and fragment-free")
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn qualification_errors_are_deterministic_on_this_target() {
    for _ in 0..64 {
        let error = FormDefinition::compiler(json!({
            "$schema": DRAFT_2020_12,
            "type": "object"
        }))
        .root_uri(uri("https://schemas.example/root.json"))
        .resource(SchemaResource::new(
            uri("https://schemas.example/first.json"),
            json!({ "$schema": "http://json-schema.org/draft-07/schema#" }),
        ))
        .resource(SchemaResource::new(
            uri("https://schemas.example/second.json"),
            json!({ "$schema": DRAFT_2020_12, "type": "not-a-json-type" }),
        ))
        .compile()
        .err()
        .expect("both caller resources fail qualification");

        let CompileError::Qualification(QualificationError::UnsupportedDialect {
            location,
            dialect,
        }) = error
        else {
            panic!("qualification returned the wrong deterministic error: {error}");
        };
        assert_eq!(location.resource(), QualificationResource::Caller(0));
        assert_eq!(
            location.retrieval_uri().as_str(),
            "https://schemas.example/first.json"
        );
        assert_eq!(location.pointer().as_str(), "/$schema");
        assert_eq!(dialect, "http://json-schema.org/draft-07/schema#");
    }
}
