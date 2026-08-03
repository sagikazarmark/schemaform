use std::{collections::BTreeSet, fs, path::Path};

use schemaform_fuzz_harness::{Target, retained_cases};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Contract {
    version: u64,
    trust_boundary: String,
    decoder: Decoder,
    execution: Execution,
    targets: Vec<TargetContract>,
}

#[derive(Deserialize)]
struct Decoder {
    max_input_bytes: usize,
    max_address_bytes: usize,
    max_address_operations: usize,
    max_generated_resources: usize,
    max_generated_schema_nodes: usize,
    max_generated_schema_depth: usize,
    max_generated_references: usize,
    max_runtime_commands: usize,
    max_transaction_operations: usize,
    max_findings_per_batch: usize,
    data_schema: DataSchemaDecoder,
    ui_schema: InputDecoder,
    form_data: FormDataDecoder,
}

#[derive(Deserialize)]
struct DataSchemaDecoder {
    bytes: usize,
    tokens: usize,
    depth: usize,
    nodes: usize,
    members: usize,
    resources: usize,
    references: usize,
    traversal: usize,
}

#[derive(Deserialize)]
struct InputDecoder {
    bytes: usize,
    tokens: usize,
    depth: usize,
    nodes: usize,
    members: usize,
    collection_length: usize,
    scalar_bytes: usize,
}

#[derive(Deserialize)]
struct FormDataDecoder {
    #[serde(flatten)]
    input: InputDecoder,
    form_tree_nodes: usize,
    repeated_items: usize,
}

#[derive(Deserialize)]
struct Execution {
    nightly_toolchain: String,
    cargo_fuzz_version: String,
    libfuzzer_sys_version: String,
    sanitizer: String,
    pull_request_seconds_per_target: u64,
    nightly_seconds_per_target: u64,
    release_seconds_per_target: u64,
    mutation_job_timeout_minutes: u64,
    corpus_minimization_timeout_minutes: u64,
    timeout_seconds_per_input: u64,
    rss_limit_mib: u64,
    retain_minimized_corpus: bool,
    browser_wasm_replay_only: bool,
}

#[derive(Deserialize)]
struct TargetContract {
    name: String,
    seed_classes: Vec<String>,
    seeds: Vec<SeedContract>,
}

#[derive(Deserialize)]
struct SeedContract {
    name: String,
    source: String,
    sha256: String,
    outcome_sha256: String,
    artifact_path: Option<String>,
    artifact_sha256: Option<String>,
    provenance: String,
    retention: String,
    expected_outcome: String,
}

#[test]
fn machine_readable_contract_matches_the_settled_fuzz_evidence() {
    let contract: Contract =
        serde_json::from_str(include_str!("../../../fuzz/evidence-contract.json"))
            .expect("the fuzz evidence contract is valid JSON");

    assert_eq!(contract.version, 2);
    assert!(contract.trust_boundary.contains("reviewed fixed schema"));
    assert!(
        contract
            .trust_boundary
            .contains("attacker-sized runtime allocation")
    );
    assert_eq!(contract.decoder.max_input_bytes, 65_536);
    assert_eq!(
        contract.decoder.max_input_bytes,
        schemaform_fuzz_harness::MAX_INPUT_BYTES
    );
    assert_eq!(
        contract.decoder.max_address_bytes,
        schemaform_fuzz_harness::MAX_ADDRESS_BYTES
    );
    assert_eq!(
        contract.decoder.max_address_operations,
        schemaform_fuzz_harness::MAX_ADDRESS_OPERATIONS
    );
    assert_eq!(
        contract.decoder.max_generated_resources,
        schemaform_fuzz_harness::MAX_GENERATED_RESOURCES
    );
    assert_eq!(
        contract.decoder.max_generated_schema_nodes,
        schemaform_fuzz_harness::MAX_GENERATED_SCHEMA_NODES
    );
    assert_eq!(
        contract.decoder.max_generated_schema_depth,
        schemaform_fuzz_harness::MAX_GENERATED_SCHEMA_DEPTH
    );
    assert_eq!(
        contract.decoder.max_generated_references,
        schemaform_fuzz_harness::MAX_GENERATED_REFERENCES
    );
    assert_eq!(
        contract.decoder.max_runtime_commands,
        schemaform_fuzz_harness::MAX_RUNTIME_COMMANDS
    );
    assert_eq!(
        contract.decoder.max_transaction_operations,
        schemaform_fuzz_harness::MAX_TRANSACTION_OPERATIONS
    );
    assert_eq!(
        contract.decoder.max_findings_per_batch,
        schemaform_fuzz_harness::MAX_FINDINGS_PER_BATCH
    );
    assert_eq!(
        (
            contract.decoder.max_runtime_commands,
            contract.decoder.max_transaction_operations,
            contract.decoder.max_findings_per_batch,
        ),
        (64, 8, 17)
    );
    assert_eq!(
        (
            contract.decoder.data_schema.bytes,
            contract.decoder.data_schema.tokens,
            contract.decoder.data_schema.depth,
            contract.decoder.data_schema.nodes,
            contract.decoder.data_schema.members,
            contract.decoder.data_schema.resources,
            contract.decoder.data_schema.references,
            contract.decoder.data_schema.traversal,
        ),
        (65_536, 4_096, 16, 256, 256, 8, 128, 1_024)
    );
    assert_input_decoder(&contract.decoder.ui_schema, 8_192, 1_024, 1_024, 64);
    assert_input_decoder(&contract.decoder.form_data.input, 8_192, 1_024, 1_024, 64);
    assert_eq!(contract.decoder.form_data.form_tree_nodes, 1_024);
    assert_eq!(contract.decoder.form_data.repeated_items, 64);
    assert_eq!(contract.execution.nightly_toolchain, "nightly-2026-07-23");
    assert_eq!(contract.execution.cargo_fuzz_version, "0.13.2");
    assert_eq!(contract.execution.libfuzzer_sys_version, "0.4.13");
    assert_eq!(contract.execution.sanitizer, "address");
    assert_eq!(contract.execution.pull_request_seconds_per_target, 60);
    assert_eq!(contract.execution.nightly_seconds_per_target, 15 * 60);
    assert_eq!(contract.execution.release_seconds_per_target, 2 * 60 * 60);
    assert_eq!(contract.execution.mutation_job_timeout_minutes, 180);
    assert_eq!(contract.execution.corpus_minimization_timeout_minutes, 20);
    assert_eq!(contract.execution.timeout_seconds_per_input, 10);
    assert_eq!(contract.execution.rss_limit_mib, 4_096);
    assert!(contract.execution.retain_minimized_corpus);
    assert!(contract.execution.browser_wasm_replay_only);

    let expected_targets = BTreeSet::from([
        "form_construction",
        "resource_compilation",
        "user_commands",
        "host_transactions",
        "external_findings",
        "ui_schema_compilation",
        "uri_pointer",
    ]);
    assert_eq!(
        contract
            .targets
            .iter()
            .map(|target| target.name.as_str())
            .collect::<BTreeSet<_>>(),
        expected_targets
    );
    for target in &contract.targets {
        let expected_classes = if matches!(
            target.name.as_str(),
            "user_commands" | "host_transactions" | "external_findings"
        ) {
            BTreeSet::from(["corpus", "model", "regression"])
        } else {
            BTreeSet::from(["corpus", "official", "regression"])
        };
        assert_eq!(
            target
                .seed_classes
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_classes
        );
        assert_eq!(target.seeds.len(), 3);
        for seed in &target.seeds {
            let retained = retained_cases()
                .iter()
                .find(|case| case.name == seed.name && target_name(case.target) == target.name)
                .expect("every contracted seed is embedded for replay");
            assert_eq!(retained.source, seed.source);
            assert_eq!(retained.expected_outcome, seed.expected_outcome);
            assert_eq!(format!("{:x}", Sha256::digest(retained.input)), seed.sha256);
            let outcome = schemaform_fuzz_harness::run(retained.target, retained.input);
            assert_eq!(retained.expected_digest, seed.outcome_sha256);
            assert_eq!(
                outcome.normalized_digest(),
                seed.outcome_sha256,
                "{} changed its normalized outcome",
                seed.name
            );
            assert!(!seed.provenance.is_empty());
            assert!(matches!(
                seed.retention.as_str(),
                "reviewed-seed" | "reviewed-regression-boundary" | "native-fuzz-failure"
            ));
            if seed.source == "regression" {
                let artifact_path = seed
                    .artifact_path
                    .as_deref()
                    .expect("regression evidence has a durable artifact path");
                let artifact = fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../..")
                        .join(artifact_path),
                )
                .expect("the contracted retained artifact exists");
                assert_eq!(format!("{:x}", Sha256::digest(&artifact)), seed.sha256);
                assert_eq!(seed.artifact_sha256.as_deref(), Some(seed.sha256.as_str()));
                assert_eq!(
                    artifact, retained.input,
                    "artifact and promoted seed differ"
                );
            } else {
                assert!(seed.artifact_path.is_none());
                assert!(seed.artifact_sha256.is_none());
            }
            assert_eq!(outcome.kind(), seed.expected_outcome);
        }
    }

    assert_eq!(retained_cases().len(), Target::ALL.len() * 3);
}

fn assert_input_decoder(
    decoder: &InputDecoder,
    tokens: usize,
    nodes: usize,
    members: usize,
    collection_length: usize,
) {
    assert_eq!(decoder.bytes, 65_536);
    assert_eq!(decoder.tokens, tokens);
    assert_eq!(decoder.depth, 16);
    assert_eq!(decoder.nodes, nodes);
    assert_eq!(decoder.members, members);
    assert_eq!(decoder.collection_length, collection_length);
    assert_eq!(decoder.scalar_bytes, 4_096);
}

fn target_name(target: Target) -> &'static str {
    match target {
        Target::ResourceCompilation => "resource_compilation",
        Target::UiSchemaCompilation => "ui_schema_compilation",
        Target::FormConstruction => "form_construction",
        Target::UriPointer => "uri_pointer",
        Target::UserCommands => "user_commands",
        Target::HostTransactions => "host_transactions",
        Target::ExternalFindings => "external_findings",
    }
}
