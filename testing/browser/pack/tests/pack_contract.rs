use std::{collections::BTreeSet, fs, path::PathBuf, process::Command};

use browser_workload_pack::{
    AccessibilityException, AccessibilityImpact, AccessibilityViolation, EvidenceManifest,
    EvidenceObject, InteractionArtifactKind, InteractionBrowser, InteractionStatus,
    LatencyContextObservation, LatencyMetricObservation, LatencyObservation,
    LatencyProcessObservation, LatencyRuns, LatencySample, LatencyWorkload, PACK_VERSION, PROFILES,
    ResourceArtifactObservation, ResourceMetricObservation, ResourceObservation,
    ResourceOperationSample, STATES, SetupOperation, VARIANTS, Workload,
};
use schemaform::{
    CompilationProfile, ExternalFinding, ExternalFindingBatch, FindingVisibility,
    FindingVisibilityPolicy, FormDefinition, JsonPointer, SubmissionOutcome,
    form::{ParseBlockerKind, SubmissionBlocker, ValidationOutcomeView},
    json::{parse_data_schema, parse_form_data, parse_ui_schema_v1},
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn pack_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("workload-pack")
}

#[test]
fn checked_in_pack_is_reproducible_and_content_addressed() {
    let root = pack_root();
    browser_workload_pack::check(&root).expect("the checked-in pack must match the generator");
    assert_sidecar(&root, "manifest.json");
    assert_sidecar(&root, "runner-manifest.json");
    assert_sidecar(&root, "interaction-manifest.json");
    assert_sidecar(&root, "artifact-manifest.json");
    assert_sidecar(&root, "resource-manifest.json");
}

#[test]
fn artifact_manifest_declares_fixed_first_release_caps() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(pack_root().join("artifact-manifest.json")).unwrap())
            .unwrap();

    assert_eq!(manifest["version"], PACK_VERSION);
    assert!(manifest.get("runner_manifest_sha256").is_none());
    assert_eq!(
        manifest["rust_wasm_tools"]["rust"],
        "1.90.0 (1159e78c4 2025-09-14)"
    );
    assert_eq!(manifest["compression"]["brotli"], "1.1.0");
    assert_eq!(
        manifest["pipeline"][0]["arguments"],
        json!([
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "browser-workload-runner"
        ])
    );
    assert_eq!(
        manifest["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|metric| (
                metric["id"].as_str().unwrap(),
                metric["safety_cap_bytes"].as_u64().unwrap(),
            ))
            .collect::<Vec<_>>(),
        [
            ("brotli-wasm-total", 1536 * 1024),
            ("brotli-wasm-incremental", 512 * 1024),
            ("brotli-runtime-javascript-total", 64 * 1024),
        ]
    );
    assert_eq!(manifest["artifacts"].as_array().unwrap().len(), 7);
    assert!(manifest.get("calibration_observation_sha256").is_none());
    assert!(manifest.get("memory_protocol").is_none());
}

#[test]
fn artifact_verification_uses_file_bytes_and_fixed_caps() {
    let workspace = std::env::temp_dir().join(format!(
        "schemaform-artifacts-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("pack")
    ));
    let root = workspace.join("testing/browser/workload-pack");
    fs::create_dir_all(&root).unwrap();
    for name in ["artifact-manifest.json", "artifact-manifest.sha256"] {
        fs::copy(pack_root().join(name), root.join(name)).unwrap();
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("artifact-manifest.json")).unwrap()).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        let path = workspace.join(artifact["path"].as_str().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, synthetic_artifact_bytes(&path)).unwrap();
    }

    browser_workload_pack::verify_artifact_files(&root).unwrap();

    let wasm =
        workspace.join("testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm.br");
    let shell = workspace
        .join("testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm.br");
    fs::write(&shell, vec![1_u8; 1024 * 1024 + 1]).unwrap();
    fs::write(&wasm, vec![0_u8; 1536 * 1024 + 1]).unwrap();
    assert_eq!(
        browser_workload_pack::verify_artifact_files(&root).unwrap_err(),
        "artifact size cap failed for brotli-wasm-total"
    );

    fs::write(&shell, [1_u8; 8]).unwrap();
    fs::write(&wasm, vec![0_u8; 512 * 1024 + 9]).unwrap();
    assert_eq!(
        browser_workload_pack::verify_artifact_files(&root).unwrap_err(),
        "artifact size cap failed for brotli-wasm-incremental"
    );

    fs::write(&wasm, [1_u8; 8]).unwrap();
    let javascript =
        workspace.join("testing/browser/artifacts/bindgen/browser_workload_runner.js.br");
    fs::write(&javascript, vec![0_u8; 64 * 1024 + 1]).unwrap();
    assert_eq!(
        browser_workload_pack::verify_artifact_files(&root).unwrap_err(),
        "artifact size cap failed for brotli-runtime-javascript-total"
    );

    fs::write(&javascript, [1_u8; 8]).unwrap();
    fs::write(&shell, [0_u8; 9]).unwrap();
    assert_eq!(
        browser_workload_pack::verify_artifact_files(&root).unwrap_err(),
        "empty-shell Brotli WASM is larger than the production artifact"
    );

    fs::write(&shell, [1_u8; 8]).unwrap();
    let product_wasm =
        workspace.join("testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm");
    fs::write(&product_wasm, [0_u8; 8]).unwrap();
    assert_eq!(
        browser_workload_pack::verify_artifact_files(&root).unwrap_err(),
        "artifact production-wasm is not WebAssembly"
    );
    fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn interaction_manifest_contains_the_exact_cross_browser_matrix() {
    let manifest = browser_workload_pack::read_interaction_manifest(&pack_root()).unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.suite, "schemaform-dioxus/browser_csr");
    assert_eq!(
        manifest.browsers,
        [
            InteractionBrowser::Chromium,
            InteractionBrowser::Firefox,
            InteractionBrowser::Webkit,
        ]
    );
    assert_eq!(manifest.viewport_widths_css_pixels, [320, 1280]);
    assert_eq!(manifest.zoom_percents, [100, 200]);
    assert_eq!(manifest.cells.len(), 12);
    assert_eq!(manifest.accessibility.engine, "axe-core");
    assert_eq!(manifest.accessibility.version, "4.10.3");
    assert!(manifest.accessibility.reviewed_exceptions.is_empty());
    assert_eq!(
        manifest
            .accessibility
            .checkpoints
            .iter()
            .flat_map(|checkpoint| checkpoint.coverage.iter().map(String::as_str))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "arrays",
            "blocked-submission",
            "boolean-control",
            "capability-findings",
            "choice-control",
            "constant-control",
            "external-findings",
            "fixed-object-control",
            "help",
            "indeterminate-findings",
            "integer-control",
            "number-control",
            "parse-findings",
            "presence-compatible",
            "presence-empty",
            "presence-incompatible",
            "presence-missing",
            "presence-null",
            "read-only",
            "string-control",
            "tabs",
            "ui-auto",
            "ui-control",
            "ui-grid",
            "ui-group",
            "ui-stack",
            "ui-tabs",
            "ui-text",
            "unsupported-regions",
            "validation-findings",
            "write-only",
        ])
    );

    let expected_cells = ["chromium", "firefox", "webkit"]
        .into_iter()
        .flat_map(|browser| {
            [320, 1280].into_iter().flat_map(move |width| {
                [100, 200]
                    .into_iter()
                    .map(move |zoom| format!("{browser}-{width}-{zoom}"))
            })
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        manifest
            .cells
            .iter()
            .map(|cell| cell.id.clone())
            .collect::<BTreeSet<_>>(),
        expected_cells
    );

    assert_eq!(
        manifest
            .scenarios
            .iter()
            .map(|scenario| scenario.area.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "array-focus",
            "business-schema-corpus",
            "controls",
            "exact-numbers",
            "findings",
            "grids",
            "ime",
            "keyboard-order",
            "localization",
            "presence-repair",
            "reactivity",
            "submission",
            "tabs",
        ])
    );
    assert_eq!(
        manifest
            .scenarios
            .iter()
            .flat_map(|scenario| scenario.assertions.iter().map(String::as_str))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "announcements",
            "default-renderers",
            "dom-identity",
            "focus",
            "form-data",
            "mount-drop",
            "renderer-entry",
            "submission",
            "submission-callback",
        ])
    );
    assert!(
        manifest
            .scenarios
            .iter()
            .all(|scenario| !scenario.traces.is_empty())
    );
}

#[test]
fn interaction_completion_rejects_missing_failed_or_mismeasured_cells() {
    let manifest = browser_workload_pack::read_interaction_manifest(&pack_root()).unwrap();
    let mut observation = browser_workload_pack::expected_interaction_observation(&manifest);
    assert_eq!(observation.cells[0].artifacts.len(), 4);
    assert_eq!(
        observation.cells[0]
            .artifacts
            .iter()
            .map(|artifact| artifact.kind)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            InteractionArtifactKind::AccessibilityReport,
            InteractionArtifactKind::BrowserLog,
            InteractionArtifactKind::Screenshot,
            InteractionArtifactKind::Trace,
        ])
    );
    browser_workload_pack::verify_interaction_observation(&manifest, &observation).unwrap();

    let removed = observation.cells.pop().unwrap();
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );
    observation.cells.push(removed);

    observation.cells[0].status = InteractionStatus::Failed;
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );
    observation.cells[0].status = InteractionStatus::Passed;

    observation.cells[0].effective_viewport_width_css_pixels += 1;
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );
    observation.cells[0].effective_viewport_width_css_pixels -= 1;

    observation.cells[0].traces.pop();
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );

    observation = browser_workload_pack::expected_interaction_observation(&manifest);
    observation.cells[0].artifacts.pop();
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );

    observation = browser_workload_pack::expected_interaction_observation(&manifest);
    observation.cells[0].accessibility.pop();
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );

    observation = browser_workload_pack::expected_interaction_observation(&manifest);
    observation.cells[0].accessibility[0].trace = "another_passing_trace".to_owned();
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );

    observation = browser_workload_pack::expected_interaction_observation(&manifest);
    observation.cells[0].accessibility[0].aria_snapshot = "- form".to_owned();
    assert!(
        browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
    );

    for impact in [
        AccessibilityImpact::Minor,
        AccessibilityImpact::Moderate,
        AccessibilityImpact::Serious,
        AccessibilityImpact::Critical,
        AccessibilityImpact::Unknown,
    ] {
        observation = browser_workload_pack::expected_interaction_observation(&manifest);
        observation.cells[0].accessibility[0]
            .violations
            .push(AccessibilityViolation {
                rule_id: "label".to_owned(),
                impact,
                nodes: 1,
                targets: vec!["#unlabeled".to_owned()],
            });
        assert!(
            browser_workload_pack::verify_interaction_observation(&manifest, &observation).is_err()
        );
    }

    let mut reviewed_manifest = manifest.clone();
    let reviewed_checkpoint = reviewed_manifest.accessibility.checkpoints[0].clone();
    let compensating_test = reviewed_manifest.accessibility.checkpoints[1].trace.clone();
    reviewed_manifest
        .accessibility
        .reviewed_exceptions
        .push(AccessibilityException {
            checkpoint: reviewed_checkpoint.id,
            rule_id: "label".to_owned(),
            impact: AccessibilityImpact::Unknown,
            nodes: 1,
            targets: vec!["#unlabeled".to_owned()],
            defect_url: "https://github.com/dequelabs/axe-core/issues/1".to_owned(),
            compensating_test,
            expires_on: "2999-01-01".to_owned(),
        });
    browser_workload_pack::verify_interaction_observation(&reviewed_manifest, &observation)
        .unwrap();

    observation.cells[0].accessibility[0].violations[0].targets =
        vec!["#different-node".to_owned()];
    assert!(
        browser_workload_pack::verify_interaction_observation(&reviewed_manifest, &observation)
            .is_err()
    );
    observation.cells[0].accessibility[0].violations[0].targets = vec!["#unlabeled".to_owned()];

    reviewed_manifest.accessibility.reviewed_exceptions[0]
        .defect_url
        .clear();
    assert!(
        browser_workload_pack::verify_interaction_observation(&reviewed_manifest, &observation)
            .is_err()
    );

    reviewed_manifest.accessibility.reviewed_exceptions[0].defect_url =
        "https://github.com/dequelabs/axe-core/issues/1".to_owned();
    reviewed_manifest.accessibility.reviewed_exceptions[0].expires_on = "2999-99-99".to_owned();
    assert!(
        browser_workload_pack::verify_interaction_observation(&reviewed_manifest, &observation)
            .is_err()
    );

    reviewed_manifest.accessibility.reviewed_exceptions[0].expires_on = "2000-01-01".to_owned();
    assert!(
        browser_workload_pack::verify_interaction_observation(&reviewed_manifest, &observation)
            .is_err()
    );
}

#[test]
fn latency_manifest_declares_the_settled_protocol_and_every_workload() {
    let manifest = browser_workload_pack::read_latency_manifest(&pack_root()).unwrap();

    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.runner, "schemaform-perf-v1");
    assert_eq!(manifest.browser, InteractionBrowser::Chromium);
    assert_eq!(manifest.protocol.hot_processes, 5);
    assert_eq!(manifest.protocol.warmups_per_process, 50);
    assert_eq!(manifest.protocol.samples_per_process, 200);
    assert_eq!(manifest.protocol.cold_context_samples, 100);
    assert_eq!(manifest.protocol.percentile_method, "nearest-rank");
    assert!(!manifest.protocol.outlier_deletion_allowed);
    assert!(!manifest.protocol.discretionary_retries_allowed);
    assert!(manifest.protocol.fresh_processes);
    assert!(manifest.protocol.fresh_cold_contexts);
    assert_eq!(manifest.metrics.len(), 232);

    let fixed_metrics = manifest
        .metrics
        .iter()
        .filter(|metric| metric.fixed_edit_gate)
        .collect::<Vec<_>>();
    assert_eq!(fixed_metrics.len(), 8);
    assert!(fixed_metrics.iter().all(|metric| {
        metric.scenario.starts_with("O500-")
            && metric.workload == LatencyWorkload::Edit
            && metric.ceiling_p95_ms == Some(16.0)
            && metric.ceiling_p99_ms == Some(32.0)
    }));
    assert!(manifest.metrics.iter().all(|metric| {
        metric.phases
            == if metric.cold {
                1
            } else if metric.workload == LatencyWorkload::Arrays {
                6
            } else {
                2
            }
    }));
    assert!(manifest.calibration_observation_sha256.is_none());

    assert_sidecar(&pack_root(), "latency-manifest.json");
}

#[test]
fn latency_percentiles_and_calibration_follow_the_fixed_arithmetic() {
    let samples = (1..=20).map(f64::from).collect::<Vec<_>>();
    assert_eq!(
        browser_workload_pack::nearest_rank(&samples, 50).unwrap(),
        10.0
    );
    assert_eq!(
        browser_workload_pack::nearest_rank(&samples, 95).unwrap(),
        19.0
    );
    assert_eq!(
        browser_workload_pack::nearest_rank(&samples, 99).unwrap(),
        20.0
    );
    assert_eq!(
        browser_workload_pack::calibrated_latency_ceiling(12.0).unwrap(),
        15.0
    );
    assert_eq!(
        browser_workload_pack::calibrated_latency_ceiling(12.01).unwrap(),
        15.5
    );
    assert!(browser_workload_pack::nearest_rank(&[], 95).is_err());
    assert!(browser_workload_pack::nearest_rank(&[f64::NAN], 95).is_err());
}

#[test]
fn latency_observations_are_recomputed_and_calibrate_only_once() {
    let root = pack_root();
    let manifest = browser_workload_pack::read_latency_manifest(&root).unwrap();
    let mut observation = conforming_latency_observation(&manifest, 1.0);

    browser_workload_pack::verify_latency_observation(&manifest, &observation).unwrap();
    let calibrated = browser_workload_pack::calibrate_latency_manifest(&manifest, &observation)
        .expect("the first conforming tracer calibrates every missing ceiling");
    assert!(calibrated.calibration_observation_sha256.is_some());
    assert!(
        calibrated
            .metrics
            .iter()
            .all(|metric| { metric.ceiling_p95_ms.is_some() && metric.ceiling_p99_ms.is_some() })
    );
    browser_workload_pack::verify_latency_observation(&calibrated, &observation).unwrap();
    assert!(browser_workload_pack::calibrate_latency_manifest(&calibrated, &observation).is_err());

    let hot = observation
        .metrics
        .iter_mut()
        .find(|metric| metric.id == "O500-generated-valid/edit")
        .unwrap();
    let LatencyRuns::Hot { processes } = &mut hot.runs else {
        panic!("edit must be a hot workload")
    };
    processes[0].samples[0].duration_ms = 33.0;
    for process in processes.iter_mut() {
        for sample in &mut process.samples {
            sample.duration_ms = 33.0;
        }
    }
    assert!(browser_workload_pack::verify_latency_observation(&manifest, &observation).is_err());

    let mut malformed = conforming_latency_observation(&manifest, 1.0);
    let LatencyRuns::Hot { processes } = &mut malformed.metrics[2].runs else {
        panic!("the third metric must be hot")
    };
    processes[0].samples.pop();
    assert!(browser_workload_pack::verify_latency_observation(&manifest, &malformed).is_err());

    let mut retried = conforming_latency_observation(&manifest, 1.0);
    retried.discretionary_retries = 1;
    assert!(browser_workload_pack::verify_latency_observation(&manifest, &retried).is_err());
}

#[test]
fn resource_manifest_declares_the_settled_memory_and_artifact_contract() {
    let manifest = browser_workload_pack::read_resource_manifest(&pack_root()).unwrap();

    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.runner, "schemaform-perf-v1");
    assert_eq!(manifest.browser, InteractionBrowser::Chromium);
    assert_eq!(
        manifest.memory_protocol.scenario,
        "A100x5-authored-64-finding"
    );
    assert_eq!(manifest.memory_protocol.operations, 1_000);
    assert_eq!(
        manifest.memory_protocol.operation_cycle,
        [
            "edit",
            "findings",
            "visibility",
            "arrays",
            "localization",
            "submission",
        ]
    );
    assert_eq!(
        manifest.memory_protocol.operation_phases,
        std::collections::BTreeMap::from([
            ("arrays".to_owned(), 6),
            ("edit".to_owned(), 2),
            ("findings".to_owned(), 2),
            ("localization".to_owned(), 2),
            ("submission".to_owned(), 2),
            ("visibility".to_owned(), 2),
        ])
    );
    assert_eq!(manifest.metrics.len(), 5);
    assert_eq!(
        manifest
            .metrics
            .iter()
            .map(|metric| (metric.id.as_str(), metric.safety_cap_bytes))
            .collect::<Vec<_>>(),
        [
            ("wasm-linear-high-water", 128 * 1024 * 1024),
            ("browser-heap-post-gc-delta", 64 * 1024 * 1024),
            ("brotli-wasm-total", 1536 * 1024),
            ("brotli-wasm-incremental", 512 * 1024),
            ("brotli-runtime-javascript-total", 64 * 1024),
        ]
    );
    assert!(
        manifest
            .metrics
            .iter()
            .all(|metric| metric.ceiling_bytes.is_none())
    );
    assert!(manifest.calibration_observation_sha256.is_none());
    assert_eq!(manifest.artifacts.len(), 7);
}

#[test]
fn resource_observations_recompute_raw_samples_and_calibrate_only_once() {
    let root = pack_root();
    let manifest = browser_workload_pack::read_resource_manifest(&root).unwrap();
    let latency = browser_workload_pack::read_latency_manifest(&root).unwrap();
    let latency_observation = conforming_latency_observation(&latency, 1.0);
    let mut observation = conforming_resource_observation(&manifest);

    browser_workload_pack::verify_resource_observation(&manifest, &observation).unwrap();
    let calibrated = browser_workload_pack::calibrate_resource_manifest(
        &manifest,
        &observation,
        &latency,
        &latency_observation,
    )
    .expect("the first fully conforming tracer calibrates every resource ceiling");
    assert!(calibrated.calibration_observation_sha256.is_some());
    assert!(
        calibrated
            .metrics
            .iter()
            .all(|metric| metric.ceiling_bytes.is_some())
    );
    assert_eq!(
        calibrated.metrics[0].ceiling_bytes,
        Some(14 * 1024 * 1024),
        "11 MiB grows to 13.2 MiB and rounds upward to 14 MiB"
    );
    assert_eq!(
        calibrated.metrics[2].ceiling_bytes,
        Some(120 * 1024),
        "100 KiB grows to exactly 120 KiB"
    );
    assert!(
        browser_workload_pack::calibrate_resource_manifest(
            &calibrated,
            &observation,
            &latency,
            &latency_observation,
        )
        .is_err()
    );

    observation.operations[10].workload = "submission".to_owned();
    assert!(browser_workload_pack::verify_resource_observation(&manifest, &observation).is_err());

    observation = conforming_resource_observation(&manifest);
    observation.metrics[0].bytes += 1;
    assert!(browser_workload_pack::verify_resource_observation(&manifest, &observation).is_err());

    observation = conforming_resource_observation(&manifest);
    observation.waivers.push("accept the miss".to_owned());
    assert!(browser_workload_pack::verify_resource_observation(&manifest, &observation).is_err());
}

#[test]
fn resource_calibration_rounds_up_and_never_exceeds_absolute_caps() {
    assert_eq!(
        browser_workload_pack::calibrated_resource_ceiling(10 * 1024 * 1024, 1024 * 1024).unwrap(),
        12 * 1024 * 1024
    );
    assert_eq!(
        browser_workload_pack::calibrated_resource_ceiling(10 * 1024 * 1024 + 1, 1024 * 1024)
            .unwrap(),
        13 * 1024 * 1024
    );

    let root = pack_root();
    let manifest = browser_workload_pack::read_resource_manifest(&root).unwrap();
    let latency = browser_workload_pack::read_latency_manifest(&root).unwrap();
    let latency_observation = conforming_latency_observation(&latency, 1.0);
    let mut observation = conforming_resource_observation(&manifest);
    observation.metrics[0].bytes = 128 * 1024 * 1024;
    for operation in &mut observation.operations {
        operation.wasm_memory_bytes = 128 * 1024 * 1024;
    }
    assert!(
        browser_workload_pack::calibrate_resource_manifest(
            &manifest,
            &observation,
            &latency,
            &latency_observation,
        )
        .is_err(),
        "a 1.20x ceiling above the absolute cap is not calibratable"
    );

    let observation = conforming_resource_observation(&manifest);
    let mut slow_latency = conforming_latency_observation(&latency, 1.0);
    let compilation = slow_latency
        .metrics
        .iter_mut()
        .find(|metric| metric.id == "S1-generated-valid/compilation")
        .unwrap();
    compilation.p50_ms = 500.0;
    compilation.p95_ms = 500.0;
    compilation.p99_ms = 500.0;
    let LatencyRuns::Cold { contexts } = &mut compilation.runs else {
        panic!("compilation must use cold contexts")
    };
    for context in contexts {
        context.sample.duration_ms = 500.0;
    }
    assert!(
        browser_workload_pack::calibrate_resource_manifest(
            &manifest,
            &observation,
            &latency,
            &slow_latency,
        )
        .is_err(),
        "resource calibration requires a latency tracer below every safety cap"
    );
}

#[test]
fn evidence_objects_are_content_addressed_and_reject_waivers() {
    let root = std::env::temp_dir().join(format!(
        "schemaform-evidence-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("pack")
    ));
    let bytes = b"raw browser evidence\n";
    let digest = hex_digest(bytes);
    let object = root.join("objects/sha256").join(&digest);
    fs::create_dir_all(object.parent().unwrap()).unwrap();
    fs::write(&object, bytes).unwrap();
    let mut manifest = EvidenceManifest {
        version: PACK_VERSION,
        hash_algorithm: "sha256".to_owned(),
        source_commit: "a".repeat(40),
        source_tree: "b".repeat(40),
        workflow_run_attempt: 1,
        discretionary_retries: 0,
        waivers: Vec::new(),
        objects: vec![EvidenceObject {
            name: "testing/browser/artifacts/resource-observation.json".to_owned(),
            kind: "raw-resource-samples".to_owned(),
            sha256: digest,
            bytes: bytes.len() as u64,
        }],
        conclusions: Vec::new(),
    };

    browser_workload_pack::verify_evidence_objects(&root, &manifest).unwrap();
    fs::write(&object, b"tampered\n").unwrap();
    assert!(browser_workload_pack::verify_evidence_objects(&root, &manifest).is_err());

    fs::write(&object, bytes).unwrap();
    manifest.waivers.push("accept missing evidence".to_owned());
    assert!(browser_workload_pack::verify_evidence_objects(&root, &manifest).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn first_release_archive_excludes_deferred_numeric_evidence() {
    let workspace = std::env::temp_dir().join(format!(
        "schemaform-first-release-evidence-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("pack")
    ));
    if workspace.exists() {
        fs::remove_dir_all(&workspace).unwrap();
    }
    let root = workspace.join("testing/browser/workload-pack");
    copy_directory(&pack_root(), &root);
    fs::write(
        workspace.join(".gitignore"),
        "testing/browser/artifacts/\nevidence/\n",
    )
    .unwrap();
    git(&workspace, &["init", "--quiet"]);
    git(&workspace, &["add", "."]);
    git(
        &workspace,
        &[
            "-c",
            "user.name=Schemaform Tests",
            "-c",
            "user.email=tests@schemaform.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );

    let interaction_manifest = browser_workload_pack::read_interaction_manifest(&root).unwrap();
    let mut interaction =
        browser_workload_pack::expected_interaction_observation(&interaction_manifest);
    for cell in &mut interaction.cells {
        for artifact in &mut cell.artifacts {
            let bytes = if artifact.kind == InteractionArtifactKind::AccessibilityReport {
                serde_json::to_vec(
                    &cell
                        .accessibility
                        .iter()
                        .map(|checkpoint| {
                            json!({
                                "id": checkpoint.id,
                                "trace": checkpoint.trace,
                                "aria_snapshot": checkpoint.aria_snapshot,
                                "report": { "violations": [] },
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap()
            } else {
                format!("{:?}\n", artifact.kind).into_bytes()
            };
            let path = workspace.join(&artifact.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &bytes).unwrap();
            artifact.bytes = bytes.len() as u64;
            artifact.sha256 = hex_digest(&bytes);
        }
    }
    let observation_path = workspace.join("testing/browser/artifacts/interaction-observation.json");
    fs::write(
        observation_path,
        serde_json::to_vec_pretty(&interaction).unwrap(),
    )
    .unwrap();

    let artifact_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("artifact-manifest.json")).unwrap()).unwrap();
    for artifact in artifact_manifest["artifacts"].as_array().unwrap() {
        let path = workspace.join(artifact["path"].as_str().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, synthetic_artifact_bytes(&path)).unwrap();
    }

    let archive = workspace.join("evidence");
    browser_workload_pack::archive_browser_evidence(&root, &archive).unwrap();
    browser_workload_pack::verify_evidence_archive(&archive).unwrap();
    let manifest: EvidenceManifest =
        serde_json::from_slice(&fs::read(archive.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(
        manifest
            .conclusions
            .iter()
            .map(|conclusion| conclusion.gate.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["artifact-size", "interactions-accessibility", "source-tree",])
    );
    assert!(
        manifest
            .objects
            .iter()
            .all(|object| !object.name.contains("latency") && !object.name.contains("resource"))
    );
    fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn manifest_contains_the_exact_representative_matrix_and_workload_coverage() {
    let root = pack_root();
    let manifest = browser_workload_pack::read_manifest(&root).unwrap();
    assert_eq!(manifest.version, PACK_VERSION);
    assert_eq!(manifest.hash_algorithm, "sha256");
    assert_eq!(manifest.scenarios.len(), 32);

    let expected = PROFILES
        .into_iter()
        .flat_map(|(profile, ..)| {
            VARIANTS.into_iter().flat_map(move |variant| {
                STATES
                    .into_iter()
                    .map(move |state| format!("{profile}-{variant}-{state}"))
            })
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        manifest
            .scenarios
            .iter()
            .map(|scenario| scenario.id.clone())
            .collect::<BTreeSet<_>>(),
        expected
    );

    let covered = manifest
        .scenarios
        .iter()
        .flat_map(|scenario| scenario.workloads.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        covered,
        BTreeSet::from([
            "arrays",
            "compilation",
            "edit",
            "findings",
            "localization",
            "mount",
            "submission",
            "visibility",
        ])
    );
    for variant in VARIANTS {
        let variant_coverage = manifest
            .scenarios
            .iter()
            .filter(|scenario| scenario.variant == variant)
            .flat_map(|scenario| scenario.workloads.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        assert_eq!(variant_coverage, covered, "{variant} workload coverage");
    }

    for scenario in &manifest.scenarios {
        let expected_shape = PROFILES
            .iter()
            .find(|(profile, ..)| *profile == scenario.profile)
            .unwrap();
        assert_eq!(
            (scenario.controls, scenario.rows, scenario.controls_per_row),
            (expected_shape.1, expected_shape.2, expected_shape.3)
        );
        let payload = browser_workload_pack::read_scenario(&root, scenario).unwrap();
        if matches!(scenario.profile.as_str(), "O50" | "O500") {
            assert!(
                payload
                    .data_schema
                    .pointer("/properties/level_1/properties/level_2/properties/level_3/properties/field_000")
                    .is_some(),
                "{} must be a depth-four fixed object",
                scenario.id
            );
        }
        assert_eq!(
            payload
                .workloads
                .iter()
                .map(Workload::name)
                .collect::<Vec<_>>(),
            scenario
                .workloads
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        assert_exact_workload_recipes(&payload.workloads, scenario);
    }
}

#[test]
fn runner_manifest_pins_every_comparison_input() {
    let runner: serde_json::Value =
        serde_json::from_slice(&fs::read(pack_root().join("runner-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(runner["environment"], "schemaform-perf-v1");
    assert_eq!(runner["hardware"]["architecture"], "x86_64");
    assert_eq!(
        runner["operating_system"]["distribution"],
        "Ubuntu 24.04.3 LTS"
    );
    assert!(runner["browsers"]["playwright"].is_string());
    for browser in ["chromium", "firefox", "webkit"] {
        assert!(runner["browsers"][browser]["version"].is_string());
        assert!(runner["browsers"][browser]["revision"].is_string());
    }
    for tool in [
        "rust",
        "cargo",
        "wasm_target",
        "wasm_bindgen_cli",
        "binaryen_wasm_opt",
        "wasm_tools",
    ] {
        assert!(runner["rust_wasm_tools"][tool].is_string());
    }
    assert!(runner["compression"]["brotli"].is_string());
    assert_eq!(runner["power"]["governor"], "performance");
    assert_eq!(runner["power"]["turbo"], "disabled");
    assert!(runner["affinity"]["measurement_cpu_list"].is_string());
    let pipeline = runner["production_artifact"]["pipeline"]
        .as_array()
        .unwrap();
    assert_eq!(
        pipeline
            .iter()
            .map(|step| step["program"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "node",
            "cargo",
            "cargo",
            "wasm-bindgen",
            "wasm-opt",
            "mv",
            "brotli",
            "brotli",
            "cargo",
            "wasm-bindgen",
            "wasm-opt",
            "mv",
            "brotli",
        ]
    );
    assert_eq!(
        pipeline[1]["arguments"].as_array().unwrap().last().unwrap(),
        "testing/browser/artifacts/runner-observation.json"
    );
    assert_eq!(
        runner["production_artifact"]["wasm"],
        "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm"
    );
    assert_eq!(pipeline[4]["arguments"][0], "-Oz");
    assert_eq!(pipeline[10]["arguments"][0], "-Oz");
    assert_eq!(
        runner["empty_shell_artifact"]["brotli_wasm"],
        "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm.br"
    );
    assert_eq!(
        runner["comparison_policy"],
        "Abort before measurement when any pinned value differs."
    );
}

#[test]
fn runner_comparison_aborts_on_every_pinned_section_mismatch() {
    let mut expected: serde_json::Value =
        serde_json::from_slice(&fs::read(pack_root().join("runner-manifest.json")).unwrap())
            .unwrap();
    let observed = expected.clone();
    browser_workload_pack::verify_runner_observation(&expected, &observed).unwrap();

    for field in [
        "version",
        "environment",
        "comparison_policy",
        "hardware",
        "operating_system",
        "browsers",
        "rust_wasm_tools",
        "compression",
        "power",
        "affinity",
        "production_artifact",
    ] {
        let original = expected[field].clone();
        expected[field] = serde_json::Value::Null;
        let error = browser_workload_pack::verify_runner_observation(&expected, &observed)
            .expect_err("every runner mismatch must abort comparison");
        assert!(error.contains("comparison aborted"));
        expected[field] = original;
    }
}

#[test]
fn every_scenario_reaches_its_declared_state_through_the_product_path() {
    let root = pack_root();
    let manifest = browser_workload_pack::read_manifest(&root).unwrap();
    for reference in &manifest.scenarios {
        let scenario = browser_workload_pack::read_scenario(&root, reference).unwrap();
        let profile = CompilationProfile::standard();
        let data_schema = parse_data_schema(
            &serde_json::to_vec(&scenario.data_schema).unwrap(),
            &profile,
        )
        .unwrap_or_else(|error| panic!("{} data schema: {error:?}", scenario.id));
        let mut compiler = FormDefinition::compiler(data_schema).profile(profile.clone());
        if let Some(ui_schema) = &scenario.ui_schema {
            let ui_schema = parse_ui_schema_v1(&serde_json::to_vec(ui_schema).unwrap(), &profile)
                .unwrap_or_else(|error| panic!("{} UI schema: {error:?}", scenario.id));
            compiler = compiler.ui_schema(ui_schema);
        }
        let definition = compiler
            .compile()
            .unwrap_or_else(|error| panic!("{} compilation: {error:?}", scenario.id));
        if scenario.variant == "authored" {
            assert!(
                definition_has_localization(&definition),
                "{} must retain localization references",
                scenario.id
            );
        }

        let form_data = parse_form_data(
            &serde_json::to_vec(&scenario.initial_form_data).unwrap(),
            &schemaform::FormDataLimits::default(),
        )
        .unwrap();
        let mut form = definition
            .form(form_data)
            .finding_visibility(FindingVisibilityPolicy::new(
                FindingVisibility::Immediate,
                FindingVisibility::Immediate,
            ))
            .build()
            .unwrap_or_else(|error| panic!("{} construction: {error:?}", scenario.id));
        assert_eq!(runtime_scalar_control_count(&form), reference.controls);

        for operation in scenario.setup {
            match operation {
                SetupOperation::InputText { binding, value } => {
                    let target = form_identity_for_binding(&form, &binding);
                    form.user().input_text(target, value).unwrap();
                }
                SetupOperation::ExternalFindings { count, binding } => {
                    let revision = form.view().data_revision();
                    let findings = (0..count)
                        .map(|index| {
                            ExternalFinding::blocking(
                                format!("workload-{index:02}"),
                                JsonPointer::parse(&binding).unwrap(),
                                json!({ "index": index }),
                            )
                        })
                        .collect::<Vec<_>>();
                    form.apply_external_findings(ExternalFindingBatch::new(
                        "browser-workload",
                        revision,
                        findings,
                    ))
                    .unwrap();
                }
            }
        }

        match scenario.state.as_str() {
            "valid" => {
                assert!(matches!(
                    form.view().validation_outcome(),
                    ValidationOutcomeView::Valid
                ));
                assert!(matches!(
                    form.prepare_submission().outcome(),
                    SubmissionOutcome::Ready(_)
                ));
            }
            "invalid" => {
                let view = form.view();
                let ValidationOutcomeView::Invalid {
                    findings,
                    truncated,
                } = view.validation_outcome()
                else {
                    panic!("{} should be invalid", scenario.id);
                };
                assert_eq!(findings.len(), 1, "{} finding count", scenario.id);
                assert!(!truncated);
                assert!(has_submission_blocker(&mut form, |blocker| matches!(
                    blocker,
                    SubmissionBlocker::Validation(_)
                )));
            }
            "parse-blocked" => {
                assert!(form.view().visible_findings().any(|finding| matches!(
                    finding,
                    schemaform::FindingView::Parse {
                        kind: ParseBlockerKind::InvalidInteger,
                        ..
                    }
                )));
                assert!(has_submission_blocker(&mut form, |blocker| matches!(
                    blocker,
                    SubmissionBlocker::Parse { .. }
                )));
            }
            "64-finding" => {
                let view = form.view();
                let ValidationOutcomeView::Invalid { findings, .. } = view.validation_outcome()
                else {
                    panic!("{} should contain validation findings", scenario.id);
                };
                assert_eq!(findings.len(), 32, "{} validation findings", scenario.id);
                assert_eq!(form.view().visible_findings().count(), 64);
                form.set_finding_visibility(FindingVisibilityPolicy::new(
                    FindingVisibility::SubmissionOnly,
                    FindingVisibility::SubmissionOnly,
                ));
                assert_eq!(form.view().visible_findings().count(), 0);
                assert!(has_submission_blocker(&mut form, |blocker| matches!(
                    blocker,
                    SubmissionBlocker::External { .. }
                )));
            }
            state => panic!("unknown state {state}"),
        }
    }
}

fn runtime_scalar_control_count(form: &schemaform::Form) -> usize {
    let mut count = 0;
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).unwrap();
        count += usize::from(matches!(
            node.definition().semantic_kind(),
            Some(
                schemaform::definition::SemanticKind::String
                    | schemaform::definition::SemanticKind::Number
                    | schemaform::definition::SemanticKind::Integer
                    | schemaform::definition::SemanticKind::Boolean
                    | schemaform::definition::SemanticKind::Choice
                    | schemaform::definition::SemanticKind::Null
            )
        ));
        pending.extend(node.children());
    }
    count
}

fn definition_has_localization(definition: &FormDefinition) -> bool {
    let mut pending = vec![definition.root()];
    while let Some(identity) = pending.pop() {
        let node = definition.node(identity).unwrap();
        if node
            .label_reference()
            .and_then(|reference| reference.key())
            .is_some()
            || node
                .item_label_reference()
                .and_then(|reference| reference.key())
                .is_some()
        {
            return true;
        }
        pending.extend(node.children());
    }
    false
}

fn form_identity_for_binding(
    form: &schemaform::Form,
    binding: &str,
) -> schemaform::InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).unwrap();
        if node
            .binding()
            .is_some_and(|candidate| candidate.pointer().as_str() == binding)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("no form node has binding {binding}")
}

fn has_submission_blocker(
    form: &mut schemaform::Form,
    predicate: impl Fn(&SubmissionBlocker) -> bool,
) -> bool {
    let preparation = form.prepare_submission();
    let SubmissionOutcome::Blocked(blockers) = preparation.outcome() else {
        return false;
    };
    blockers.iter().any(predicate)
}

fn conforming_latency_observation(
    manifest: &browser_workload_pack::LatencyManifest,
    duration_ms: f64,
) -> LatencyObservation {
    let root = pack_root();
    let runner_observation =
        serde_json::from_slice(&fs::read(root.join("runner-manifest.json")).unwrap()).unwrap();
    let interaction_manifest = browser_workload_pack::read_interaction_manifest(&root).unwrap();
    LatencyObservation {
        version: manifest.version,
        workflow_run_attempt: 1,
        runner_observation,
        workload_manifest_sha256: manifest.workload_manifest_sha256.clone(),
        production_artifact_sha256: "a".repeat(64),
        environment_sanity_passed: true,
        discretionary_retries: 0,
        outliers_removed: 0,
        interaction: browser_workload_pack::expected_interaction_observation(&interaction_manifest),
        metrics: manifest
            .metrics
            .iter()
            .map(|metric| LatencyMetricObservation {
                id: metric.id.clone(),
                p50_ms: duration_ms,
                p95_ms: duration_ms,
                p99_ms: duration_ms,
                runs: if metric.cold {
                    LatencyRuns::Cold {
                        contexts: (0..manifest.protocol.cold_context_samples)
                            .map(|context| LatencyContextObservation {
                                context,
                                fresh_context: true,
                                sample: LatencySample {
                                    sequence: 0,
                                    phase: 0,
                                    duration_ms,
                                },
                            })
                            .collect(),
                    }
                } else {
                    LatencyRuns::Hot {
                        processes: (0..manifest.protocol.hot_processes)
                            .map(|process| LatencyProcessObservation {
                                process,
                                fresh_process: true,
                                warmups_completed: manifest.protocol.warmups_per_process,
                                samples: (0..manifest.protocol.samples_per_process)
                                    .map(|sequence| LatencySample {
                                        sequence,
                                        phase: sequence % metric.phases,
                                        duration_ms,
                                    })
                                    .collect(),
                            })
                            .collect(),
                    }
                },
            })
            .collect(),
    }
}

fn conforming_resource_observation(
    manifest: &browser_workload_pack::ResourceManifest,
) -> ResourceObservation {
    let runner_observation =
        serde_json::from_slice(&fs::read(pack_root().join("runner-manifest.json")).unwrap())
            .unwrap();
    let mib = 1024_u64 * 1024;
    let artifact_sizes = [
        ("production-wasm", 200 * 1024),
        ("production-javascript", 20 * 1024),
        ("production-brotli-wasm", 100 * 1024),
        ("production-brotli-javascript", 10 * 1024),
        ("empty-shell-wasm", 100 * 1024),
        ("empty-shell-javascript", 10 * 1024),
        ("empty-shell-brotli-wasm", 60 * 1024),
    ];
    ResourceObservation {
        version: manifest.version,
        workflow_run_attempt: 1,
        runner_observation,
        workload_manifest_sha256: manifest.workload_manifest_sha256.clone(),
        environment_sanity_passed: true,
        discretionary_retries: 0,
        waivers: Vec::new(),
        scenario: manifest.memory_protocol.scenario.clone(),
        wasm_memory_before_bytes: 10 * mib,
        operations: (0..manifest.memory_protocol.operations)
            .map(|sequence| {
                let cycle = sequence % manifest.memory_protocol.operation_cycle.len();
                let workload = manifest.memory_protocol.operation_cycle[cycle].clone();
                let phases = manifest.memory_protocol.operation_phases[&workload];
                ResourceOperationSample {
                    sequence,
                    workload,
                    phase: (sequence / manifest.memory_protocol.operation_cycle.len()) % phases,
                    wasm_memory_bytes: 11 * mib,
                }
            })
            .collect(),
        browser_heap_before_bytes: 20 * mib,
        browser_heap_after_bytes: 21 * mib,
        metrics: vec![
            resource_metric("wasm-linear-high-water", 11 * mib),
            resource_metric("browser-heap-post-gc-delta", mib),
            resource_metric("brotli-wasm-total", 100 * 1024),
            resource_metric("brotli-wasm-incremental", 40 * 1024),
            resource_metric("brotli-runtime-javascript-total", 10 * 1024),
        ],
        artifacts: manifest
            .artifacts
            .iter()
            .zip(artifact_sizes)
            .map(|(artifact, (id, bytes))| {
                assert_eq!(artifact.id, id);
                ResourceArtifactObservation {
                    id: artifact.id.clone(),
                    path: artifact.path.clone(),
                    sha256: if id == "production-wasm" {
                        "a".repeat(64)
                    } else {
                        "b".repeat(64)
                    },
                    bytes,
                }
            })
            .collect(),
    }
}

fn resource_metric(id: &str, bytes: u64) -> ResourceMetricObservation {
    ResourceMetricObservation {
        id: id.to_owned(),
        bytes,
    }
}

fn copy_directory(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn git(root: &std::path::Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {} failed", arguments.join(" "));
}

fn synthetic_artifact_bytes(path: &std::path::Path) -> Vec<u8> {
    match path.extension().and_then(std::ffi::OsStr::to_str) {
        Some("wasm") => b"\0asm\x01\0\0\0".to_vec(),
        Some("js") => b"export {};\n".to_vec(),
        _ => vec![1_u8; 8],
    }
}

fn assert_sidecar(root: &std::path::Path, name: &str) {
    let contents = fs::read(root.join(name)).unwrap();
    let sidecar_name = name.strip_suffix(".json").unwrap();
    let sidecar = fs::read_to_string(root.join(format!("{sidecar_name}.sha256"))).unwrap();
    assert_eq!(sidecar, format!("{}  {name}\n", hex_digest(&contents)));
}

fn assert_exact_workload_recipes(
    workloads: &[Workload],
    scenario: &browser_workload_pack::ScenarioReference,
) {
    let target = if scenario.rows == 0 {
        if scenario.profile == "S1" {
            "/field_000"
        } else {
            "/level_1/level_2/level_3/field_000"
        }
    } else {
        "/rows/0/field_000"
    };
    for workload in workloads {
        match workload {
            Workload::Compilation {
                cold,
                fresh_browser_context,
            }
            | Workload::Mount {
                cold,
                fresh_browser_context,
            } => assert!(*cold && *fresh_browser_context),
            Workload::Edit {
                binding,
                alternating_values,
            } => {
                assert_eq!(binding, target);
                assert_eq!(alternating_values, &["1", "2"]);
            }
            Workload::Findings {
                binding,
                count,
                alternating_actions,
            } => {
                assert_eq!(binding, target);
                assert_eq!(*count, 64);
                assert_eq!(alternating_actions, &["install", "clear"]);
            }
            Workload::Visibility { policies } => {
                assert_eq!(policies, &["immediate", "submission-only"]);
            }
            Workload::Arrays {
                binding,
                operations,
            } => {
                assert_eq!(scenario.profile, "A100x5");
                assert_eq!(binding, "/rows");
                assert_eq!(
                    operations,
                    &[
                        "append",
                        "remove-last",
                        "insert-before-last",
                        "remove-before-last",
                        "move-last-up",
                        "move-before-last-down",
                    ]
                );
            }
            Workload::Localization {
                locales,
                message_source,
            } => {
                assert_eq!(locales, &["en", "hu"]);
                assert_eq!(
                    message_source,
                    if scenario.variant == "authored" {
                        "ui-schema-key"
                    } else {
                        "generated-fallback"
                    }
                );
            }
            Workload::Submission { expected } => assert_eq!(
                expected,
                if scenario.state == "valid" {
                    "ready"
                } else {
                    "blocked"
                }
            ),
        }
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
