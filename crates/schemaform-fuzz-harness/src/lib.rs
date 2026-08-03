#![forbid(unsafe_code)]

use schemaform::{
    CompilationProfile, DefinitionFingerprint, FormDataLimits, FormDefinition, JsonPointer,
    RetrievalUri, SchemaResource,
    json::{parse_data_schema, parse_form_data, parse_ui_schema_v1},
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod runtime;

pub use runtime::{
    MAX_FINDINGS_PER_BATCH, MAX_RUNTIME_COMMANDS, MAX_TRANSACTION_OPERATIONS, RuntimeOutcome,
};

pub const MAX_INPUT_BYTES: usize = 65_536;
pub const MAX_ADDRESS_BYTES: usize = 4_096;
pub const MAX_ADDRESS_OPERATIONS: usize = 16;
pub const MAX_GENERATED_RESOURCES: usize = 8;
pub const MAX_GENERATED_SCHEMA_NODES: usize = 256;
pub const MAX_GENERATED_SCHEMA_DEPTH: usize = 16;
pub const MAX_GENERATED_REFERENCES: usize = 128;
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    ResourceCompilation,
    UiSchemaCompilation,
    FormConstruction,
    UriPointer,
    UserCommands,
    HostTransactions,
    ExternalFindings,
}

impl Target {
    pub const ALL: [Self; 7] = [
        Self::ResourceCompilation,
        Self::UiSchemaCompilation,
        Self::FormConstruction,
        Self::UriPointer,
        Self::UserCommands,
        Self::HostTransactions,
        Self::ExternalFindings,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    InputTooLarge,
    ResourceParse(Result<Value, String>),
    ResourceCompile(Result<DefinitionFingerprint, String>),
    UiSchema(Result<DefinitionFingerprint, String>),
    Form(Result<FormSnapshot, String>),
    Addresses(Vec<(Result<String, String>, Result<String, String>)>),
    UserCommands(RuntimeOutcome),
    HostTransactions(RuntimeOutcome),
    ExternalFindings(RuntimeOutcome),
}

impl Outcome {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InputTooLarge => "input-too-large",
            Self::ResourceParse(Ok(_)) => "resource-parse:success",
            Self::ResourceParse(Err(_)) => "resource-parse:error",
            Self::ResourceCompile(Ok(_)) => "resource-compile:success",
            Self::ResourceCompile(Err(error)) if error == "qualification:unresolved-reference" => {
                "resource-compile:unresolved-reference"
            }
            Self::ResourceCompile(Err(_)) => "resource-compile:error",
            Self::UiSchema(Ok(_)) => "ui-schema:success",
            Self::UiSchema(Err(_)) => "ui-schema:error",
            Self::Form(Ok(_)) => "form:success",
            Self::Form(Err(_)) => "form:error",
            Self::Addresses(_) => "addresses:observed",
            Self::UserCommands(_) => "user-commands:completed",
            Self::HostTransactions(_) => "host-transactions:completed",
            Self::ExternalFindings(_) => "external-findings:completed",
        }
    }

    pub fn normalized_digest(&self) -> String {
        let normalized = match self {
            Self::InputTooLarge => json!({ "outcome": "input-too-large" }),
            Self::ResourceParse(result) => {
                json!({ "outcome": "resource-parse", "result": result })
            }
            Self::ResourceCompile(result) => json!({
                "outcome": "resource-compile",
                "result": fingerprint_result(result),
            }),
            Self::UiSchema(result) => json!({
                "outcome": "ui-schema",
                "result": fingerprint_result(result),
            }),
            Self::Form(result) => json!({ "outcome": "form", "result": result }),
            Self::Addresses(result) => json!({ "outcome": "addresses", "result": result }),
            Self::UserCommands(result) => {
                json!({ "outcome": "user-commands", "result": result })
            }
            Self::HostTransactions(result) => {
                json!({ "outcome": "host-transactions", "result": result })
            }
            Self::ExternalFindings(result) => {
                json!({ "outcome": "external-findings", "result": result })
            }
        };
        let encoded = serde_json::to_vec(&normalized)
            .expect("normalized fuzz outcomes always have a canonical JSON encoding");
        format!("{:x}", Sha256::digest(encoded))
    }
}

fn fingerprint_result(result: &Result<DefinitionFingerprint, String>) -> Value {
    match result {
        Ok(_) => json!({ "ok": true }),
        Err(error) => json!({ "error": error }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormSnapshot {
    form_data: Value,
    nodes: Vec<NodeSnapshot>,
    validation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NodeSnapshot {
    kind: String,
    semantic_kind: String,
    binding: Option<String>,
    item_ordinal: Option<usize>,
}

pub struct RetainedCase {
    pub target: Target,
    pub source: &'static str,
    pub name: &'static str,
    pub input: &'static [u8],
    pub expected_outcome: &'static str,
    pub expected_digest: &'static str,
}

pub fn run(target: Target, input: &[u8]) -> Outcome {
    if input.len() > MAX_INPUT_BYTES {
        return Outcome::InputTooLarge;
    }
    match target {
        Target::ResourceCompilation => resource_compilation(input),
        Target::UiSchemaCompilation => ui_schema_compilation(input),
        Target::FormConstruction => form_construction(input),
        Target::UriPointer => uri_pointer(input),
        Target::UserCommands => Outcome::UserCommands(runtime::user_commands(input)),
        Target::HostTransactions => Outcome::HostTransactions(runtime::host_transactions(input)),
        Target::ExternalFindings => Outcome::ExternalFindings(runtime::external_findings(input)),
    }
}

pub fn run_deterministically(target: Target, input: &[u8]) -> Outcome {
    let first = run(target, input);
    let second = run(target, input);
    assert_eq!(
        first, second,
        "fuzz target produced a nondeterministic outcome"
    );
    first
}

fn resource_compilation(input: &[u8]) -> Outcome {
    let selector = input.first().copied().unwrap_or(0) % 5;
    if selector == 0 {
        let profile = fuzz_compilation_profile();
        return Outcome::ResourceParse(
            parse_data_schema(input.get(1..).unwrap_or_default(), &profile)
                .map_err(|error| format!("parse:{error}")),
        );
    }

    // Mutated bytes select and parameterize this finite grammar. They never become a schema
    // evaluated by the stock validator.
    let property_count = usize::from(input.get(1).copied().unwrap_or(0) % 9);
    let properties = (0..property_count)
        .map(|index| {
            let kind = input.get(index + 2).copied().unwrap_or(0) % 4;
            let schema = match kind {
                0 => json!({ "type": "string" }),
                1 => json!({ "type": "integer" }),
                2 => json!({ "type": "boolean" }),
                _ => json!({ "type": ["null", "string"] }),
            };
            (format!("field{index}"), schema)
        })
        .collect::<serde_json::Map<_, _>>();
    let mut schema = json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    });

    let resource = if selector == 2 {
        schema["properties"]["shared"] = json!({ "$ref": "urn:schemaform:fuzz:shared" });
        Some(SchemaResource::new(
            RetrievalUri::parse("urn:schemaform:fuzz:shared").expect("reviewed URI is valid"),
            json!({ "$schema": DIALECT, "type": "string" }),
        ))
    } else if selector == 3 {
        schema["$defs"] = json!({ "local": { "type": "string" } });
        schema["properties"]["local"] = json!({ "$ref": "#/$defs/local" });
        None
    } else if selector == 4 {
        schema["properties"]["missing"] = json!({ "$ref": "https://unavailable.invalid/schema" });
        None
    } else {
        None
    };
    Outcome::ResourceCompile(compile(schema, resource))
}

fn compile(
    schema: Value,
    resource: Option<SchemaResource>,
) -> Result<DefinitionFingerprint, String> {
    let mut compiler = FormDefinition::compiler(schema)
        .root_uri(RetrievalUri::parse("urn:schemaform:fuzz:root").expect("reviewed URI is valid"))
        .profile(fuzz_compilation_profile());
    if let Some(resource) = resource {
        compiler = compiler.resource(resource);
    }
    compiler
        .compile()
        .map(|definition| definition.fingerprint())
        .map_err(|error| match error {
            schemaform::CompileError::Qualification(
                schemaform::QualificationError::UnresolvedReference { .. },
            ) => "qualification:unresolved-reference".to_owned(),
            error => format!("compile:{error}"),
        })
}

fn ui_schema_compilation(input: &[u8]) -> Outcome {
    let profile = fuzz_compilation_profile();
    let result = parse_ui_schema_v1(input, &profile)
        .map_err(|error| format!("parse:{error}"))
        .and_then(|ui_schema| {
            FormDefinition::compiler(reviewed_definition_schema())
                .ui_schema(ui_schema)
                .profile(profile)
                .compile()
                .map(|definition| definition.fingerprint())
                .map_err(|error| format!("compile:{error}"))
        });
    Outcome::UiSchema(result)
}

fn form_construction(input: &[u8]) -> Outcome {
    let limits = fuzz_form_data_limits();
    let result = parse_form_data(input, &limits)
        .map_err(|error| format!("parse:{error}"))
        .and_then(|form_data| {
            FormDefinition::compile(reviewed_definition_schema())
                .map_err(|error| format!("definition:{error}"))?
                .form(form_data)
                .limits(limits)
                .build()
                .map(snapshot_form)
                .map_err(|error| format!("form:{error:?}"))
        });
    Outcome::Form(result)
}

fn snapshot_form(form: schemaform::Form) -> FormSnapshot {
    let view = form.view();
    let mut pending = vec![view.root()];
    let mut items = Vec::new();
    let mut nodes = Vec::new();
    while let Some(identity) = pending.pop() {
        let node = form
            .node(identity)
            .expect("traversed form identities remain present");
        let mut children = node.children().collect::<Vec<_>>();
        children.reverse();
        pending.extend(children);
        let item_ordinal = node.item_identity().map(|item| {
            items
                .iter()
                .position(|known| known == &item)
                .unwrap_or_else(|| {
                    items.push(item);
                    items.len() - 1
                })
        });
        nodes.push(NodeSnapshot {
            kind: format!("{:?}", node.definition().kind()),
            semantic_kind: format!("{:?}", node.definition().semantic_kind()),
            binding: node
                .binding()
                .map(|binding| binding.pointer().as_str().to_owned()),
            item_ordinal,
        });
    }
    FormSnapshot {
        form_data: form.form_data().clone(),
        nodes,
        validation: format!("{:?}", view.validation_outcome()),
    }
}

fn uri_pointer(input: &[u8]) -> Outcome {
    let observations = input
        .split(|byte| *byte == 0 || *byte == b'\n')
        .take(MAX_ADDRESS_OPERATIONS)
        .map(|bytes| {
            if bytes.len() > MAX_ADDRESS_BYTES {
                return (
                    Err("decoder:address-too-large".to_owned()),
                    Err("decoder:address-too-large".to_owned()),
                );
            }
            let text = String::from_utf8_lossy(bytes).into_owned();
            let uri = RetrievalUri::parse(text.clone())
                .map(|uri| uri.as_str().to_owned())
                .map_err(|error| format!("{error:?}"));
            let pointer = JsonPointer::parse(text)
                .map(|pointer| pointer.as_str().to_owned())
                .map_err(|error| format!("{error:?}"));
            (uri, pointer)
        })
        .collect();
    Outcome::Addresses(observations)
}

fn reviewed_definition_schema() -> Value {
    json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "age": { "type": ["null", "integer"] },
            "tags": {
                "type": "array",
                "maxItems": 16,
                "items": { "type": "string" }
            }
        }
    })
}

fn fuzz_compilation_profile() -> CompilationProfile {
    CompilationProfile::default()
        .max_data_schema_bytes(MAX_INPUT_BYTES)
        .max_data_schema_tokens(4_096)
        .max_data_schema_depth(MAX_GENERATED_SCHEMA_DEPTH)
        .max_data_schema_nodes(MAX_GENERATED_SCHEMA_NODES)
        .max_data_schema_members(256)
        .max_data_schema_resources(MAX_GENERATED_RESOURCES)
        .max_data_schema_references(MAX_GENERATED_REFERENCES)
        .max_data_schema_traversal(1_024)
        .max_ui_schema_bytes(MAX_INPUT_BYTES)
        .max_ui_schema_tokens(8_192)
        .max_ui_schema_depth(16)
        .max_ui_schema_nodes(1_024)
        .max_ui_schema_members(1_024)
        .max_ui_schema_collection_length(64)
        .max_ui_schema_scalar_bytes(4_096)
}

fn fuzz_form_data_limits() -> FormDataLimits {
    FormDataLimits::default()
        .max_bytes(MAX_INPUT_BYTES)
        .max_tokens(8_192)
        .max_depth(16)
        .max_nodes(1_024)
        .max_members(1_024)
        .max_collection_length(64)
        .max_scalar_bytes(4_096)
        .max_form_tree_nodes(1_024)
        .max_repeated_items(64)
}

pub fn retained_cases() -> &'static [RetainedCase] {
    &[
        RetainedCase {
            target: Target::ResourceCompilation,
            source: "official",
            name: "official-local-reference",
            input: include_bytes!(
                "../../../fuzz/corpus/resource_compilation/official-local-reference"
            ),
            expected_outcome: "resource-compile:success",
            expected_digest: "87320c975bec29b10bd57883ac63f431495cec6e76d0a2f0620aa5b3489a2388",
        },
        RetainedCase {
            target: Target::ResourceCompilation,
            source: "corpus",
            name: "corpus-supplied-resource",
            input: include_bytes!(
                "../../../fuzz/corpus/resource_compilation/corpus-supplied-resource"
            ),
            expected_outcome: "resource-compile:success",
            expected_digest: "87320c975bec29b10bd57883ac63f431495cec6e76d0a2f0620aa5b3489a2388",
        },
        RetainedCase {
            target: Target::ResourceCompilation,
            source: "regression",
            name: "regression-unresolved",
            input: include_bytes!(
                "../../../fuzz/corpus/resource_compilation/regression-unresolved"
            ),
            expected_outcome: "resource-compile:unresolved-reference",
            expected_digest: "a0ae49785ed2cb2a4ddeab9d33304045fcbc1e89e7be0787c2a1870539e49bb7",
        },
        RetainedCase {
            target: Target::UiSchemaCompilation,
            source: "official",
            name: "official-stack",
            input: include_bytes!("../../../fuzz/corpus/ui_schema_compilation/official-stack"),
            expected_outcome: "ui-schema:success",
            expected_digest: "b2965f7acc3292708180e6702c6caae6d1cc70461e106e286a48d2591198ad5d",
        },
        RetainedCase {
            target: Target::UiSchemaCompilation,
            source: "corpus",
            name: "corpus-controls",
            input: include_bytes!("../../../fuzz/corpus/ui_schema_compilation/corpus-controls"),
            expected_outcome: "ui-schema:success",
            expected_digest: "b2965f7acc3292708180e6702c6caae6d1cc70461e106e286a48d2591198ad5d",
        },
        RetainedCase {
            target: Target::UiSchemaCompilation,
            source: "regression",
            name: "regression-invalid-binding",
            input: include_bytes!(
                "../../../fuzz/corpus/ui_schema_compilation/regression-invalid-binding"
            ),
            expected_outcome: "ui-schema:error",
            expected_digest: "df85352993d66b6cd827cdb2104206dd6cd8f320dfc0a3f7e762160faa6341ae",
        },
        RetainedCase {
            target: Target::FormConstruction,
            source: "official",
            name: "official-empty-object",
            input: include_bytes!("../../../fuzz/corpus/form_construction/official-empty-object"),
            expected_outcome: "form:success",
            expected_digest: "23e6fa56d5fbe11c2cee110d993cd1af248f594900e7d9846cf636135a33a0cc",
        },
        RetainedCase {
            target: Target::FormConstruction,
            source: "corpus",
            name: "corpus-business-data",
            input: include_bytes!("../../../fuzz/corpus/form_construction/corpus-business-data"),
            expected_outcome: "form:success",
            expected_digest: "23733f9b42730520a7d712274cf6c9affde8dd24dc863248600f0b3beb4a0d84",
        },
        RetainedCase {
            target: Target::FormConstruction,
            source: "regression",
            name: "regression-incompatible-array",
            input: include_bytes!(
                "../../../fuzz/corpus/form_construction/regression-incompatible-array"
            ),
            expected_outcome: "form:success",
            expected_digest: "b07882a5943c49ad0631cae79afa94756f581b9cc723eda29c35cf5d4149eaa8",
        },
        RetainedCase {
            target: Target::UriPointer,
            source: "official",
            name: "official-rfc6901",
            input: include_bytes!("../../../fuzz/corpus/uri_pointer/official-rfc6901"),
            expected_outcome: "addresses:observed",
            expected_digest: "7aac6ec251981e91abbedbb351648d7e63a4e91230d50b78e72062cc29949931",
        },
        RetainedCase {
            target: Target::UriPointer,
            source: "corpus",
            name: "corpus-resource-uris",
            input: include_bytes!("../../../fuzz/corpus/uri_pointer/corpus-resource-uris"),
            expected_outcome: "addresses:observed",
            expected_digest: "dcf0cac04b68e8e7cf0a4284128b89dce0d3af94a17ca28769c520215a6fc534",
        },
        RetainedCase {
            target: Target::UriPointer,
            source: "regression",
            name: "regression-invalid-addresses",
            input: include_bytes!("../../../fuzz/corpus/uri_pointer/regression-invalid-addresses"),
            expected_outcome: "addresses:observed",
            expected_digest: "00b461c5dd3d0327776e0e2fb2040fc08982eb2672076e04dee4d592296b3bb3",
        },
        RetainedCase {
            target: Target::UserCommands,
            source: "model",
            name: "model-all-user-actions",
            input: include_bytes!("../../../fuzz/corpus/user_commands/model-all-user-actions"),
            expected_outcome: "user-commands:completed",
            expected_digest: "a5d95805815fe03a1eb048e7ad07933bbd822f7be2d00cbf306d02cddbb34dd1",
        },
        RetainedCase {
            target: Target::UserCommands,
            source: "corpus",
            name: "corpus-identity-movement",
            input: include_bytes!("../../../fuzz/corpus/user_commands/corpus-identity-movement"),
            expected_outcome: "user-commands:completed",
            expected_digest: "5ae60f4bbac5cb5266de0a8837ddba13b542f00f7b12e9a8c30e2f42bbc4bba2",
        },
        RetainedCase {
            target: Target::UserCommands,
            source: "regression",
            name: "regression-retired-target",
            input: include_bytes!("../../../fuzz/corpus/user_commands/regression-retired-target"),
            expected_outcome: "user-commands:completed",
            expected_digest: "186e53e08c26f4d9b2fc32f632a862da562798436cd83e1d93d3052fa702fe17",
        },
        RetainedCase {
            target: Target::HostTransactions,
            source: "model",
            name: "model-typed-host-methods",
            input: include_bytes!(
                "../../../fuzz/corpus/host_transactions/model-typed-host-methods"
            ),
            expected_outcome: "host-transactions:completed",
            expected_digest: "075c644147227685e80b636af7762b4be6343aceab77ab08fab96baf69a90c6c",
        },
        RetainedCase {
            target: Target::HostTransactions,
            source: "corpus",
            name: "corpus-atomic-mixed-transaction",
            input: include_bytes!(
                "../../../fuzz/corpus/host_transactions/corpus-atomic-mixed-transaction"
            ),
            expected_outcome: "host-transactions:completed",
            expected_digest: "7cf391d6676f2be4c8fa7b81792a1407a99bafc916e8a62c2cae62d13f99c752",
        },
        RetainedCase {
            target: Target::HostTransactions,
            source: "regression",
            name: "regression-closure-rollback",
            input: include_bytes!(
                "../../../fuzz/corpus/host_transactions/regression-closure-rollback"
            ),
            expected_outcome: "host-transactions:completed",
            expected_digest: "be794f77cb2051b36721e3ebcb1f73839bd4ce62e6e5d184f5d6e675fb748d70",
        },
        RetainedCase {
            target: Target::ExternalFindings,
            source: "model",
            name: "model-revision-scopes",
            input: include_bytes!("../../../fuzz/corpus/external_findings/model-revision-scopes"),
            expected_outcome: "external-findings:completed",
            expected_digest: "309add2c0d986f8857e7b5192a83f8749a484c2023e91016a2ce1fafd3d5a9c4",
        },
        RetainedCase {
            target: Target::ExternalFindings,
            source: "corpus",
            name: "corpus-replace-empty-dedup",
            input: include_bytes!(
                "../../../fuzz/corpus/external_findings/corpus-replace-empty-dedup"
            ),
            expected_outcome: "external-findings:completed",
            expected_digest: "ef34badd8d40fdd36fe3728ab832a9b4946287868360dd170762b2b9b1bcad9b",
        },
        RetainedCase {
            target: Target::ExternalFindings,
            source: "regression",
            name: "regression-over-limit-batch",
            input: include_bytes!(
                "../../../fuzz/corpus/external_findings/regression-over-limit-batch"
            ),
            expected_outcome: "external-findings:completed",
            expected_digest: "9e1c6584b60ddba86748f42f07a6ddb10c7c37ba320ba2b48610644f5099d08f",
        },
    ]
}
