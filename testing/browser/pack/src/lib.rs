use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const PACK_VERSION: u64 = 1;
const WORKLOAD_PACK_PATH: &str = "testing/browser/workload-pack";
pub const PROFILES: [(&str, usize, usize, usize); 4] = [
    ("S1", 1, 0, 0),
    ("O50", 50, 0, 0),
    ("O500", 500, 0, 0),
    ("A100x5", 500, 100, 5),
];
pub const VARIANTS: [&str; 2] = ["generated", "authored"];
pub const STATES: [&str; 4] = ["valid", "invalid", "parse-blocked", "64-finding"];
pub const INTERACTION_BROWSERS: [InteractionBrowser; 3] = [
    InteractionBrowser::Chromium,
    InteractionBrowser::Firefox,
    InteractionBrowser::Webkit,
];
pub const INTERACTION_VIEWPORT_WIDTHS: [u32; 2] = [320, 1280];
pub const INTERACTION_ZOOM_PERCENTS: [u16; 2] = [100, 200];
pub const LATENCY_HOT_PROCESSES: usize = 5;
pub const LATENCY_WARMUPS_PER_PROCESS: usize = 50;
pub const LATENCY_SAMPLES_PER_PROCESS: usize = 200;
pub const LATENCY_COLD_CONTEXT_SAMPLES: usize = 100;
pub const RESOURCE_OPERATIONS: usize = 1_000;
const MIB: u64 = 1024 * 1024;
const KIB: u64 = 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackManifest {
    pub version: u64,
    pub hash_algorithm: String,
    pub object_directory: String,
    pub scenarios: Vec<ScenarioReference>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScenarioReference {
    pub id: String,
    pub profile: String,
    pub variant: String,
    pub state: String,
    pub controls: usize,
    pub rows: usize,
    pub controls_per_row: usize,
    pub object_sha256: String,
    pub workloads: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InteractionManifest {
    pub version: u64,
    pub suite: String,
    pub browsers: Vec<InteractionBrowser>,
    pub viewport_widths_css_pixels: Vec<u32>,
    pub zoom_percents: Vec<u16>,
    pub zoom_protocol: String,
    pub accessibility: AccessibilityGate,
    pub scenarios: Vec<InteractionScenario>,
    pub cells: Vec<InteractionCell>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccessibilityGate {
    pub engine: String,
    pub version: String,
    pub checkpoints: Vec<AccessibilityCheckpoint>,
    pub reviewed_exceptions: Vec<AccessibilityException>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccessibilityCheckpoint {
    pub id: String,
    pub trace: String,
    pub coverage: Vec<String>,
    pub aria_required: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccessibilityException {
    pub checkpoint: String,
    pub rule_id: String,
    pub impact: AccessibilityImpact,
    pub nodes: usize,
    pub targets: Vec<String>,
    pub defect_url: String,
    pub compensating_test: String,
    pub expires_on: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessibilityImpact {
    Minor,
    Moderate,
    Serious,
    Critical,
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccessibilityViolation {
    pub rule_id: String,
    pub impact: AccessibilityImpact,
    pub nodes: usize,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccessibilityCheckpointObservation {
    pub id: String,
    pub trace: String,
    pub aria_snapshot: String,
    pub violations: Vec<AccessibilityViolation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractionBrowser {
    Chromium,
    Firefox,
    Webkit,
}

impl InteractionBrowser {
    fn as_str(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Firefox => "firefox",
            Self::Webkit => "webkit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractionStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InteractionScenario {
    pub area: String,
    pub traces: Vec<String>,
    pub assertions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InteractionCell {
    pub id: String,
    pub browser: InteractionBrowser,
    pub viewport_width_css_pixels: u32,
    pub zoom_percent: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InteractionObservation {
    pub version: u64,
    pub workflow_run_attempt: u64,
    pub cells: Vec<InteractionCellObservation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InteractionCellObservation {
    pub browser: InteractionBrowser,
    pub viewport_width_css_pixels: u32,
    pub zoom_percent: u16,
    pub effective_viewport_width_css_pixels: u32,
    pub status: InteractionStatus,
    pub traces: Vec<String>,
    pub accessibility: Vec<AccessibilityCheckpointObservation>,
    pub artifacts: Vec<InteractionArtifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InteractionArtifact {
    pub kind: InteractionArtifactKind,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InteractionArtifactKind {
    Trace,
    Screenshot,
    AccessibilityReport,
    BrowserLog,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencyManifest {
    pub version: u64,
    pub runner: String,
    pub browser: InteractionBrowser,
    pub runner_manifest_sha256: String,
    pub workload_manifest_sha256: String,
    pub protocol: LatencyProtocol,
    pub calibration_observation_sha256: Option<String>,
    pub metrics: Vec<LatencyMetric>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LatencyProtocol {
    pub hot_processes: usize,
    pub warmups_per_process: usize,
    pub samples_per_process: usize,
    pub cold_context_samples: usize,
    pub percentile_method: String,
    pub percentiles: Vec<u8>,
    pub timing_boundary: String,
    pub fresh_processes: bool,
    pub fresh_cold_contexts: bool,
    pub outlier_deletion_allowed: bool,
    pub discretionary_retries_allowed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencyMetric {
    pub id: String,
    pub scenario: String,
    pub workload: LatencyWorkload,
    pub cold: bool,
    pub phases: usize,
    pub fixed_edit_gate: bool,
    pub safety_cap_p95_ms: f64,
    pub safety_cap_p99_ms: f64,
    pub ceiling_p95_ms: Option<f64>,
    pub ceiling_p99_ms: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LatencyWorkload {
    Compilation,
    Mount,
    Edit,
    Findings,
    Visibility,
    Localization,
    Submission,
    Arrays,
}

impl LatencyWorkload {
    fn as_str(self) -> &'static str {
        match self {
            Self::Compilation => "compilation",
            Self::Mount => "mount",
            Self::Edit => "edit",
            Self::Findings => "findings",
            Self::Visibility => "visibility",
            Self::Localization => "localization",
            Self::Submission => "submission",
            Self::Arrays => "arrays",
        }
    }

    fn is_cold(self) -> bool {
        matches!(self, Self::Compilation | Self::Mount)
    }

    fn phases(self) -> usize {
        match self {
            Self::Compilation | Self::Mount => 1,
            Self::Arrays => 6,
            _ => 2,
        }
    }

    fn safety_caps(self, controls: usize) -> (f64, f64) {
        match self {
            Self::Compilation => (100.0, 200.0),
            Self::Mount => (250.0, 500.0),
            Self::Localization if controls == 500 => (50.0, 100.0),
            _ => (16.0, 32.0),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LatencyCalibration {
    observation_sha256: Option<String>,
    observation: Option<LatencyObservation>,
    ceilings: Vec<LatencyCeiling>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LatencyCeiling {
    id: String,
    baseline_p95_ms: f64,
    baseline_p99_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencyObservation {
    pub version: u64,
    pub workflow_run_attempt: u64,
    pub runner_observation: Value,
    pub workload_manifest_sha256: String,
    pub production_artifact_sha256: String,
    pub environment_sanity_passed: bool,
    pub discretionary_retries: usize,
    pub outliers_removed: usize,
    pub interaction: InteractionObservation,
    pub metrics: Vec<LatencyMetricObservation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencyMetricObservation {
    pub id: String,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub runs: LatencyRuns,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LatencyRuns {
    Hot {
        processes: Vec<LatencyProcessObservation>,
    },
    Cold {
        contexts: Vec<LatencyContextObservation>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencyProcessObservation {
    pub process: usize,
    pub fresh_process: bool,
    pub warmups_completed: usize,
    pub samples: Vec<LatencySample>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencyContextObservation {
    pub context: usize,
    pub fresh_context: bool,
    pub sample: LatencySample,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatencySample {
    pub sequence: usize,
    pub phase: usize,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceManifest {
    pub version: u64,
    pub runner: String,
    pub browser: InteractionBrowser,
    pub runner_manifest_sha256: String,
    pub workload_manifest_sha256: String,
    pub memory_protocol: MemoryProtocol,
    pub calibration_observation_sha256: Option<String>,
    pub metrics: Vec<ResourceMetric>,
    pub artifacts: Vec<ResourceArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MemoryProtocol {
    pub scenario: String,
    pub operations: usize,
    pub operation_cycle: Vec<String>,
    pub operation_phases: BTreeMap<String, usize>,
    pub wasm_sampling: String,
    pub browser_heap_measurement: String,
    pub settle_condition: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceMetricKind {
    Memory,
    CompressedSize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceMetric {
    pub id: String,
    pub kind: ResourceMetricKind,
    pub safety_cap_bytes: u64,
    pub calibration_rounding_bytes: u64,
    pub ceiling_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResourceArtifact {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ArtifactManifest {
    pub version: u64,
    pub rust_wasm_tools: Value,
    pub compression: Value,
    pub pipeline: Vec<Value>,
    pub metrics: Vec<ArtifactMetric>,
    pub artifacts: Vec<ResourceArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ArtifactMetric {
    pub id: String,
    pub safety_cap_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceObservation {
    pub version: u64,
    pub workflow_run_attempt: u64,
    pub runner_observation: Value,
    pub workload_manifest_sha256: String,
    pub environment_sanity_passed: bool,
    pub discretionary_retries: usize,
    pub waivers: Vec<String>,
    pub scenario: String,
    pub wasm_memory_before_bytes: u64,
    pub operations: Vec<ResourceOperationSample>,
    pub browser_heap_before_bytes: u64,
    pub browser_heap_after_bytes: u64,
    pub metrics: Vec<ResourceMetricObservation>,
    pub artifacts: Vec<ResourceArtifactObservation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceOperationSample {
    pub sequence: usize,
    pub workload: String,
    pub phase: usize,
    pub wasm_memory_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceMetricObservation {
    pub id: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceArtifactObservation {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ResourceCalibration {
    observation_sha256: Option<String>,
    resource_observation: Option<ResourceObservation>,
    latency_observation: Option<LatencyObservation>,
    ceilings: Vec<ResourceCeiling>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ResourceCeiling {
    id: String,
    baseline_bytes: u64,
    bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvidenceManifest {
    pub version: u64,
    pub hash_algorithm: String,
    pub source_commit: String,
    pub source_tree: String,
    pub workflow_run_attempt: u64,
    pub discretionary_retries: usize,
    pub waivers: Vec<String>,
    pub objects: Vec<EvidenceObject>,
    pub conclusions: Vec<EvidenceConclusion>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvidenceObject {
    pub name: String,
    pub kind: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvidenceConclusion {
    pub gate: String,
    pub status: EvidenceStatus,
    pub evidence_sha256: Vec<String>,
    pub details: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scenario {
    pub version: u64,
    pub id: String,
    pub profile: String,
    pub variant: String,
    pub state: String,
    pub data_schema: Value,
    pub ui_schema: Option<Value>,
    pub initial_form_data: Value,
    pub setup: Vec<SetupOperation>,
    pub workloads: Vec<Workload>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum SetupOperation {
    InputText { binding: String, value: String },
    ExternalFindings { count: usize, binding: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "workload", rename_all = "kebab-case")]
pub enum Workload {
    Compilation {
        cold: bool,
        fresh_browser_context: bool,
    },
    Mount {
        cold: bool,
        fresh_browser_context: bool,
    },
    Edit {
        binding: String,
        alternating_values: [String; 2],
    },
    Findings {
        binding: String,
        count: usize,
        alternating_actions: [String; 2],
    },
    Visibility {
        policies: [String; 2],
    },
    Arrays {
        binding: String,
        operations: Vec<String>,
    },
    Localization {
        locales: [String; 2],
        message_source: String,
    },
    Submission {
        expected: String,
    },
}

impl Workload {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Compilation { .. } => "compilation",
            Self::Mount { .. } => "mount",
            Self::Edit { .. } => "edit",
            Self::Findings { .. } => "findings",
            Self::Visibility { .. } => "visibility",
            Self::Arrays { .. } => "arrays",
            Self::Localization { .. } => "localization",
            Self::Submission { .. } => "submission",
        }
    }
}

pub fn generate(root: &Path) -> Result<(), String> {
    let files = build_files()?;
    if root.exists() {
        fs::remove_dir_all(root).map_err(|error| format!("remove {}: {error}", root.display()))?;
    }
    for (relative, contents) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        fs::write(&path, contents).map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    Ok(())
}

pub fn check(root: &Path) -> Result<(), String> {
    let expected = build_files()?;
    for (relative, contents) in &expected {
        let path = root.join(relative);
        let actual =
            fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
        if &actual != contents {
            return Err(format!(
                "{} is not reproducible; run the generator",
                path.display()
            ));
        }
    }
    let actual_paths = files_below(root)?;
    let expected_paths = expected.keys().cloned().collect::<BTreeSet<_>>();
    if actual_paths != expected_paths {
        return Err("browser workload pack contains missing or untracked files".to_owned());
    }
    Ok(())
}

fn workspace_root(root: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root)
        .map_err(|error| format!("resolve browser workload pack root: {error}"))?;
    root.ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .ok_or_else(|| "browser workload pack root is not below testing/browser".to_owned())
}

pub fn verify_runner_file(root: &Path, observation_path: &Path) -> Result<(), String> {
    let expected_path = root.join("runner-manifest.json");
    let expected: Value = serde_json::from_slice(
        &fs::read(&expected_path)
            .map_err(|error| format!("read {}: {error}", expected_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", expected_path.display()))?;
    let observed: Value = serde_json::from_slice(
        &fs::read(observation_path)
            .map_err(|error| format!("read {}: {error}", observation_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", observation_path.display()))?;
    verify_runner_observation(&expected, &observed)
}

pub fn verify_interaction_file(root: &Path, observation_path: &Path) -> Result<(), String> {
    let manifest = read_interaction_manifest(root)?;
    let observation: InteractionObservation = serde_json::from_slice(
        &fs::read(observation_path)
            .map_err(|error| format!("read {}: {error}", observation_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", observation_path.display()))?;
    verify_interaction_observation(&manifest, &observation)?;
    let workspace = workspace_root(root)?;
    verify_interaction_artifact_contents(&manifest, &observation, |relative| {
        let path = workspace.join(relative);
        fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))
    })
}

pub fn verify_artifact_files(root: &Path) -> Result<(), String> {
    let manifest_path = root.join("artifact-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("read {}: {error}", manifest_path.display()))?;
    let sidecar_path = root.join("artifact-manifest.sha256");
    let sidecar_bytes = fs::read(&sidecar_path)
        .map_err(|error| format!("read {}: {error}", sidecar_path.display()))?;
    if sidecar_bytes != sidecar("artifact-manifest.json", &manifest_bytes) {
        return Err("artifact manifest sidecar mismatch".to_owned());
    }
    let manifest: ArtifactManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("parse {}: {error}", manifest_path.display()))?;
    validate_artifact_manifest(&manifest)?;
    let workspace = workspace_root(root)?;
    verify_artifact_contents(&manifest, |relative| {
        let path = workspace.join(relative);
        fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))
    })
}

pub fn verify_latency_file(root: &Path, observation_path: &Path) -> Result<(), String> {
    let manifest = read_latency_manifest(root)?;
    verify_latency_input_files(root, &manifest)?;
    let observation: LatencyObservation = serde_json::from_slice(
        &fs::read(observation_path)
            .map_err(|error| format!("read {}: {error}", observation_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", observation_path.display()))?;
    verify_latency_observation(&manifest, &observation)
}

pub fn verify_resource_file(root: &Path, observation_path: &Path) -> Result<(), String> {
    let manifest = read_resource_manifest(root)?;
    verify_resource_input_files(root, &manifest)?;
    let observation: ResourceObservation = serde_json::from_slice(
        &fs::read(observation_path)
            .map_err(|error| format!("read {}: {error}", observation_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", observation_path.display()))?;
    verify_resource_observation(&manifest, &observation)?;
    verify_resource_artifact_files(root, &manifest, &observation)
}

pub fn calibrate_latency_file(
    root: &Path,
    observation_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    let manifest = read_latency_manifest(root)?;
    verify_latency_input_files(root, &manifest)?;
    let observation: LatencyObservation = serde_json::from_slice(
        &fs::read(observation_path)
            .map_err(|error| format!("read {}: {error}", observation_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", observation_path.display()))?;
    let calibrated = calibrate_latency_manifest(&manifest, &observation)?;
    let observed = observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric))
        .collect::<BTreeMap<_, _>>();
    let calibration = LatencyCalibration {
        observation_sha256: calibrated.calibration_observation_sha256,
        observation: Some(observation.clone()),
        ceilings: calibrated
            .metrics
            .into_iter()
            .filter(|metric| !metric.fixed_edit_gate)
            .map(|metric| {
                let baseline = observed[metric.id.as_str()];
                LatencyCeiling {
                    id: metric.id,
                    baseline_p95_ms: baseline.p95_ms,
                    baseline_p99_ms: baseline.p99_ms,
                    p95_ms: metric
                        .ceiling_p95_ms
                        .expect("calibration fills every p95 ceiling"),
                    p99_ms: metric
                        .ceiling_p99_ms
                        .expect("calibration fills every p99 ceiling"),
                }
            })
            .collect(),
    };
    fs::write(output_path, canonical_json(&calibration)?)
        .map_err(|error| format!("write {}: {error}", output_path.display()))
}

pub fn calibrate_resource_file(
    root: &Path,
    resource_observation_path: &Path,
    latency_observation_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    let manifest = read_resource_manifest(root)?;
    let latency_manifest = read_latency_manifest(root)?;
    verify_resource_input_files(root, &manifest)?;
    verify_latency_input_files(root, &latency_manifest)?;
    let resource_observation: ResourceObservation = serde_json::from_slice(
        &fs::read(resource_observation_path)
            .map_err(|error| format!("read {}: {error}", resource_observation_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", resource_observation_path.display()))?;
    let latency_observation: LatencyObservation = serde_json::from_slice(
        &fs::read(latency_observation_path)
            .map_err(|error| format!("read {}: {error}", latency_observation_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", latency_observation_path.display()))?;
    verify_resource_artifact_files(root, &manifest, &resource_observation)?;
    let calibrated = calibrate_resource_manifest(
        &manifest,
        &resource_observation,
        &latency_manifest,
        &latency_observation,
    )?;
    let observed = resource_observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric.bytes))
        .collect::<BTreeMap<_, _>>();
    let calibration = ResourceCalibration {
        observation_sha256: calibrated.calibration_observation_sha256,
        resource_observation: Some(resource_observation.clone()),
        latency_observation: Some(latency_observation),
        ceilings: calibrated
            .metrics
            .into_iter()
            .map(|metric| ResourceCeiling {
                baseline_bytes: observed[metric.id.as_str()],
                id: metric.id,
                bytes: metric
                    .ceiling_bytes
                    .expect("calibration fills every resource ceiling"),
            })
            .collect(),
    };
    fs::write(output_path, canonical_json(&calibration)?)
        .map_err(|error| format!("write {}: {error}", output_path.display()))
}

pub fn archive_browser_evidence(root: &Path, output_root: &Path) -> Result<(), String> {
    if output_root.exists() {
        return Err(format!(
            "evidence archive already exists: {}",
            output_root.display()
        ));
    }
    let workspace = workspace_root(root)?;
    let source_commit = git_output(&workspace, &["rev-parse", "HEAD"])?;
    let source_tree = git_output(&workspace, &["rev-parse", "HEAD^{tree}"])?;
    let source_bundle = create_source_bundle(&workspace)?;
    let required = required_evidence_files();
    let mut objects = Vec::with_capacity(required.len());
    let mut contents = BTreeMap::new();
    for (name, kind) in required {
        let bytes = if name == "testing/browser/artifacts/source.bundle" {
            source_bundle.clone()
        } else {
            fs::read(workspace.join(&name))
                .map_err(|error| format!("read evidence {name}: {error}"))?
        };
        let digest = sha256(&bytes);
        objects.push(EvidenceObject {
            name: name.clone(),
            kind,
            sha256: digest,
            bytes: bytes.len() as u64,
        });
        contents.insert(name, bytes);
    }
    let object_digests = objects
        .iter()
        .map(|object| (object.name.as_str(), object.sha256.as_str()))
        .collect::<BTreeMap<_, _>>();
    let conclusion_evidence = expected_conclusion_evidence(&object_digests);
    let clean_source = git_clean(&workspace);
    let conclusions = vec![
        evidence_conclusion(
            "source-tree",
            clean_source,
            conclusion_evidence["source-tree"].clone(),
        ),
        evidence_conclusion(
            "interactions-accessibility",
            verify_interaction_file(
                root,
                &workspace.join("testing/browser/artifacts/interaction-observation.json"),
            ),
            conclusion_evidence["interactions-accessibility"].clone(),
        ),
        evidence_conclusion(
            "artifact-size",
            verify_artifact_files(root),
            conclusion_evidence["artifact-size"].clone(),
        ),
    ];
    let manifest = EvidenceManifest {
        version: PACK_VERSION,
        hash_algorithm: "sha256".to_owned(),
        source_commit,
        source_tree,
        workflow_run_attempt: workflow_run_attempt()?,
        discretionary_retries: 0,
        waivers: Vec::new(),
        objects,
        conclusions,
    };
    fs::create_dir_all(output_root.join("objects/sha256"))
        .map_err(|error| format!("create {}: {error}", output_root.display()))?;
    for object in &manifest.objects {
        let bytes = contents
            .get(&object.name)
            .expect("every evidence object retained its bytes");
        let path = output_root.join("objects/sha256").join(&object.sha256);
        if !path.exists() {
            fs::write(&path, bytes)
                .map_err(|error| format!("write {}: {error}", path.display()))?;
        }
    }
    let manifest_bytes = canonical_json(&manifest)?;
    fs::write(output_root.join("manifest.json"), &manifest_bytes)
        .map_err(|error| format!("write evidence manifest: {error}"))?;
    fs::write(
        output_root.join("manifest.sha256"),
        sidecar("manifest.json", &manifest_bytes),
    )
    .map_err(|error| format!("write evidence manifest sidecar: {error}"))?;
    if manifest
        .conclusions
        .iter()
        .any(|conclusion| conclusion.status == EvidenceStatus::Failed)
    {
        return Err("browser evidence archive contains failed conclusions".to_owned());
    }
    Ok(())
}

pub fn verify_evidence_archive(root: &Path) -> Result<(), String> {
    let manifest_bytes = fs::read(root.join("manifest.json"))
        .map_err(|error| format!("read evidence manifest: {error}"))?;
    let sidecar_bytes = fs::read(root.join("manifest.sha256"))
        .map_err(|error| format!("read evidence manifest sidecar: {error}"))?;
    if sidecar_bytes != sidecar("manifest.json", &manifest_bytes) {
        return Err("evidence manifest sidecar mismatch".to_owned());
    }
    let manifest: EvidenceManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("parse evidence manifest: {error}"))?;
    verify_evidence_objects(root, &manifest)?;
    let expected_objects = required_evidence_files()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let observed_objects = manifest
        .objects
        .iter()
        .map(|object| (object.name.clone(), object.kind.clone()))
        .collect::<BTreeSet<_>>();
    if observed_objects != expected_objects {
        return Err("evidence archive does not contain the exact required inventory".to_owned());
    }
    let expected_gates =
        BTreeSet::from(["source-tree", "interactions-accessibility", "artifact-size"]);
    let observed_gates = manifest
        .conclusions
        .iter()
        .map(|conclusion| conclusion.gate.as_str())
        .collect::<BTreeSet<_>>();
    if manifest.conclusions.len() != expected_gates.len()
        || observed_gates != expected_gates
        || manifest
            .conclusions
            .iter()
            .any(|conclusion| conclusion.status != EvidenceStatus::Passed)
    {
        return Err("evidence archive does not contain every passing conclusion".to_owned());
    }
    let object_digests = manifest
        .objects
        .iter()
        .map(|object| (object.name.as_str(), object.sha256.as_str()))
        .collect::<BTreeMap<_, _>>();
    let expected_evidence = expected_conclusion_evidence(&object_digests);
    for conclusion in &manifest.conclusions {
        if conclusion.evidence_sha256 != expected_evidence[conclusion.gate.as_str()] {
            return Err(format!(
                "evidence conclusion {} references the wrong objects",
                conclusion.gate
            ));
        }
    }
    verify_archived_contract(&manifest, root)
}

pub fn verify_evidence_objects(root: &Path, manifest: &EvidenceManifest) -> Result<(), String> {
    if manifest.version != PACK_VERSION
        || manifest.hash_algorithm != "sha256"
        || !is_git_object_id(&manifest.source_commit)
        || !is_git_object_id(&manifest.source_tree)
        || manifest.workflow_run_attempt != 1
        || manifest.discretionary_retries != 0
        || !manifest.waivers.is_empty()
    {
        return Err("evidence manifest changed the no-waiver archive contract".to_owned());
    }
    let mut names = BTreeSet::new();
    let mut digests = BTreeSet::new();
    for object in &manifest.objects {
        if !names.insert(object.name.as_str())
            || !is_safe_relative_path(&object.name)
            || !is_sha256(&object.sha256)
        {
            return Err("evidence manifest contains an invalid or duplicate object".to_owned());
        }
        digests.insert(object.sha256.as_str());
        let path = root.join("objects/sha256").join(&object.sha256);
        let bytes = fs::read(&path)
            .map_err(|error| format!("read evidence object {}: {error}", object.sha256))?;
        if bytes.len() as u64 != object.bytes || sha256(&bytes) != object.sha256 {
            return Err(format!(
                "evidence object {} is not content addressed",
                object.name
            ));
        }
    }
    let actual = files_below(&root.join("objects/sha256"))?;
    let expected = digests.iter().map(PathBuf::from).collect::<BTreeSet<_>>();
    if actual != expected {
        return Err("evidence object store contains missing or unreferenced bytes".to_owned());
    }
    let mut gates = BTreeSet::new();
    for conclusion in &manifest.conclusions {
        if !gates.insert(conclusion.gate.as_str())
            || conclusion
                .evidence_sha256
                .iter()
                .any(|digest| !digests.contains(digest.as_str()))
        {
            return Err("evidence manifest contains an invalid conclusion".to_owned());
        }
    }
    Ok(())
}

pub fn verify_latency_observation(
    manifest: &LatencyManifest,
    observation: &LatencyObservation,
) -> Result<(), String> {
    validate_latency_manifest(manifest)?;
    if observation.version != manifest.version {
        return Err("latency observation version mismatch".to_owned());
    }
    if observation.workflow_run_attempt != 1 {
        return Err("latency evidence came from a retried workflow run".to_owned());
    }
    if !observation.environment_sanity_passed {
        return Err("latency environment sanity did not pass".to_owned());
    }
    if observation.discretionary_retries != 0 || observation.outliers_removed != 0 {
        return Err("latency observations may not retry or remove outliers".to_owned());
    }
    if observation.workload_manifest_sha256 != manifest.workload_manifest_sha256 {
        return Err("latency workload manifest mismatch".to_owned());
    }
    if !is_sha256(&observation.production_artifact_sha256) {
        return Err("latency observation has an invalid production artifact digest".to_owned());
    }
    let observed_runner = canonical_json(&observation.runner_observation)?;
    if sha256(&observed_runner) != manifest.runner_manifest_sha256 {
        return Err("runner observation mismatch; comparison aborted".to_owned());
    }
    verify_interaction_observation(&interaction_manifest(), &observation.interaction)
        .map_err(|error| format!("latency semantic preflight failed: {error}"))?;

    if observation.metrics.len() != manifest.metrics.len() {
        return Err("latency observation does not contain the exact metric set".to_owned());
    }
    let observed = observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric))
        .collect::<BTreeMap<_, _>>();
    if observed.len() != manifest.metrics.len() {
        return Err("latency observation contains duplicate metrics".to_owned());
    }

    for metric in &manifest.metrics {
        let observation = observed
            .get(metric.id.as_str())
            .ok_or_else(|| format!("latency observation is missing {}", metric.id))?;
        let samples = validate_latency_runs(manifest, metric, &observation.runs)?;
        let p50 = nearest_rank(&samples, 50)?;
        let p95 = nearest_rank(&samples, 95)?;
        let p99 = nearest_rank(&samples, 99)?;
        if observation.p50_ms != p50 || observation.p95_ms != p95 || observation.p99_ms != p99 {
            return Err(format!(
                "latency summary for {} does not match its raw samples",
                metric.id
            ));
        }
        if metric.fixed_edit_gate && (p95 > 16.0 || p99 > 32.0) {
            return Err(format!(
                "fixed O500 scalar-edit gate failed for {}: p95={p95} ms p99={p99} ms",
                metric.id
            ));
        }
        if metric.ceiling_p95_ms.is_some_and(|ceiling| p95 > ceiling)
            || metric.ceiling_p99_ms.is_some_and(|ceiling| p99 > ceiling)
        {
            return Err(format!(
                "latency ceiling failed for {}: p95={p95} ms p99={p99} ms",
                metric.id
            ));
        }
    }
    Ok(())
}

pub fn calibrate_latency_manifest(
    manifest: &LatencyManifest,
    observation: &LatencyObservation,
) -> Result<LatencyManifest, String> {
    if manifest.calibration_observation_sha256.is_some() {
        return Err("latency ceilings are already calibrated".to_owned());
    }
    verify_latency_observation(manifest, observation)?;
    let observed = observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric))
        .collect::<BTreeMap<_, _>>();
    let mut calibrated = manifest.clone();
    for metric in &mut calibrated.metrics {
        if metric.fixed_edit_gate {
            continue;
        }
        let samples = validate_latency_runs(
            manifest,
            metric,
            &observed
                .get(metric.id.as_str())
                .expect("verification checked the exact metric set")
                .runs,
        )?;
        let p95 = calibrated_latency_ceiling(nearest_rank(&samples, 95)?)?;
        let p99 = calibrated_latency_ceiling(nearest_rank(&samples, 99)?)?;
        if p95 > metric.safety_cap_p95_ms || p99 > metric.safety_cap_p99_ms {
            return Err(format!(
                "{} cannot calibrate below its absolute safety cap",
                metric.id
            ));
        }
        metric.ceiling_p95_ms = Some(p95);
        metric.ceiling_p99_ms = Some(p99);
    }
    calibrated.calibration_observation_sha256 = Some(sha256(&canonical_json(observation)?));
    validate_latency_manifest(&calibrated)?;
    Ok(calibrated)
}

pub fn nearest_rank(samples: &[f64], percentile: u8) -> Result<f64, String> {
    if samples.is_empty() || !(1..=100).contains(&percentile) {
        return Err("nearest-rank requires samples and a percentile from 1 through 100".to_owned());
    }
    if samples
        .iter()
        .any(|sample| !sample.is_finite() || *sample < 0.0)
    {
        return Err("latency samples must be finite and nonnegative".to_owned());
    }
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    let rank = (ordered.len() * usize::from(percentile)).div_ceil(100);
    Ok(ordered[rank - 1])
}

pub fn calibrated_latency_ceiling(baseline_ms: f64) -> Result<f64, String> {
    if !baseline_ms.is_finite() || baseline_ms < 0.0 {
        return Err("latency baseline must be finite and nonnegative".to_owned());
    }
    let ceiling = (baseline_ms * 1.25 * 2.0).ceil() / 2.0;
    if ceiling.is_finite() {
        Ok(ceiling)
    } else {
        Err("calibrated latency ceiling overflowed".to_owned())
    }
}

pub fn verify_resource_observation(
    manifest: &ResourceManifest,
    observation: &ResourceObservation,
) -> Result<(), String> {
    validate_resource_manifest(manifest)?;
    if observation.version != manifest.version
        || observation.workflow_run_attempt != 1
        || observation.workload_manifest_sha256 != manifest.workload_manifest_sha256
        || observation.scenario != manifest.memory_protocol.scenario
    {
        return Err("resource observation does not match the contract".to_owned());
    }
    if !observation.environment_sanity_passed {
        return Err("resource environment sanity did not pass".to_owned());
    }
    if observation.discretionary_retries != 0 || !observation.waivers.is_empty() {
        return Err("resource observations may not retry or contain waivers".to_owned());
    }
    let observed_runner = canonical_json(&observation.runner_observation)?;
    if sha256(&observed_runner) != manifest.runner_manifest_sha256 {
        return Err("runner observation mismatch; comparison aborted".to_owned());
    }
    validate_resource_operations(manifest, observation)?;
    let expected_artifacts = manifest
        .artifacts
        .iter()
        .map(|artifact| (artifact.id.as_str(), artifact))
        .collect::<BTreeMap<_, _>>();
    if observation.artifacts.len() != expected_artifacts.len() {
        return Err("resource observation does not contain the exact artifact set".to_owned());
    }
    let artifacts = observation
        .artifacts
        .iter()
        .map(|artifact| (artifact.id.as_str(), artifact))
        .collect::<BTreeMap<_, _>>();
    if artifacts.len() != expected_artifacts.len() {
        return Err("resource observation contains duplicate artifacts".to_owned());
    }
    for (id, expected) in expected_artifacts {
        let artifact = artifacts
            .get(id)
            .ok_or_else(|| format!("resource observation is missing artifact {id}"))?;
        if artifact.path != expected.path || !is_sha256(&artifact.sha256) {
            return Err(format!(
                "resource artifact {id} does not match the contract"
            ));
        }
    }

    let expected_metrics = resource_metric_values(observation, &artifacts)?;
    if observation.metrics.len() != manifest.metrics.len() {
        return Err("resource observation does not contain the exact metric set".to_owned());
    }
    let metrics = observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric.bytes))
        .collect::<BTreeMap<_, _>>();
    if metrics.len() != manifest.metrics.len() {
        return Err("resource observation contains duplicate metrics".to_owned());
    }
    for metric in &manifest.metrics {
        let measured = metrics
            .get(metric.id.as_str())
            .ok_or_else(|| format!("resource observation is missing metric {}", metric.id))?;
        let expected = expected_metrics
            .get(metric.id.as_str())
            .expect("the contract metric set is complete");
        if measured != expected {
            return Err(format!(
                "resource summary for {} does not match its raw evidence",
                metric.id
            ));
        }
        if *measured > metric.safety_cap_bytes {
            return Err(format!("resource absolute cap failed for {}", metric.id));
        }
        if metric
            .ceiling_bytes
            .is_some_and(|ceiling| *measured > ceiling)
        {
            return Err(format!("resource ceiling failed for {}", metric.id));
        }
    }
    Ok(())
}

pub fn calibrate_resource_manifest(
    manifest: &ResourceManifest,
    observation: &ResourceObservation,
    latency_manifest: &LatencyManifest,
    latency_observation: &LatencyObservation,
) -> Result<ResourceManifest, String> {
    if manifest.calibration_observation_sha256.is_some() {
        return Err("resource ceilings are already calibrated".to_owned());
    }
    if latency_manifest.calibration_observation_sha256.is_some() {
        verify_latency_observation(latency_manifest, latency_observation)
            .map_err(|error| format!("resource calibration latency preflight failed: {error}"))?;
    } else {
        calibrate_latency_manifest(latency_manifest, latency_observation)
            .map(|_| ())
            .map_err(|error| {
                format!("resource calibration latency tracer is not calibratable: {error}")
            })?;
    }
    verify_resource_observation(manifest, observation)?;
    let product_wasm = observation
        .artifacts
        .iter()
        .find(|artifact| artifact.id == "production-wasm")
        .expect("resource verification checked the exact artifact set");
    if product_wasm.sha256 != latency_observation.production_artifact_sha256 {
        return Err("latency and resource evidence used different production WASM".to_owned());
    }
    let measured = observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric.bytes))
        .collect::<BTreeMap<_, _>>();
    let mut calibrated = manifest.clone();
    for metric in &mut calibrated.metrics {
        let ceiling = calibrated_resource_ceiling(
            *measured
                .get(metric.id.as_str())
                .expect("resource verification checked every metric"),
            metric.calibration_rounding_bytes,
        )?;
        if ceiling > metric.safety_cap_bytes {
            return Err(format!(
                "{} cannot calibrate below its absolute safety cap",
                metric.id
            ));
        }
        metric.ceiling_bytes = Some(ceiling);
    }
    calibrated.calibration_observation_sha256 = Some(sha256(&canonical_json(&(
        observation,
        latency_observation,
    ))?));
    validate_resource_manifest(&calibrated)?;
    Ok(calibrated)
}

pub fn calibrated_resource_ceiling(
    baseline_bytes: u64,
    rounding_bytes: u64,
) -> Result<u64, String> {
    if rounding_bytes == 0 {
        return Err("resource calibration rounding must be positive".to_owned());
    }
    let scaled = (u128::from(baseline_bytes) * 6).div_ceil(5);
    let rounding = u128::from(rounding_bytes);
    let rounded = scaled.div_ceil(rounding) * rounding;
    u64::try_from(rounded).map_err(|_| "calibrated resource ceiling overflowed".to_owned())
}

fn validate_resource_manifest(manifest: &ResourceManifest) -> Result<(), String> {
    if manifest.version != PACK_VERSION
        || manifest.runner != "schemaform-perf-v1"
        || manifest.browser != InteractionBrowser::Chromium
        || manifest.memory_protocol != memory_protocol()
        || !is_sha256(&manifest.runner_manifest_sha256)
        || !is_sha256(&manifest.workload_manifest_sha256)
        || manifest.artifacts != resource_artifacts()
    {
        return Err("resource manifest does not declare the settled protocol".to_owned());
    }
    if manifest
        .calibration_observation_sha256
        .as_deref()
        .is_some_and(|digest| !is_sha256(digest))
    {
        return Err("resource manifest has an invalid calibration digest".to_owned());
    }
    let expected = resource_metrics();
    if manifest.metrics.len() != expected.len() {
        return Err("resource manifest does not contain the exact metric set".to_owned());
    }
    for (metric, expected) in manifest.metrics.iter().zip(expected) {
        if metric.id != expected.id
            || metric.kind != expected.kind
            || metric.safety_cap_bytes != expected.safety_cap_bytes
            || metric.calibration_rounding_bytes != expected.calibration_rounding_bytes
            || metric
                .ceiling_bytes
                .is_some_and(|ceiling| ceiling > metric.safety_cap_bytes)
        {
            return Err(format!(
                "resource metric {} changed the settled contract",
                metric.id
            ));
        }
    }
    let calibrated = manifest.calibration_observation_sha256.is_some();
    if manifest
        .metrics
        .iter()
        .any(|metric| metric.ceiling_bytes.is_some() != calibrated)
    {
        return Err("resource ceilings must be calibrated together exactly once".to_owned());
    }
    Ok(())
}

fn validate_resource_operations(
    manifest: &ResourceManifest,
    observation: &ResourceObservation,
) -> Result<(), String> {
    if observation.operations.len() != manifest.memory_protocol.operations
        || observation.wasm_memory_before_bytes == 0
        || !observation.wasm_memory_before_bytes.is_multiple_of(65_536)
    {
        return Err("resource observation does not contain the exact WASM sample set".to_owned());
    }
    let cycle = &manifest.memory_protocol.operation_cycle;
    for (sequence, sample) in observation.operations.iter().enumerate() {
        let workload = &cycle[sequence % cycle.len()];
        let phases = manifest.memory_protocol.operation_phases[workload];
        if sample.sequence != sequence
            || &sample.workload != workload
            || sample.phase != (sequence / cycle.len()) % phases
            || sample.wasm_memory_bytes == 0
            || sample.wasm_memory_bytes % 65_536 != 0
        {
            return Err(format!(
                "resource operation {sequence} changed the mixed workload"
            ));
        }
    }
    Ok(())
}

fn resource_metric_values(
    observation: &ResourceObservation,
    artifacts: &BTreeMap<&str, &ResourceArtifactObservation>,
) -> Result<BTreeMap<&'static str, u64>, String> {
    let wasm_high_water = observation
        .operations
        .iter()
        .map(|sample| sample.wasm_memory_bytes)
        .chain([observation.wasm_memory_before_bytes])
        .max()
        .expect("the memory protocol contains samples");
    let product_wasm = artifacts["production-brotli-wasm"].bytes;
    let shell_wasm = artifacts["empty-shell-brotli-wasm"].bytes;
    let incremental_wasm = product_wasm.checked_sub(shell_wasm).ok_or_else(|| {
        "empty-shell Brotli WASM is larger than the production artifact".to_owned()
    })?;
    Ok(BTreeMap::from([
        ("wasm-linear-high-water", wasm_high_water),
        (
            "browser-heap-post-gc-delta",
            observation
                .browser_heap_after_bytes
                .saturating_sub(observation.browser_heap_before_bytes),
        ),
        ("brotli-wasm-total", product_wasm),
        ("brotli-wasm-incremental", incremental_wasm),
        (
            "brotli-runtime-javascript-total",
            artifacts["production-brotli-javascript"].bytes,
        ),
    ]))
}

fn validate_artifact_manifest(manifest: &ArtifactManifest) -> Result<(), String> {
    if manifest != &artifact_manifest() {
        return Err(
            "artifact manifest does not declare the fixed first-release contract".to_owned(),
        );
    }
    Ok(())
}

fn verify_artifact_contents(
    manifest: &ArtifactManifest,
    mut read_artifact: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    validate_artifact_manifest(manifest)?;
    let mut bytes = BTreeMap::new();
    for artifact in &manifest.artifacts {
        let contents = read_artifact(&artifact.path)?;
        match artifact.id.as_str() {
            "production-wasm" | "empty-shell-wasm" if !contents.starts_with(b"\0asm") => {
                return Err(format!("artifact {} is not WebAssembly", artifact.id));
            }
            "production-javascript" | "empty-shell-javascript"
                if contents.is_empty() || std::str::from_utf8(&contents).is_err() =>
            {
                return Err(format!("artifact {} is not UTF-8 JavaScript", artifact.id));
            }
            "production-brotli-wasm"
            | "production-brotli-javascript"
            | "empty-shell-brotli-wasm"
                if contents.is_empty() =>
            {
                return Err(format!("artifact {} is empty", artifact.id));
            }
            _ => {}
        }
        bytes.insert(artifact.id.as_str(), contents);
    }
    let production_wasm = bytes["production-brotli-wasm"].len() as u64;
    let shell_wasm = bytes["empty-shell-brotli-wasm"].len() as u64;
    let measured = BTreeMap::from([
        ("brotli-wasm-total", production_wasm),
        (
            "brotli-wasm-incremental",
            production_wasm.checked_sub(shell_wasm).ok_or_else(|| {
                "empty-shell Brotli WASM is larger than the production artifact".to_owned()
            })?,
        ),
        (
            "brotli-runtime-javascript-total",
            bytes["production-brotli-javascript"].len() as u64,
        ),
    ]);
    for metric in &manifest.metrics {
        if measured[metric.id.as_str()] > metric.safety_cap_bytes {
            return Err(format!("artifact size cap failed for {}", metric.id));
        }
    }
    Ok(())
}

fn verify_resource_input_files(root: &Path, manifest: &ResourceManifest) -> Result<(), String> {
    for (name, expected) in [
        ("manifest.json", manifest.workload_manifest_sha256.as_str()),
        (
            "runner-manifest.json",
            manifest.runner_manifest_sha256.as_str(),
        ),
    ] {
        let path = root.join(name);
        let contents =
            fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
        if sha256(&contents) != expected {
            return Err(format!("{name} digest mismatch; comparison aborted"));
        }
    }
    Ok(())
}

fn verify_resource_artifact_files(
    root: &Path,
    manifest: &ResourceManifest,
    observation: &ResourceObservation,
) -> Result<(), String> {
    let workspace = workspace_root(root)?;
    verify_resource_artifact_contents(manifest, observation, |relative| {
        let path = workspace.join(relative);
        fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))
    })
}

fn verify_resource_artifact_contents(
    manifest: &ResourceManifest,
    observation: &ResourceObservation,
    mut read_artifact: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    for expected in &manifest.artifacts {
        let observed = observation
            .artifacts
            .iter()
            .find(|artifact| artifact.id == expected.id)
            .ok_or_else(|| format!("resource observation is missing artifact {}", expected.id))?;
        let bytes = read_artifact(&expected.path)?;
        if observed.bytes != bytes.len() as u64 || observed.sha256 != sha256(&bytes) {
            return Err(format!(
                "resource artifact {} digest or size mismatch",
                expected.id
            ));
        }
    }
    Ok(())
}

fn required_evidence_files() -> Vec<(String, String)> {
    let mut files = vec![(
        "testing/browser/artifacts/source.bundle".to_owned(),
        "source-bundle".to_owned(),
    )];
    for stem in [
        "manifest",
        "runner-manifest",
        "interaction-manifest",
        "artifact-manifest",
    ] {
        files.push((
            format!("{WORKLOAD_PACK_PATH}/{stem}.json"),
            "contract-manifest".to_owned(),
        ));
        files.push((
            format!("{WORKLOAD_PACK_PATH}/{stem}.sha256"),
            "contract-digest".to_owned(),
        ));
    }
    files.push((
        "testing/browser/artifacts/interaction-observation.json".to_owned(),
        "browser-traces-accessibility".to_owned(),
    ));
    for (profile, controls, rows, controls_per_row) in PROFILES {
        for variant in VARIANTS {
            for state in STATES {
                let bytes = canonical_json(&scenario(
                    profile,
                    controls,
                    rows,
                    controls_per_row,
                    variant,
                    state,
                ))
                .expect("generated workload scenarios serialize");
                files.push((
                    format!(
                        "{WORKLOAD_PACK_PATH}/objects/sha256/{}.json",
                        sha256(&bytes)
                    ),
                    "workload-fixture".to_owned(),
                ));
            }
        }
    }
    files.extend(
        interaction_manifest()
            .cells
            .iter()
            .flat_map(interaction_artifacts)
            .map(|artifact| {
                (
                    artifact.path,
                    match artifact.kind {
                        InteractionArtifactKind::Trace => "browser-trace",
                        InteractionArtifactKind::Screenshot => "browser-screenshot",
                        InteractionArtifactKind::AccessibilityReport => "accessibility-report",
                        InteractionArtifactKind::BrowserLog => "browser-log",
                    }
                    .to_owned(),
                )
            }),
    );
    files.extend(
        resource_artifacts()
            .into_iter()
            .map(|artifact| (artifact.path, "optimized-artifact".to_owned())),
    );
    files
}

fn expected_conclusion_evidence(
    objects: &BTreeMap<&str, &str>,
) -> BTreeMap<&'static str, Vec<String>> {
    let digest = |name: &str| objects.get(name).map(|digest| (*digest).to_owned());
    let mut artifact_evidence = [
        "testing/browser/workload-pack/artifact-manifest.json",
        "testing/browser/workload-pack/artifact-manifest.sha256",
    ]
    .into_iter()
    .filter_map(digest)
    .collect::<Vec<_>>();
    artifact_evidence.extend(
        resource_artifacts()
            .into_iter()
            .filter_map(|artifact| digest(&artifact.path)),
    );
    let mut interaction_evidence = [
        "testing/browser/workload-pack/runner-manifest.json",
        "testing/browser/workload-pack/runner-manifest.sha256",
        "testing/browser/workload-pack/interaction-manifest.json",
        "testing/browser/workload-pack/interaction-manifest.sha256",
        "testing/browser/artifacts/interaction-observation.json",
    ]
    .into_iter()
    .filter_map(digest)
    .collect::<Vec<_>>();
    interaction_evidence.extend(
        interaction_manifest()
            .cells
            .iter()
            .flat_map(interaction_artifacts)
            .filter_map(|artifact| digest(&artifact.path)),
    );
    BTreeMap::from([
        (
            "source-tree",
            digest("testing/browser/artifacts/source.bundle")
                .into_iter()
                .collect(),
        ),
        ("interactions-accessibility", interaction_evidence),
        ("artifact-size", artifact_evidence),
    ])
}

fn evidence_conclusion(
    gate: &str,
    result: Result<(), String>,
    evidence_sha256: Vec<String>,
) -> EvidenceConclusion {
    match result {
        Ok(()) => EvidenceConclusion {
            gate: gate.to_owned(),
            status: EvidenceStatus::Passed,
            evidence_sha256,
            details: "passed".to_owned(),
        },
        Err(error) => EvidenceConclusion {
            gate: gate.to_owned(),
            status: EvidenceStatus::Failed,
            evidence_sha256,
            details: error,
        },
    }
}

fn verify_archived_contract(manifest: &EvidenceManifest, root: &Path) -> Result<(), String> {
    let objects = manifest
        .objects
        .iter()
        .map(|object| (object.name.as_str(), object))
        .collect::<BTreeMap<_, _>>();
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let object = objects
            .get(name)
            .ok_or_else(|| format!("evidence archive is missing {name}"))?;
        fs::read(root.join("objects/sha256").join(&object.sha256))
            .map_err(|error| format!("read archived {name}: {error}"))
    };
    let tracked_files = required_evidence_files()
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| name.starts_with(WORKLOAD_PACK_PATH))
        .map(|name| read(&name).map(|bytes| (name, bytes)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    verify_source_bundle(
        &read("testing/browser/artifacts/source.bundle")?,
        &manifest.source_commit,
        &manifest.source_tree,
        &tracked_files,
    )?;
    for stem in [
        "manifest",
        "runner-manifest",
        "interaction-manifest",
        "artifact-manifest",
    ] {
        let name = format!("{WORKLOAD_PACK_PATH}/{stem}.json");
        let bytes = read(&name)?;
        let digest_name = format!("{WORKLOAD_PACK_PATH}/{stem}.sha256");
        if read(&digest_name)? != sidecar(&format!("{stem}.json"), &bytes) {
            return Err(format!("archived {stem} sidecar mismatch"));
        }
    }
    let runner: Value =
        serde_json::from_slice(&read("testing/browser/workload-pack/runner-manifest.json")?)
            .map_err(|error| format!("parse archived runner manifest: {error}"))?;
    verify_runner_observation(&runner_manifest(), &runner)
        .map_err(|error| format!("archived runner contract mismatch: {error}"))?;
    let interactions: InteractionManifest = serde_json::from_slice(&read(
        "testing/browser/workload-pack/interaction-manifest.json",
    )?)
    .map_err(|error| format!("parse archived interaction manifest: {error}"))?;
    let interaction_observation: InteractionObservation = serde_json::from_slice(&read(
        "testing/browser/artifacts/interaction-observation.json",
    )?)
    .map_err(|error| format!("parse archived interaction observation: {error}"))?;
    verify_interaction_observation(&interactions, &interaction_observation)?;
    verify_interaction_artifact_contents(&interactions, &interaction_observation, &read)?;

    let artifacts: ArtifactManifest = serde_json::from_slice(&read(
        "testing/browser/workload-pack/artifact-manifest.json",
    )?)
    .map_err(|error| format!("parse archived artifact manifest: {error}"))?;
    verify_artifact_contents(&artifacts, &read)?;
    Ok(())
}

fn is_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn git_output(root: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .map_err(|error| format!("run git {}: {error}", arguments.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("git output is not UTF-8: {error}"))
}

fn git_clean(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("inspect git worktree: {error}"))?;
    if output.status.success() && output.stdout.is_empty() {
        Ok(())
    } else {
        Err("source worktree contains tracked or untracked changes".to_owned())
    }
}

fn workflow_run_attempt() -> Result<u64, String> {
    let value = std::env::var("QUALIFICATION_ATTEMPT")
        .or_else(|_| std::env::var("GITHUB_RUN_ATTEMPT"))
        .unwrap_or_else(|_| "1".to_owned());
    let attempt = value
        .parse::<u64>()
        .map_err(|error| format!("parse workflow run attempt: {error}"))?;
    if attempt == 0 {
        Err("workflow run attempt must be positive".to_owned())
    } else {
        Ok(attempt)
    }
}

fn create_source_bundle(root: &Path) -> Result<Vec<u8>, String> {
    let path = std::env::temp_dir().join(format!(
        "schemaform-source-{}-{}.bundle",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock predates the Unix epoch".to_owned())?
            .as_nanos()
    ));
    let status = Command::new("git")
        .args(["bundle", "create"])
        .arg(&path)
        .arg("HEAD")
        .current_dir(root)
        .status()
        .map_err(|error| format!("create source bundle: {error}"))?;
    if !status.success() {
        return Err("create source bundle: git bundle failed".to_owned());
    }
    let bytes = fs::read(&path).map_err(|error| format!("read source bundle: {error}"));
    let cleanup = fs::remove_file(&path).map_err(|error| format!("remove source bundle: {error}"));
    let bytes = bytes?;
    cleanup?;
    Ok(bytes)
}

fn verify_source_bundle(
    bytes: &[u8],
    commit: &str,
    tree: &str,
    tracked_files: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let temp = std::env::temp_dir().join(format!(
        "schemaform-source-verify-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock predates the Unix epoch".to_owned())?
            .as_nanos()
    ));
    fs::create_dir(&temp).map_err(|error| format!("create source verification dir: {error}"))?;
    let result = (|| {
        let bundle = temp.join("source.bundle");
        let repository = temp.join("repository.git");
        fs::write(&bundle, bytes).map_err(|error| format!("write source bundle: {error}"))?;
        let status = Command::new("git")
            .args(["clone", "--bare", "--quiet"])
            .arg(&bundle)
            .arg(&repository)
            .status()
            .map_err(|error| format!("clone source bundle: {error}"))?;
        if !status.success() {
            return Err("archived source bundle is invalid".to_owned());
        }
        if git_output(&repository, &["rev-parse", "HEAD"])? != commit
            || git_output(&repository, &["rev-parse", "HEAD^{tree}"])? != tree
        {
            return Err(
                "archived source bundle does not match the declared commit and tree".to_owned(),
            );
        }
        let fsck = Command::new("git")
            .args(["fsck", "--full"])
            .current_dir(&repository)
            .status()
            .map_err(|error| format!("verify source bundle objects: {error}"))?;
        if !fsck.success() {
            return Err("archived source bundle has missing or invalid objects".to_owned());
        }
        for (name, expected) in tracked_files {
            let output = Command::new("git")
                .args(["show", &format!("HEAD:{name}")])
                .current_dir(&repository)
                .output()
                .map_err(|error| format!("read {name} from source bundle: {error}"))?;
            if !output.status.success() || output.stdout != *expected {
                return Err(format!(
                    "archived evidence {name} does not match the declared source tree"
                ));
            }
        }
        Ok(())
    })();
    let cleanup = fs::remove_dir_all(&temp)
        .map_err(|error| format!("remove source verification dir: {error}"));
    result?;
    cleanup
}

fn validate_latency_manifest(manifest: &LatencyManifest) -> Result<(), String> {
    if manifest.version != PACK_VERSION
        || manifest.runner != "schemaform-perf-v1"
        || manifest.browser != InteractionBrowser::Chromium
        || manifest.protocol != latency_protocol()
        || !is_sha256(&manifest.runner_manifest_sha256)
        || !is_sha256(&manifest.workload_manifest_sha256)
    {
        return Err("latency manifest does not declare the settled protocol".to_owned());
    }
    if manifest
        .calibration_observation_sha256
        .as_deref()
        .is_some_and(|digest| !is_sha256(digest))
    {
        return Err("latency manifest has an invalid calibration digest".to_owned());
    }
    let expected = latency_metrics();
    if manifest.metrics.len() != expected.len() {
        return Err("latency manifest does not contain the exact metric set".to_owned());
    }
    for (metric, expected) in manifest.metrics.iter().zip(expected) {
        if metric.id != expected.id
            || metric.scenario != expected.scenario
            || metric.workload != expected.workload
            || metric.cold != expected.cold
            || metric.phases != expected.phases
            || metric.fixed_edit_gate != expected.fixed_edit_gate
            || metric.safety_cap_p95_ms != expected.safety_cap_p95_ms
            || metric.safety_cap_p99_ms != expected.safety_cap_p99_ms
        {
            return Err(format!(
                "latency metric {} changed the settled contract",
                metric.id
            ));
        }
        if metric.fixed_edit_gate {
            if metric.ceiling_p95_ms != Some(16.0) || metric.ceiling_p99_ms != Some(32.0) {
                return Err(format!("{} changed the fixed O500 edit gate", metric.id));
            }
        } else {
            for (ceiling, cap) in [
                (metric.ceiling_p95_ms, metric.safety_cap_p95_ms),
                (metric.ceiling_p99_ms, metric.safety_cap_p99_ms),
            ] {
                if ceiling.is_some_and(|value| !value.is_finite() || value < 0.0 || value > cap) {
                    return Err(format!("{} exceeds its absolute safety cap", metric.id));
                }
            }
        }
    }
    let calibrated = manifest.calibration_observation_sha256.is_some();
    if manifest
        .metrics
        .iter()
        .filter(|metric| !metric.fixed_edit_gate)
        .any(|metric| {
            metric.ceiling_p95_ms.is_some() != calibrated
                || metric.ceiling_p99_ms.is_some() != calibrated
        })
    {
        return Err("latency ceilings must be calibrated together exactly once".to_owned());
    }
    Ok(())
}

fn verify_latency_input_files(root: &Path, manifest: &LatencyManifest) -> Result<(), String> {
    for (name, expected) in [
        ("manifest.json", manifest.workload_manifest_sha256.as_str()),
        (
            "runner-manifest.json",
            manifest.runner_manifest_sha256.as_str(),
        ),
    ] {
        let path = root.join(name);
        let contents =
            fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
        if sha256(&contents) != expected {
            return Err(format!("{name} digest mismatch; comparison aborted"));
        }
    }
    Ok(())
}

fn validate_latency_runs(
    manifest: &LatencyManifest,
    metric: &LatencyMetric,
    runs: &LatencyRuns,
) -> Result<Vec<f64>, String> {
    let mut durations = Vec::new();
    match (metric.cold, runs) {
        (true, LatencyRuns::Cold { contexts }) => {
            if contexts.len() != manifest.protocol.cold_context_samples {
                return Err(format!("{} does not contain 100 cold contexts", metric.id));
            }
            for (index, context) in contexts.iter().enumerate() {
                if context.context != index
                    || !context.fresh_context
                    || context.sample.sequence != 0
                    || context.sample.phase != 0
                {
                    return Err(format!(
                        "{} contains a nonconforming cold context",
                        metric.id
                    ));
                }
                durations.push(context.sample.duration_ms);
            }
        }
        (false, LatencyRuns::Hot { processes }) => {
            if processes.len() != manifest.protocol.hot_processes {
                return Err(format!(
                    "{} does not contain five fresh processes",
                    metric.id
                ));
            }
            for (index, process) in processes.iter().enumerate() {
                if process.process != index
                    || !process.fresh_process
                    || process.warmups_completed != manifest.protocol.warmups_per_process
                    || process.samples.len() != manifest.protocol.samples_per_process
                {
                    return Err(format!(
                        "{} contains a nonconforming hot process",
                        metric.id
                    ));
                }
                for (sequence, sample) in process.samples.iter().enumerate() {
                    if sample.sequence != sequence || sample.phase != sequence % metric.phases {
                        return Err(format!("{} contains a non-alternating sample", metric.id));
                    }
                    durations.push(sample.duration_ms);
                }
            }
        }
        _ => return Err(format!("{} used the wrong hot/cold protocol", metric.id)),
    }
    if durations
        .iter()
        .any(|duration| !duration.is_finite() || *duration < 0.0)
    {
        return Err(format!("{} contains an invalid duration", metric.id));
    }
    Ok(durations)
}

fn latency_protocol() -> LatencyProtocol {
    LatencyProtocol {
        hot_processes: LATENCY_HOT_PROCESSES,
        warmups_per_process: LATENCY_WARMUPS_PER_PROCESS,
        samples_per_process: LATENCY_SAMPLES_PER_PROCESS,
        cold_context_samples: LATENCY_COLD_CONTEXT_SAMPLES,
        percentile_method: "nearest-rank".to_owned(),
        percentiles: vec![50, 95, 99],
        timing_boundary: "performance.now() at handler entry through the committed DOM state-revision sentinel for edits or commit token for other operations".to_owned(),
        fresh_processes: true,
        fresh_cold_contexts: true,
        outlier_deletion_allowed: false,
        discretionary_retries_allowed: false,
    }
}

fn latency_metrics() -> Vec<LatencyMetric> {
    let mut metrics = Vec::new();
    for (profile, controls, ..) in PROFILES {
        for variant in VARIANTS {
            for state in STATES {
                let scenario = format!("{profile}-{variant}-{state}");
                for workload in [
                    LatencyWorkload::Compilation,
                    LatencyWorkload::Mount,
                    LatencyWorkload::Edit,
                    LatencyWorkload::Findings,
                    LatencyWorkload::Visibility,
                    LatencyWorkload::Localization,
                    LatencyWorkload::Submission,
                ]
                .into_iter()
                .chain((profile == "A100x5").then_some(LatencyWorkload::Arrays))
                {
                    let cold = workload.is_cold();
                    let (safety_cap_p95_ms, safety_cap_p99_ms) = workload.safety_caps(controls);
                    let fixed_edit_gate = profile == "O500" && workload == LatencyWorkload::Edit;
                    metrics.push(LatencyMetric {
                        id: format!("{scenario}/{}", workload.as_str()),
                        scenario: scenario.clone(),
                        workload,
                        cold,
                        phases: workload.phases(),
                        fixed_edit_gate,
                        safety_cap_p95_ms,
                        safety_cap_p99_ms,
                        ceiling_p95_ms: fixed_edit_gate.then_some(16.0),
                        ceiling_p99_ms: fixed_edit_gate.then_some(32.0),
                    });
                }
            }
        }
    }
    metrics
}

fn apply_latency_calibration(manifest: &mut LatencyManifest) -> Result<(), String> {
    let calibration: LatencyCalibration = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/latency-calibration.json"
    )))
    .map_err(|error| format!("parse latency-calibration.json: {error}"))?;
    apply_latency_calibration_source(manifest, &calibration)
}

fn apply_latency_calibration_source(
    manifest: &mut LatencyManifest,
    calibration: &LatencyCalibration,
) -> Result<(), String> {
    let Some(observation_sha256) = calibration.observation_sha256.as_ref() else {
        if calibration.observation.is_none() && calibration.ceilings.is_empty() {
            return Ok(());
        }
        return Err("uncalibrated latency source contains ceilings".to_owned());
    };
    if !is_sha256(observation_sha256) {
        return Err("latency calibration source has an invalid observation digest".to_owned());
    }
    let observation = calibration
        .observation
        .as_ref()
        .ok_or_else(|| "latency calibration source has no raw observation".to_owned())?;
    if sha256(&canonical_json(observation)?) != *observation_sha256 {
        return Err("latency calibration observation digest mismatch".to_owned());
    }
    verify_latency_observation(manifest, observation)
        .map_err(|error| format!("latency calibration observation is invalid: {error}"))?;
    let observed = observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric))
        .collect::<BTreeMap<_, _>>();
    let ceilings = calibration
        .ceilings
        .iter()
        .map(|ceiling| (ceiling.id.as_str(), ceiling))
        .collect::<BTreeMap<_, _>>();
    let missing = manifest
        .metrics
        .iter()
        .filter(|metric| !metric.fixed_edit_gate)
        .count();
    if ceilings.len() != missing || calibration.ceilings.len() != missing {
        return Err("latency calibration source does not contain every missing metric".to_owned());
    }
    for metric in manifest
        .metrics
        .iter_mut()
        .filter(|metric| !metric.fixed_edit_gate)
    {
        let ceiling = ceilings
            .get(metric.id.as_str())
            .ok_or_else(|| format!("latency calibration is missing {}", metric.id))?;
        if calibrated_latency_ceiling(ceiling.baseline_p95_ms)? != ceiling.p95_ms
            || calibrated_latency_ceiling(ceiling.baseline_p99_ms)? != ceiling.p99_ms
            || ceiling.baseline_p95_ms != observed[metric.id.as_str()].p95_ms
            || ceiling.baseline_p99_ms != observed[metric.id.as_str()].p99_ms
        {
            return Err(format!(
                "latency calibration for {} does not match its baseline",
                metric.id
            ));
        }
        metric.ceiling_p95_ms = Some(ceiling.p95_ms);
        metric.ceiling_p99_ms = Some(ceiling.p99_ms);
    }
    manifest.calibration_observation_sha256 = Some(observation_sha256.clone());
    validate_latency_manifest(manifest)
}

fn memory_protocol() -> MemoryProtocol {
    MemoryProtocol {
        scenario: "A100x5-authored-64-finding".to_owned(),
        operations: RESOURCE_OPERATIONS,
        operation_cycle: [
            "edit",
            "findings",
            "visibility",
            "arrays",
            "localization",
            "submission",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        operation_phases: BTreeMap::from([
            ("arrays".to_owned(), 6),
            ("edit".to_owned(), 2),
            ("findings".to_owned(), 2),
            ("localization".to_owned(), 2),
            ("submission".to_owned(), 2),
            ("visibility".to_owned(), 2),
        ]),
        wasm_sampling: "WebAssembly.Memory.buffer.byteLength after mount and every committed operation; report the maximum".to_owned(),
        browser_heap_measurement: "Chromium CDP HeapProfiler.collectGarbage followed by Performance.getMetrics JSHeapUsedSize after mount and after the settled workload".to_owned(),
        settle_condition: "Each operation observes the Dioxus commit token before memory is sampled"
            .to_owned(),
    }
}

fn resource_metrics() -> Vec<ResourceMetric> {
    [
        (
            "wasm-linear-high-water",
            ResourceMetricKind::Memory,
            128 * MIB,
            MIB,
        ),
        (
            "browser-heap-post-gc-delta",
            ResourceMetricKind::Memory,
            64 * MIB,
            MIB,
        ),
        (
            "brotli-wasm-total",
            ResourceMetricKind::CompressedSize,
            1536 * KIB,
            KIB,
        ),
        (
            "brotli-wasm-incremental",
            ResourceMetricKind::CompressedSize,
            512 * KIB,
            KIB,
        ),
        (
            "brotli-runtime-javascript-total",
            ResourceMetricKind::CompressedSize,
            64 * KIB,
            KIB,
        ),
    ]
    .into_iter()
    .map(
        |(id, kind, safety_cap_bytes, calibration_rounding_bytes)| ResourceMetric {
            id: id.to_owned(),
            kind,
            safety_cap_bytes,
            calibration_rounding_bytes,
            ceiling_bytes: None,
        },
    )
    .collect()
}

fn artifact_metrics() -> Vec<ArtifactMetric> {
    resource_metrics()
        .into_iter()
        .filter(|metric| metric.kind == ResourceMetricKind::CompressedSize)
        .map(|metric| ArtifactMetric {
            id: metric.id,
            safety_cap_bytes: metric.safety_cap_bytes,
        })
        .collect()
}

fn artifact_manifest() -> ArtifactManifest {
    ArtifactManifest {
        version: PACK_VERSION,
        rust_wasm_tools: rust_wasm_tools(),
        compression: compression_contract(),
        pipeline: artifact_pipeline(),
        metrics: artifact_metrics(),
        artifacts: resource_artifacts(),
    }
}

fn resource_artifacts() -> Vec<ResourceArtifact> {
    [
        (
            "production-wasm",
            "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm",
        ),
        (
            "production-javascript",
            "testing/browser/artifacts/bindgen/browser_workload_runner.js",
        ),
        (
            "production-brotli-wasm",
            "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm.br",
        ),
        (
            "production-brotli-javascript",
            "testing/browser/artifacts/bindgen/browser_workload_runner.js.br",
        ),
        (
            "empty-shell-wasm",
            "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm",
        ),
        (
            "empty-shell-javascript",
            "testing/browser/artifacts/empty-shell/browser_workload_empty_shell.js",
        ),
        (
            "empty-shell-brotli-wasm",
            "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm.br",
        ),
    ]
    .into_iter()
    .map(|(id, path)| ResourceArtifact {
        id: id.to_owned(),
        path: path.to_owned(),
    })
    .collect()
}

fn apply_resource_calibration(manifest: &mut ResourceManifest) -> Result<(), String> {
    let calibration: ResourceCalibration = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/resource-calibration.json"
    )))
    .map_err(|error| format!("parse resource-calibration.json: {error}"))?;
    apply_resource_calibration_source(manifest, &calibration)
}

fn apply_resource_calibration_source(
    manifest: &mut ResourceManifest,
    calibration: &ResourceCalibration,
) -> Result<(), String> {
    let Some(observation_sha256) = calibration.observation_sha256.as_ref() else {
        if calibration.resource_observation.is_none()
            && calibration.latency_observation.is_none()
            && calibration.ceilings.is_empty()
        {
            return Ok(());
        }
        return Err("uncalibrated resource source contains ceilings".to_owned());
    };
    if !is_sha256(observation_sha256) {
        return Err("resource calibration source has an invalid observation digest".to_owned());
    }
    let resource_observation = calibration
        .resource_observation
        .as_ref()
        .ok_or_else(|| "resource calibration source has no raw resource observation".to_owned())?;
    let latency_observation = calibration
        .latency_observation
        .as_ref()
        .ok_or_else(|| "resource calibration source has no raw latency observation".to_owned())?;
    if sha256(&canonical_json(&(
        resource_observation,
        latency_observation,
    ))?) != *observation_sha256
    {
        return Err("resource calibration observation digest mismatch".to_owned());
    }
    verify_resource_observation(manifest, resource_observation)
        .map_err(|error| format!("resource calibration observation is invalid: {error}"))?;
    let latency_manifest = LatencyManifest {
        version: PACK_VERSION,
        runner: "schemaform-perf-v1".to_owned(),
        browser: InteractionBrowser::Chromium,
        runner_manifest_sha256: manifest.runner_manifest_sha256.clone(),
        workload_manifest_sha256: manifest.workload_manifest_sha256.clone(),
        protocol: latency_protocol(),
        calibration_observation_sha256: None,
        metrics: latency_metrics(),
    };
    calibrate_latency_manifest(&latency_manifest, latency_observation)
        .map_err(|error| format!("resource calibration latency observation is invalid: {error}"))?;
    let product_wasm = resource_observation
        .artifacts
        .iter()
        .find(|artifact| artifact.id == "production-wasm")
        .expect("resource observation has the exact artifact set");
    if product_wasm.sha256 != latency_observation.production_artifact_sha256 {
        return Err("resource calibration did not use the latency production WASM".to_owned());
    }
    let observed = resource_observation
        .metrics
        .iter()
        .map(|metric| (metric.id.as_str(), metric.bytes))
        .collect::<BTreeMap<_, _>>();
    let ceilings = calibration
        .ceilings
        .iter()
        .map(|ceiling| (ceiling.id.as_str(), ceiling))
        .collect::<BTreeMap<_, _>>();
    if ceilings.len() != manifest.metrics.len()
        || calibration.ceilings.len() != manifest.metrics.len()
    {
        return Err("resource calibration source does not contain every metric".to_owned());
    }
    for metric in &mut manifest.metrics {
        let ceiling = ceilings
            .get(metric.id.as_str())
            .ok_or_else(|| format!("resource calibration is missing {}", metric.id))?;
        if calibrated_resource_ceiling(ceiling.baseline_bytes, metric.calibration_rounding_bytes)?
            != ceiling.bytes
            || ceiling.baseline_bytes != observed[metric.id.as_str()]
        {
            return Err(format!(
                "resource calibration for {} does not match its baseline",
                metric.id
            ));
        }
        metric.ceiling_bytes = Some(ceiling.bytes);
    }
    manifest.calibration_observation_sha256 = Some(observation_sha256.clone());
    validate_resource_manifest(manifest)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn verify_interaction_observation(
    manifest: &InteractionManifest,
    observation: &InteractionObservation,
) -> Result<(), String> {
    if observation.version != manifest.version {
        return Err("interaction observation version mismatch".to_owned());
    }
    if observation.workflow_run_attempt != 1 {
        return Err("interaction evidence came from a retried workflow run".to_owned());
    }
    let required_traces = interaction_traces(manifest);
    let required_checkpoints = manifest
        .accessibility
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.id.as_str())
        .collect::<BTreeSet<_>>();
    let today = utc_date(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock predates the Unix epoch".to_owned())?
            .as_secs()
            / 86_400,
    );
    for exception in &manifest.accessibility.reviewed_exceptions {
        let checkpoint_trace = manifest
            .accessibility
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == exception.checkpoint)
            .map(|checkpoint| checkpoint.trace.as_str());
        if !required_checkpoints.contains(exception.checkpoint.as_str())
            || !exception.defect_url.starts_with("https://")
            || !(exception.defect_url.contains("/issues/") || exception.defect_url.contains("/bug"))
            || !required_traces.contains(exception.compensating_test.as_str())
            || checkpoint_trace == Some(exception.compensating_test.as_str())
            || exception.targets.is_empty()
            || !is_valid_iso_date(&exception.expires_on)
            || exception.expires_on <= today
        {
            return Err(
                "accessibility exception is incomplete or references an unknown checkpoint"
                    .to_owned(),
            );
        }
    }
    let expected = manifest
        .cells
        .iter()
        .map(|cell| {
            (
                (
                    cell.browser,
                    cell.viewport_width_css_pixels,
                    cell.zoom_percent,
                ),
                cell,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if observation.cells.len() != expected.len() {
        return Err("interaction observation does not contain the exact matrix".to_owned());
    }
    let mut observed = BTreeSet::new();
    for cell in &observation.cells {
        let key = (
            cell.browser,
            cell.viewport_width_css_pixels,
            cell.zoom_percent,
        );
        if !observed.insert(key) || !expected.contains_key(&key) {
            return Err(
                "interaction observation contains an unexpected or duplicate cell".to_owned(),
            );
        }
        if cell.status != InteractionStatus::Passed {
            return Err(format!("interaction cell {:?} did not pass", key));
        }
        if cell.effective_viewport_width_css_pixels != cell.viewport_width_css_pixels {
            return Err(format!(
                "interaction cell {:?} used the wrong effective viewport",
                key
            ));
        }
        if cell
            .traces
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != required_traces
        {
            return Err(format!(
                "interaction cell {:?} did not execute every required trace",
                key
            ));
        }
        let expected_artifacts = interaction_artifacts(expected[&key]);
        if cell.artifacts.len() != expected_artifacts.len() {
            return Err(format!(
                "interaction cell {:?} did not retain every raw artifact",
                key
            ));
        }
        let artifacts = cell
            .artifacts
            .iter()
            .map(|artifact| (artifact.kind, artifact))
            .collect::<BTreeMap<_, _>>();
        if artifacts.len() != expected_artifacts.len()
            || expected_artifacts.iter().any(|expected| {
                artifacts.get(&expected.kind).is_none_or(|artifact| {
                    artifact.path != expected.path
                        || !is_sha256(&artifact.sha256)
                        || artifact.bytes == 0
                })
            })
        {
            return Err(format!(
                "interaction cell {:?} has incomplete raw artifact metadata",
                key
            ));
        }
        let observed_checkpoints = cell
            .accessibility
            .iter()
            .map(|checkpoint| checkpoint.id.as_str())
            .collect::<BTreeSet<_>>();
        if cell.accessibility.len() != required_checkpoints.len()
            || observed_checkpoints != required_checkpoints
        {
            return Err(format!(
                "interaction cell {:?} did not scan every accessibility checkpoint exactly once",
                key
            ));
        }
        for checkpoint in &cell.accessibility {
            let expected_checkpoint = manifest
                .accessibility
                .checkpoints
                .iter()
                .find(|expected| expected.id == checkpoint.id)
                .expect("the checkpoint set was verified");
            if checkpoint.trace != expected_checkpoint.trace
                || expected_checkpoint
                    .aria_required
                    .iter()
                    .any(|required| !checkpoint.aria_snapshot.contains(required))
            {
                return Err(format!(
                    "interaction cell {:?} reported incomplete accessibility-tree evidence at {}",
                    key, checkpoint.id
                ));
            }
            for violation in &checkpoint.violations {
                let reviewed = manifest
                    .accessibility
                    .reviewed_exceptions
                    .iter()
                    .any(|exception| {
                        exception.checkpoint == checkpoint.id
                            && exception.rule_id == violation.rule_id
                            && exception.impact == violation.impact
                            && exception.nodes == violation.nodes
                            && exception.targets == violation.targets
                    });
                if !reviewed || violation.nodes == 0 {
                    return Err(format!(
                        "interaction cell {:?} reported unreviewed accessibility violation {} at {}",
                        key, violation.rule_id, checkpoint.id
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit()))
    {
        return false;
    }
    let year = value[0..4].parse::<u16>().expect("digits were checked");
    let month = value[5..7].parse::<u8>().expect("digits were checked");
    let day = value[8..10].parse::<u8>().expect("digits were checked");
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day > 0 && day <= days
}

fn utc_date(days_since_epoch: u64) -> String {
    let days =
        i64::try_from(days_since_epoch).expect("the current date should fit in i64") + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn expected_interaction_observation(manifest: &InteractionManifest) -> InteractionObservation {
    let traces = interaction_traces(manifest)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    InteractionObservation {
        version: manifest.version,
        workflow_run_attempt: 1,
        cells: manifest
            .cells
            .iter()
            .map(|cell| InteractionCellObservation {
                browser: cell.browser,
                viewport_width_css_pixels: cell.viewport_width_css_pixels,
                zoom_percent: cell.zoom_percent,
                effective_viewport_width_css_pixels: cell.viewport_width_css_pixels,
                status: InteractionStatus::Passed,
                traces: traces.clone(),
                accessibility: manifest
                    .accessibility
                    .checkpoints
                    .iter()
                    .map(|checkpoint| AccessibilityCheckpointObservation {
                        id: checkpoint.id.clone(),
                        trace: checkpoint.trace.clone(),
                        aria_snapshot: checkpoint.aria_required.join("\n"),
                        violations: Vec::new(),
                    })
                    .collect(),
                artifacts: interaction_artifacts(cell)
                    .into_iter()
                    .map(|mut artifact| {
                        artifact.sha256 = "a".repeat(64);
                        artifact.bytes = 1;
                        artifact
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn verify_runner_observation(expected: &Value, observed: &Value) -> Result<(), String> {
    if expected != observed {
        return Err("runner observation mismatch; comparison aborted".to_owned());
    }
    Ok(())
}

pub fn read_manifest(root: &Path) -> Result<PackManifest, String> {
    let path = root.join("manifest.json");
    serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))
}

pub fn read_interaction_manifest(root: &Path) -> Result<InteractionManifest, String> {
    let path = root.join("interaction-manifest.json");
    serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))
}

pub fn read_latency_manifest(root: &Path) -> Result<LatencyManifest, String> {
    let path = root.join("latency-manifest.json");
    serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))
}

pub fn read_resource_manifest(root: &Path) -> Result<ResourceManifest, String> {
    let path = root.join("resource-manifest.json");
    serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))
}

pub fn read_scenario(root: &Path, reference: &ScenarioReference) -> Result<Scenario, String> {
    let path = root
        .join("objects/sha256")
        .join(format!("{}.json", reference.object_sha256));
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if sha256(&bytes) != reference.object_sha256 {
        return Err(format!("object digest mismatch: {}", path.display()));
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

pub fn scenario_by_id(id: &str) -> Option<Scenario> {
    for (profile, controls, rows, controls_per_row) in PROFILES {
        for variant in VARIANTS {
            for state in STATES {
                if id == format!("{profile}-{variant}-{state}") {
                    return Some(scenario(
                        profile,
                        controls,
                        rows,
                        controls_per_row,
                        variant,
                        state,
                    ));
                }
            }
        }
    }
    None
}

fn build_files() -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    let mut references = Vec::new();
    for (profile, controls, rows, controls_per_row) in PROFILES {
        for variant in VARIANTS {
            for state in STATES {
                let scenario = scenario(profile, controls, rows, controls_per_row, variant, state);
                let bytes = canonical_json(&scenario)?;
                let digest = sha256(&bytes);
                let workloads = scenario
                    .workloads
                    .iter()
                    .map(Workload::name)
                    .map(str::to_owned)
                    .collect();
                references.push(ScenarioReference {
                    id: scenario.id.clone(),
                    profile: profile.to_owned(),
                    variant: variant.to_owned(),
                    state: state.to_owned(),
                    controls,
                    rows,
                    controls_per_row,
                    object_sha256: digest.clone(),
                    workloads,
                });
                files.insert(
                    PathBuf::from(format!("objects/sha256/{digest}.json")),
                    bytes,
                );
            }
        }
    }
    let manifest = PackManifest {
        version: PACK_VERSION,
        hash_algorithm: "sha256".to_owned(),
        object_directory: "objects/sha256".to_owned(),
        scenarios: references,
    };
    let manifest = canonical_json(&manifest)?;
    let workload_manifest_sha256 = sha256(&manifest);
    files.insert(
        PathBuf::from("manifest.sha256"),
        sidecar("manifest.json", &manifest),
    );
    files.insert(PathBuf::from("manifest.json"), manifest);

    let interactions = canonical_json(&interaction_manifest())?;
    files.insert(
        PathBuf::from("interaction-manifest.sha256"),
        sidecar("interaction-manifest.json", &interactions),
    );
    files.insert(PathBuf::from("interaction-manifest.json"), interactions);

    let runner = canonical_json(&runner_manifest())?;
    let runner_manifest_sha256 = sha256(&runner);
    files.insert(
        PathBuf::from("runner-manifest.sha256"),
        sidecar("runner-manifest.json", &runner),
    );
    files.insert(PathBuf::from("runner-manifest.json"), runner);

    let artifacts = canonical_json(&artifact_manifest())?;
    files.insert(
        PathBuf::from("artifact-manifest.sha256"),
        sidecar("artifact-manifest.json", &artifacts),
    );
    files.insert(PathBuf::from("artifact-manifest.json"), artifacts);

    let mut latency = LatencyManifest {
        version: PACK_VERSION,
        runner: "schemaform-perf-v1".to_owned(),
        browser: InteractionBrowser::Chromium,
        runner_manifest_sha256: runner_manifest_sha256.clone(),
        workload_manifest_sha256: workload_manifest_sha256.clone(),
        protocol: latency_protocol(),
        calibration_observation_sha256: None,
        metrics: latency_metrics(),
    };
    apply_latency_calibration(&mut latency)?;
    let latency = canonical_json(&latency)?;
    files.insert(
        PathBuf::from("latency-manifest.sha256"),
        sidecar("latency-manifest.json", &latency),
    );
    files.insert(PathBuf::from("latency-manifest.json"), latency);

    let mut resources = ResourceManifest {
        version: PACK_VERSION,
        runner: "schemaform-perf-v1".to_owned(),
        browser: InteractionBrowser::Chromium,
        runner_manifest_sha256,
        workload_manifest_sha256,
        memory_protocol: memory_protocol(),
        calibration_observation_sha256: None,
        metrics: resource_metrics(),
        artifacts: resource_artifacts(),
    };
    apply_resource_calibration(&mut resources)?;
    let resources = canonical_json(&resources)?;
    files.insert(
        PathBuf::from("resource-manifest.sha256"),
        sidecar("resource-manifest.json", &resources),
    );
    files.insert(PathBuf::from("resource-manifest.json"), resources);
    files.insert(PathBuf::from("README.md"), README.as_bytes().to_vec());
    Ok(files)
}

fn interaction_manifest() -> InteractionManifest {
    let scenarios = vec![
        interaction_scenario(
            "controls",
            &[
                "generated_string_control_mounts_edits_and_submits_an_immutable_snapshot",
                "boolean_and_scalar_constant_controls_preserve_native_semantics",
                "finite_scalar_choices_use_opaque_tokens_and_submit_exact_values",
                "authored_ui_schema_renders_semantics_and_preserves_form_behavior",
            ],
            &["form-data", "submission-callback"],
        ),
        interaction_scenario(
            "keyboard-order",
            &[
                "responsive_grid_behavior_matches_the_active_css_viewport",
                "tabs_support_keyboard_navigation_adapter_local_selection_and_summary_focus",
            ],
            &["focus"],
        ),
        interaction_scenario(
            "presence-repair",
            &[
                "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
                "optional_fixed_object_materializes_repairs_removes_and_submits_in_the_browser",
            ],
            &["form-data", "submission-callback"],
        ),
        interaction_scenario(
            "array-focus",
            &[
                "scalar_array_structural_actions_preserve_dom_identity_focus_and_announcements",
                "fixed_object_array_rows_edit_focus_localize_and_submit_in_the_browser",
                "duplicate_fixed_object_array_lifecycle_updates_dom_keys_by_item_identity",
                "scalar_array_add_focuses_the_first_focusable_row_action",
            ],
            &[
                "announcements",
                "dom-identity",
                "focus",
                "form-data",
                "submission-callback",
            ],
        ),
        interaction_scenario(
            "tabs",
            &["tabs_support_keyboard_navigation_adapter_local_selection_and_summary_focus"],
            &["dom-identity", "focus", "form-data", "submission-callback"],
        ),
        interaction_scenario(
            "ime",
            &["ime_composition_stays_local_across_presentation_updates_and_commits_on_end"],
            &["dom-identity", "form-data"],
        ),
        interaction_scenario(
            "exact-numbers",
            &[
                "arbitrary_precision_integer_browser_trace_matches_the_core_facade",
                "arbitrary_precision_decimal_browser_trace_matches_the_core_facade",
            ],
            &["form-data", "submission-callback"],
        ),
        interaction_scenario(
            "grids",
            &["responsive_grid_behavior_matches_the_active_css_viewport"],
            &["dom-identity", "focus", "form-data", "submission-callback"],
        ),
        interaction_scenario(
            "findings",
            &[
                "external_visibility_parse_feedback_and_submission_focus_follow_core_policy",
                "custom_presenters_receive_one_deterministic_collection_per_target",
            ],
            &["focus", "form-data", "submission-callback"],
        ),
        interaction_scenario(
            "localization",
            &[
                "locale_and_presenter_changes_update_only_reactive_plain_text_presentation",
                "fixed_object_array_rows_edit_focus_localize_and_submit_in_the_browser",
            ],
            &["dom-identity", "form-data"],
        ),
        interaction_scenario(
            "reactivity",
            &[
                "ordinary_scalar_edit_updates_only_subscribed_displayed_state_generated",
                "ordinary_scalar_edit_updates_only_subscribed_displayed_state_authored",
            ],
            &["dom-identity", "mount-drop", "renderer-entry"],
        ),
        interaction_scenario(
            "submission",
            &[
                "generated_string_control_mounts_edits_and_submits_an_immutable_snapshot",
                "external_visibility_parse_feedback_and_submission_focus_follow_core_policy",
            ],
            &["focus", "form-data", "submission-callback"],
        ),
        interaction_scenario(
            "business-schema-corpus",
            &["every_in_profile_business_schema_executes_through_the_default_browser_adapter"],
            &["default-renderers", "form-data", "submission"],
        ),
    ];
    let cells = INTERACTION_BROWSERS
        .into_iter()
        .flat_map(|browser| {
            INTERACTION_VIEWPORT_WIDTHS
                .into_iter()
                .flat_map(move |width| {
                    INTERACTION_ZOOM_PERCENTS
                        .into_iter()
                        .map(move |zoom| InteractionCell {
                            id: format!("{}-{width}-{zoom}", browser.as_str()),
                            browser,
                            viewport_width_css_pixels: width,
                            zoom_percent: zoom,
                        })
                })
        })
        .collect();
    InteractionManifest {
        version: 1,
        suite: "schemaform-dioxus/browser_csr".to_owned(),
        browsers: INTERACTION_BROWSERS.to_vec(),
        viewport_widths_css_pixels: INTERACTION_VIEWPORT_WIDTHS.to_vec(),
        zoom_percents: INTERACTION_ZOOM_PERCENTS.to_vec(),
        zoom_protocol: "Set the Playwright CSS viewport to the requested width, apply CSS zoom to every mounted form root, and assert innerWidth equals the requested CSS width. Harness-owned nested viewport dimensions remain unscaled.".to_owned(),
        accessibility: AccessibilityGate {
            engine: "axe-core".to_owned(),
            version: "4.10.3".to_owned(),
            checkpoints: accessibility_checkpoints(),
            reviewed_exceptions: Vec::new(),
        },
        scenarios,
        cells,
    }
}

fn accessibility_checkpoints() -> Vec<AccessibilityCheckpoint> {
    vec![
        accessibility_checkpoint(
            "authored-ui",
            "authored_ui_schema_renders_semantics_and_preserves_form_behavior",
            &["ui-control", "ui-stack", "ui-group", "ui-text", "help"],
            &[
                "textbox \"Localized second field\"",
                "Localized family-name help.",
                "group \"Localized primary details\"",
            ],
        ),
        accessibility_checkpoint(
            "tabs-blocked",
            "tabs_support_keyboard_navigation_adapter_local_selection_and_summary_focus",
            &["ui-tabs", "tabs", "blocked-submission"],
            &[
                "tablist \"Tabs\"",
                "tab \"Contact\" [selected]",
                "tabpanel \"Contact\"",
                "button \"Value does not satisfy minLength.\"",
            ],
        ),
        accessibility_checkpoint(
            "grid",
            "responsive_grid_behavior_matches_the_active_css_viewport",
            &["ui-grid"],
            &["textbox \"First\"", "textbox \"Second\""],
        ),
        accessibility_checkpoint(
            "auto",
            "explicit_auto_renders_only_at_its_authored_position",
            &["ui-auto"],
            &[
                "paragraph: Generated fields",
                "textbox \"second\"",
                "paragraph: End fields",
            ],
        ),
        accessibility_checkpoint(
            "string",
            "generated_string_control_mounts_edits_and_submits_an_immutable_snapshot",
            &["string-control"],
            &["textbox \"Full name\": Ada", "textbox \"Full name\": Lin"],
        ),
        accessibility_checkpoint(
            "array-scalar",
            "scalar_array_structural_actions_preserve_dom_identity_focus_and_announcements",
            &["arrays"],
            &["group \"Tags\"", "button \"Add Tags item\"", "status"],
        ),
        accessibility_checkpoint(
            "array-object",
            "fixed_object_array_rows_edit_focus_localize_and_submit_in_the_browser",
            &["arrays"],
            &[
                "group \"People\"",
                "group \"Person\"",
                "group \"Address\"",
                "City fallback help.",
            ],
        ),
        accessibility_checkpoint(
            "read-write-only",
            "builtins_present_read_only_and_write_only_data_without_losing_submission_values",
            &["read-only", "write-only"],
            &[
                "group \"profile\"",
                "status \"Name\": Ada",
                "textbox \"Replace Secret\"",
                "status \"Secret region\": Value is set",
            ],
        ),
        accessibility_checkpoint(
            "presence-missing",
            "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
            &["presence-missing"],
            &["textbox \"Value\"", "button \"Set Value\""],
        ),
        accessibility_checkpoint(
            "presence-empty",
            "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
            &["presence-empty"],
            &["textbox \"Value\"", "button \"Remove Value\""],
        ),
        accessibility_checkpoint(
            "presence-null",
            "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
            &["presence-null"],
            &["textbox \"Value\"", "button \"Remove Value\""],
        ),
        accessibility_checkpoint(
            "presence-incompatible",
            "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
            &["presence-incompatible"],
            &[
                "textbox \"Value\": \"7\"",
                "status: \"7\"",
                "button \"Replace Value\"",
            ],
        ),
        accessibility_checkpoint(
            "presence-compatible",
            "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
            &["presence-compatible"],
            &["textbox \"Value\"", "button \"Clear Value\""],
        ),
        accessibility_checkpoint(
            "integer-parse-blocked",
            "arbitrary_precision_integer_browser_trace_matches_the_core_facade",
            &["integer-control", "parse-findings", "blocked-submission"],
            &[
                "button \"Enter a valid integer.\"",
                "textbox \"Quantity\": \"-\"",
            ],
        ),
        accessibility_checkpoint(
            "number-validation-blocked",
            "arbitrary_precision_decimal_browser_trace_matches_the_core_facade",
            &[
                "number-control",
                "validation-findings",
                "blocked-submission",
            ],
            &[
                "button \"Value must be at least",
                "textbox \"Rate\": \"0.10000000000000000000000000000000000000009\"",
            ],
        ),
        accessibility_checkpoint(
            "boolean-constant",
            "boolean_and_scalar_constant_controls_preserve_native_semantics",
            &["boolean-control", "constant-control"],
            &["checkbox \"Enabled\"", "status \"Region\": EU"],
        ),
        accessibility_checkpoint(
            "choice",
            "finite_scalar_choices_use_opaque_tokens_and_submit_exact_values",
            &["choice-control"],
            &[
                "combobox \"Choice\"",
                "option \"None\" [selected]",
                "status \"Region\": EU",
            ],
        ),
        accessibility_checkpoint(
            "unsupported",
            "unsupported_one_of_region_is_presented_and_blocks_browser_submission",
            &[
                "unsupported-regions",
                "capability-findings",
                "blocked-submission",
            ],
            &[
                "region \"Contact\"",
                "button \"This form region cannot be edited because oneOf branch selection is unsupported.\"",
            ],
        ),
        accessibility_checkpoint(
            "fixed-object",
            "optional_fixed_object_materializes_repairs_removes_and_submits_in_the_browser",
            &["fixed-object-control"],
            &[
                "group \"Settings\"",
                "button \"Add Settings\"",
                "textbox \"Name\"",
            ],
        ),
        accessibility_checkpoint(
            "external-parse-blocked",
            "external_visibility_parse_feedback_and_submission_focus_follow_core_policy",
            &["external-findings", "parse-findings", "blocked-submission"],
            &[
                "button \"server reported server-retry-required.\"",
                "button \"Enter a valid integer.\"",
                "textbox \"Quantity\": \"-\"",
            ],
        ),
        accessibility_checkpoint(
            "indeterminate-blocked",
            "indeterminate_validation_is_presented_and_blocks_browser_submission",
            &["indeterminate-findings", "blocked-submission"],
            &[
                "button \"Validation could not be completed reliably.\"",
                "textbox \"Name\"",
            ],
        ),
    ]
}

fn accessibility_checkpoint(
    id: &str,
    trace: &str,
    coverage: &[&str],
    aria_required: &[&str],
) -> AccessibilityCheckpoint {
    AccessibilityCheckpoint {
        id: id.to_owned(),
        trace: trace.to_owned(),
        coverage: coverage.iter().map(|value| (*value).to_owned()).collect(),
        aria_required: aria_required
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
    }
}

fn interaction_scenario(area: &str, traces: &[&str], assertions: &[&str]) -> InteractionScenario {
    InteractionScenario {
        area: area.to_owned(),
        traces: traces.iter().map(|value| (*value).to_owned()).collect(),
        assertions: assertions.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn interaction_artifacts(cell: &InteractionCell) -> Vec<InteractionArtifact> {
    let root = format!("testing/browser/artifacts/interactions/{}", cell.id);
    [
        (InteractionArtifactKind::Trace, "trace.zip"),
        (InteractionArtifactKind::Screenshot, "screenshot.png"),
        (
            InteractionArtifactKind::AccessibilityReport,
            "accessibility.json",
        ),
        (InteractionArtifactKind::BrowserLog, "browser.log"),
    ]
    .into_iter()
    .map(|(kind, name)| InteractionArtifact {
        kind,
        path: format!("{root}/{name}"),
        sha256: String::new(),
        bytes: 0,
    })
    .collect()
}

fn verify_interaction_artifact_contents(
    manifest: &InteractionManifest,
    observation: &InteractionObservation,
    mut read_artifact: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    for expected_cell in &manifest.cells {
        let observed_cell = observation
            .cells
            .iter()
            .find(|cell| {
                cell.browser == expected_cell.browser
                    && cell.viewport_width_css_pixels == expected_cell.viewport_width_css_pixels
                    && cell.zoom_percent == expected_cell.zoom_percent
            })
            .ok_or_else(|| format!("interaction observation is missing {}", expected_cell.id))?;
        for expected in interaction_artifacts(expected_cell) {
            let observed = observed_cell
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == expected.kind)
                .ok_or_else(|| {
                    format!(
                        "interaction cell {} is missing a raw artifact",
                        expected_cell.id
                    )
                })?;
            let bytes = read_artifact(&expected.path)?;
            if bytes.len() as u64 != observed.bytes || sha256(&bytes) != observed.sha256 {
                return Err(format!(
                    "interaction artifact {:?} for {} changed bytes",
                    expected.kind, expected_cell.id
                ));
            }
            if expected.kind == InteractionArtifactKind::AccessibilityReport {
                verify_raw_accessibility(observed_cell, &bytes)?;
            }
        }
    }
    Ok(())
}

fn verify_raw_accessibility(cell: &InteractionCellObservation, bytes: &[u8]) -> Result<(), String> {
    let reports: Vec<Value> = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse raw accessibility report: {error}"))?;
    if reports.len() != cell.accessibility.len() {
        return Err("raw accessibility report changed the checkpoint set".to_owned());
    }
    for checkpoint in &cell.accessibility {
        let raw = reports
            .iter()
            .find(|report| report["id"] == checkpoint.id)
            .ok_or_else(|| format!("raw accessibility report is missing {}", checkpoint.id))?;
        if raw["trace"] != checkpoint.trace || raw["aria_snapshot"] != checkpoint.aria_snapshot {
            return Err(format!(
                "raw accessibility report changed checkpoint {}",
                checkpoint.id
            ));
        }
        let violations = raw["report"]["violations"]
            .as_array()
            .ok_or_else(|| "raw axe report has no violations array".to_owned())?;
        if violations.len() != checkpoint.violations.len() {
            return Err(format!(
                "raw axe report changed violations for {}",
                checkpoint.id
            ));
        }
        for (raw, summarized) in violations.iter().zip(&checkpoint.violations) {
            let impact = serde_json::from_value::<AccessibilityImpact>(
                raw.get("impact")
                    .cloned()
                    .unwrap_or_else(|| Value::String("unknown".to_owned())),
            )
            .map_err(|error| format!("parse raw axe impact: {error}"))?;
            let nodes = raw["nodes"]
                .as_array()
                .ok_or_else(|| "raw axe violation has no nodes array".to_owned())?;
            let mut targets = nodes
                .iter()
                .flat_map(|node| {
                    node["target"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>();
            targets.sort();
            if raw["id"] != summarized.rule_id
                || impact != summarized.impact
                || nodes.len() != summarized.nodes
                || targets != summarized.targets
            {
                return Err(format!(
                    "raw axe report does not match the summary for {}",
                    checkpoint.id
                ));
            }
        }
    }
    Ok(())
}

fn interaction_traces(manifest: &InteractionManifest) -> BTreeSet<&str> {
    manifest
        .scenarios
        .iter()
        .flat_map(|scenario| scenario.traces.iter().map(String::as_str))
        .chain(
            manifest
                .accessibility
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.trace.as_str()),
        )
        .collect()
}

fn scenario(
    profile: &str,
    controls: usize,
    rows: usize,
    controls_per_row: usize,
    variant: &str,
    state: &str,
) -> Scenario {
    let array = rows > 0;
    let data_schema = if array {
        array_schema(controls_per_row)
    } else {
        object_schema(profile, controls)
    };
    let mut initial_form_data = if array {
        array_data(rows, controls_per_row)
    } else {
        object_data(profile, controls)
    };
    let target = if array {
        "/rows/0/field_000"
    } else if profile == "S1" {
        "/field_000"
    } else {
        "/level_1/level_2/level_3/field_000"
    };
    if state == "invalid" {
        *initial_form_data
            .pointer_mut(target)
            .expect("target exists") = if profile == "S1" {
            json!("invalid")
        } else {
            invalid_field_value(0)
        };
    } else if state == "64-finding" {
        apply_validation_finding_state(profile, &mut initial_form_data);
    }
    let setup = match state {
        "parse-blocked" => vec![SetupOperation::InputText {
            binding: target.to_owned(),
            value: "-".to_owned(),
        }],
        "64-finding" => vec![SetupOperation::ExternalFindings {
            count: 32,
            binding: target.to_owned(),
        }],
        _ => Vec::new(),
    };
    Scenario {
        version: PACK_VERSION,
        id: format!("{profile}-{variant}-{state}"),
        profile: profile.to_owned(),
        variant: variant.to_owned(),
        state: state.to_owned(),
        ui_schema: (variant == "authored")
            .then(|| authored_ui_schema(profile, controls, rows, controls_per_row)),
        data_schema,
        initial_form_data,
        setup,
        workloads: workloads(profile, variant, state, target),
    }
}

fn object_schema(profile: &str, controls: usize) -> Value {
    let properties = (0..controls)
        .map(|index| (field(index), field_schema(index, profile == "S1")))
        .collect::<serde_json::Map<_, _>>();
    let mut leaf = json!({
        "title": "Representative object controls",
        "type": "object",
        "required": properties.keys().cloned().collect::<Vec<_>>(),
        "properties": properties,
        "additionalProperties": false
    });
    if profile == "S1" {
        leaf["allOf"] = Value::Array(
            (0..32)
                .map(|_| json!({ "properties": { "field_000": { "minimum": 0 } } }))
                .collect(),
        );
    }
    if profile == "S1" {
        leaf["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
        leaf["$id"] = json!("urn:schemaform:browser-workload:object");
        return leaf;
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:browser-workload:object",
        "title": "Representative object workload",
        "type": "object",
        "required": ["level_1"],
        "properties": {
            "level_1": {
                "title": "Level 1",
                "type": "object",
                "required": ["level_2"],
                "properties": {
                    "level_2": {
                        "title": "Level 2",
                        "type": "object",
                        "required": ["level_3"],
                        "properties": { "level_3": leaf },
                        "additionalProperties": false
                    }
                },
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

fn array_schema(controls_per_row: usize) -> Value {
    let properties = (0..controls_per_row)
        .map(|index| (field(index), field_schema(index, false)))
        .collect::<serde_json::Map<_, _>>();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:browser-workload:array",
        "title": "Representative array workload",
        "type": "object",
        "required": ["rows"],
        "properties": {
            "rows": {
                "type": "array",
                "title": "Rows",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "required": properties.keys().cloned().collect::<Vec<_>>(),
                    "properties": properties,
                    "additionalProperties": false
                }
            }
        },
        "additionalProperties": false
    })
}

fn field_schema(index: usize, s1: bool) -> Value {
    let title = format!("Field {index:03}");
    match index % 5 {
        0 if s1 => json!({ "type": "integer", "title": title }),
        0 => json!({ "type": "integer", "minimum": 0, "title": title }),
        1 => json!({ "type": "string", "minLength": 1, "title": title }),
        2 => json!({ "type": "number", "minimum": 0, "title": title }),
        3 => json!({ "type": "boolean", "title": title }),
        _ => json!({ "enum": ["alpha", "beta", "gamma"], "title": title }),
    }
}

fn object_data(profile: &str, controls: usize) -> Value {
    let controls = control_data(controls);
    if profile == "S1" {
        controls
    } else {
        json!({ "level_1": { "level_2": { "level_3": controls } } })
    }
}

fn array_data(rows: usize, controls_per_row: usize) -> Value {
    json!({
        "rows": (0..rows)
            .map(|_| control_data(controls_per_row))
            .collect::<Vec<_>>()
    })
}

fn control_data(controls: usize) -> Value {
    Value::Object(
        (0..controls)
            .map(|index| (field(index), field_value(index)))
            .collect(),
    )
}

fn apply_validation_finding_state(profile: &str, form_data: &mut Value) {
    if profile == "S1" {
        form_data["field_000"] = json!(-1);
        return;
    }
    if profile == "A100x5" {
        for index in 0..32 {
            form_data["rows"][index / 5][field(index % 5)] = invalid_field_value(index % 5);
        }
        return;
    }
    for index in 0..32 {
        form_data["level_1"]["level_2"]["level_3"][field(index)] = invalid_field_value(index);
    }
}

fn invalid_field_value(index: usize) -> Value {
    match index % 5 {
        0 | 2 => json!(-1),
        1 => json!(""),
        3 => Value::Null,
        _ => json!("outside-the-finite-choice"),
    }
}

fn field_value(index: usize) -> Value {
    match index % 5 {
        0 => json!(index),
        1 => json!(format!("value-{index:03}")),
        2 => json!(
            format!("{}.25", index + 1)
                .parse::<f64>()
                .expect("finite number")
        ),
        3 => json!(index.is_multiple_of(2)),
        _ => json!("alpha"),
    }
}

fn authored_ui_schema(
    profile: &str,
    controls: usize,
    rows: usize,
    controls_per_row: usize,
) -> Value {
    if rows > 0 {
        return json!({
            "version": 1,
            "root": {
                "type": "stack",
                "value": {
                    "id": "workload-root",
                    "children": [{
                        "type": "control",
                        "value": {
                            "id": "rows-control",
                            "binding": { "origin": "root", "pointer": "/rows" },
                            "label": localized("workload.rows", "Rows"),
                            "item_label": { "key": "workload.row", "fallback": "Row" },
                            "item_template": {
                                "type": "grid",
                                "value": {
                                    "id": "row-grid",
                                    "cells": (0..controls_per_row).map(|index| json!({
                                        "compact_span": 12,
                                        "wide_span": 4,
                                        "child": control(index, "item_template", "")
                                    })).collect::<Vec<_>>()
                                }
                            }
                        }
                    }]
                }
            }
        });
    }
    json!({
        "version": 1,
        "root": {
            "type": "stack",
            "value": {
                "id": "workload-root",
                "children": (0..controls).map(|index| control(
                    index,
                    "root",
                    if profile == "S1" { "" } else { "/level_1/level_2/level_3" }
                )).collect::<Vec<_>>()
            }
        }
    })
}

fn control(index: usize, origin: &str, prefix: &str) -> Value {
    json!({
        "type": "control",
        "value": {
            "id": format!("field-{index:03}"),
            "binding": { "origin": origin, "pointer": format!("{prefix}/{}", field(index)) },
            "label": localized(&format!("workload.field.{index:03}"), &format!("Field {index:03}")),
            "help": localized(&format!("workload.help.{index:03}"), "Representative control")
        }
    })
}

fn localized(key: &str, fallback: &str) -> Value {
    json!({ "value": { "key": key, "fallback": fallback } })
}

fn field(index: usize) -> String {
    format!("field_{index:03}")
}

fn workloads(profile: &str, variant: &str, state: &str, target: &str) -> Vec<Workload> {
    let mut values = vec![
        Workload::Compilation {
            cold: true,
            fresh_browser_context: true,
        },
        Workload::Mount {
            cold: true,
            fresh_browser_context: true,
        },
        Workload::Edit {
            binding: target.to_owned(),
            alternating_values: ["1".to_owned(), "2".to_owned()],
        },
        Workload::Findings {
            binding: target.to_owned(),
            count: 64,
            alternating_actions: ["install".to_owned(), "clear".to_owned()],
        },
        Workload::Visibility {
            policies: ["immediate".to_owned(), "submission-only".to_owned()],
        },
        Workload::Localization {
            locales: ["en".to_owned(), "hu".to_owned()],
            message_source: if variant == "authored" {
                "ui-schema-key"
            } else {
                "generated-fallback"
            }
            .to_owned(),
        },
        Workload::Submission {
            expected: if state == "valid" { "ready" } else { "blocked" }.to_owned(),
        },
    ];
    if profile == "A100x5" {
        values.push(Workload::Arrays {
            binding: "/rows".to_owned(),
            operations: [
                "append",
                "remove-last",
                "insert-before-last",
                "remove-before-last",
                "move-last-up",
                "move-before-last-down",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        });
    }
    values
}

fn runner_manifest() -> Value {
    let mut pipeline = vec![
        json!({
            "program": "node",
            "arguments": ["testing/browser/scripts/observe-browser-runner.js"]
        }),
        json!({
            "program": "cargo",
            "arguments": ["run", "--locked", "-p", "browser-workload-pack", "--", "verify-runner", "testing/browser/artifacts/runner-observation.json"]
        }),
    ];
    pipeline.extend(artifact_pipeline());
    json!({
        "version": 1,
        "environment": "schemaform-perf-v1",
        "comparison_policy": "Abort before measurement when any pinned value differs.",
        "hardware": {
            "architecture": "x86_64",
            "cpu_vendor": "GenuineIntel",
            "cpu_model": "Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz",
            "physical_cores": 8,
            "logical_cpus": 16,
            "microcode": "0x2f0005c",
            "memory_bytes": 68719476736_u64
        },
        "operating_system": {
            "distribution": "Ubuntu 24.04.3 LTS",
            "kernel": "6.8.0-83-generic",
            "libc": "glibc 2.39"
        },
        "browsers": {
            "playwright": "1.55.0",
            "chromium": { "version": "140.0.7339.16", "revision": "1187" },
            "firefox": { "version": "141.0", "revision": "1490" },
            "webkit": { "version": "26.0", "revision": "2203" }
        },
        "rust_wasm_tools": rust_wasm_tools(),
        "compression": compression_contract(),
        "production_artifact": {
            "pipeline": pipeline,
            "wasm": "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm",
            "javascript": "testing/browser/artifacts/bindgen/browser_workload_runner.js",
            "brotli_wasm": "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm.br",
            "brotli_javascript": "testing/browser/artifacts/bindgen/browser_workload_runner.js.br"
        },
        "empty_shell_artifact": {
            "wasm": "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm",
            "javascript": "testing/browser/artifacts/empty-shell/browser_workload_empty_shell.js",
            "brotli_wasm": "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm.br"
        },
        "power": {
            "ac_power_required": true,
            "governor": "performance",
            "intel_pstate_min_perf_pct": 100,
            "intel_pstate_max_perf_pct": 100,
            "turbo": "disabled"
        },
        "affinity": {
            "measurement_cpu_list": "2-7",
            "browser_process_cpu_list": "2-7",
            "runner_process_cpu_list": "2-7"
        }
    })
}

fn rust_wasm_tools() -> Value {
    json!({
        "rust": "1.90.0 (1159e78c4 2025-09-14)",
        "cargo": "1.90.0 (840b83a10 2025-07-30)",
        "wasm_target": "wasm32-unknown-unknown",
        "wasm_bindgen_cli": "0.2.126",
        "binaryen_wasm_opt": "123",
        "wasm_tools": "1.239.0"
    })
}

fn compression_contract() -> Value {
    json!({
        "brotli": "1.1.0",
        "quality": 11,
        "window": 22
    })
}

fn artifact_pipeline() -> Vec<Value> {
    vec![
        json!({
            "program": "cargo",
            "arguments": ["build", "--locked", "--release", "--target", "wasm32-unknown-unknown", "-p", "browser-workload-runner"]
        }),
        json!({
            "program": "wasm-bindgen",
            "arguments": ["--target", "web", "--no-typescript", "--out-dir", "testing/browser/artifacts/bindgen", "target/wasm32-unknown-unknown/release/browser_workload_runner.wasm"]
        }),
        json!({
            "program": "wasm-opt",
            "arguments": ["-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "-o", "testing/browser/artifacts/browser_workload_runner_bg.optimized.wasm", "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm"]
        }),
        json!({
            "program": "mv",
            "arguments": ["testing/browser/artifacts/browser_workload_runner_bg.optimized.wasm", "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm"]
        }),
        json!({
            "program": "brotli",
            "arguments": ["--quality", "11", "--lgwin", "22", "--output", "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm.br", "testing/browser/artifacts/bindgen/browser_workload_runner_bg.wasm"]
        }),
        json!({
            "program": "brotli",
            "arguments": ["--quality", "11", "--lgwin", "22", "--output", "testing/browser/artifacts/bindgen/browser_workload_runner.js.br", "testing/browser/artifacts/bindgen/browser_workload_runner.js"]
        }),
        json!({
            "program": "cargo",
            "arguments": ["build", "--locked", "--release", "--target", "wasm32-unknown-unknown", "-p", "browser-workload-empty-shell"]
        }),
        json!({
            "program": "wasm-bindgen",
            "arguments": ["--target", "web", "--no-typescript", "--out-dir", "testing/browser/artifacts/empty-shell", "target/wasm32-unknown-unknown/release/browser_workload_empty_shell.wasm"]
        }),
        json!({
            "program": "wasm-opt",
            "arguments": ["-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "-o", "testing/browser/artifacts/browser_workload_empty_shell_bg.optimized.wasm", "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm"]
        }),
        json!({
            "program": "mv",
            "arguments": ["testing/browser/artifacts/browser_workload_empty_shell_bg.optimized.wasm", "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm"]
        }),
        json!({
            "program": "brotli",
            "arguments": ["--quality", "11", "--lgwin", "22", "--output", "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm.br", "testing/browser/artifacts/empty-shell/browser_workload_empty_shell_bg.wasm"]
        }),
    ]
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn sidecar(name: &str, contents: &[u8]) -> Vec<u8> {
    format!("{}  {name}\n", sha256(contents)).into_bytes()
}

fn sha256(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn files_below(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    fn visit(root: &Path, current: &Path, paths: &mut BTreeSet<PathBuf>) -> Result<(), String> {
        for entry in fs::read_dir(current)
            .map_err(|error| format!("read directory {}: {error}", current.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, paths)?;
            } else {
                paths.insert(
                    path.strip_prefix(root)
                        .expect("path is below root")
                        .to_owned(),
                );
            }
        }
        Ok(())
    }
    let mut paths = BTreeSet::new();
    visit(root, root, &mut paths)?;
    Ok(paths)
}

const README: &str = r#"# Browser workload pack

This directory is generated by `cargo run -p browser-workload-pack -- generate`.
Do not edit generated files. `manifest.json` names every scenario and addresses its
immutable payload by SHA-256. The `.sha256` sidecars address the workload, runner,
interaction, artifact, latency, and resource manifests themselves. Latency and
runtime-memory contracts are retained for post-first-release qualification.

The exact scenario matrix is `S1`, `O50`, `O500`, and `A100x5`, each in generated
and authored variants and valid, invalid, parse-blocked, and 64-finding states.
`S1`, `O50`, and `O500` contain 1, 50, and 500 controls. `A100x5` contains 100
fixed-object rows with five controls per row. Setup operations are replayed only
after form construction, so parse blockers and external findings are represented
without corrupting canonical form data.

The runner manifest is a comparison lock, not a description of arbitrary developer
machines. Browser numeric evidence must abort before sampling unless every pinned
hardware, operating-system, browser, tool, power, and affinity value matches. The
release runner records those observed sections with
`node testing/browser/scripts/observe-browser-runner.js` and must pass the result to
`cargo run -p browser-workload-pack -- verify-runner OBSERVATION.json`; any missing
or unequal section exits unsuccessfully before comparison. `production_artifact.pipeline`
is the complete ordered command contract for the runner WASM, JavaScript glue,
optimized WASM, and Brotli WASM artifacts.

The production runner deserializes a content-addressed scenario supplied by the
driver, so fixtures are not embedded in the measured WASM. It applies setup before
binding and exports separate `compile_workload`, `mount_workload`, and
`run_workload(name, phase)` boundaries. Mount and hot operations expose an
instrumented DOM commit sentinel; their returned `performance.now()` handler-entry
mark is compared with the observation time of that sentinel.

`latency-manifest.json` declares every scenario/workload metric, five fresh Chromium
processes with 50 warmups and 200 measured alternating operations each, 100 fresh
contexts for each cold metric, nearest-rank p50/p95/p99, the fixed O500 edit gate,
and the absolute calibration caps. `testing/browser/scripts/run-browser-latency.js` refuses to sample
until the runner and interaction observations pass, archives every raw sample, and
does not contain retry or outlier-removal paths. Verify a frozen contract with
`cargo run -p browser-workload-pack -- verify-latency OBSERVATION.json`. The first
fully conforming tracer can fill the generator's one-time source with
`cargo run -p browser-workload-pack -- calibrate-latency OBSERVATION.json
testing/browser/pack/latency-calibration.json`; regenerate the pack to freeze
the ceilings and observation digest. The generated manifest prevents a second
calibration.

`resource-manifest.json` fixes the `A100x5-authored-64-finding` settled mixed
workload, records WASM linear memory after every one of 1,000 committed operations,
and measures Chromium's post-GC retained JS heap before and after that workload. It
also names every optimized product and empty-shell artifact used to calculate total
and incremental Brotli WASM plus total Brotli runtime JavaScript. The verifier
recomputes all five summaries, hashes the actual artifact bytes, enforces the
128 MiB WASM high-water, 64 MiB retained-heap delta, 1536/512 KiB WASM, and 64 KiB JavaScript safety caps, and rejects
retries or waivers. The first fully conforming latency and resource tracer can fill
the one-time 1.20x MiB/KiB-rounded calibration with `calibrate-resources`.

The latency and runtime-memory contracts above are not first-release gates and do
not create quantitative claims until their post-release calibration is completed.

`artifact-manifest.json` carries the hardware-independent production and
empty-shell pipeline plus fixed caps of 1536 KiB total Brotli WASM, 512 KiB
incremental Brotli WASM, and 64 KiB total Brotli runtime JavaScript. Run
`cargo run -p browser-workload-pack -- verify-artifacts` after building the listed
files; verification reads their bytes directly and requires no browser observation.

`archive-evidence` stores the first-release contracts, browser traces and
accessibility reports, optimized artifacts, and exact source under `objects/sha256`.
Its root manifest records the source commit and tree, backed by an independently
verifiable Git bundle, and content-addresses source-tree,
interactions-accessibility, and artifact-size conclusions. `verify-evidence`
reloads only archived bytes and rejects an incomplete inventory, failed
conclusion, retry, waiver, changed hash, or unreferenced object. Deferred latency,
runtime-memory, calibration, and hardware-runner observations are excluded.

`interaction-manifest.json` is the release interaction gate. It maps each required
contract area to the real-DOM traces that assert it and declares the exact pinned
browser, CSS-viewport, and zoom Cartesian product. Run the interactive
`wasm-bindgen-test` server, execute `testing/browser/scripts/run-browser-interaction-matrix.js` with
Playwright from the runner manifest, then verify its machine-readable completion
record with `cargo run -p browser-workload-pack -- verify-interactions
testing/browser/artifacts/interaction-observation.json`. CSS zoom is applied before the test
form mounts while the CSS viewport remains at its requested width; every cell asserts
that `innerWidth` equals its requested CSS viewport width. Harness-owned nested
viewport dimensions remain unscaled so their CSS-pixel assertions stay independent.
The same manifest pins `axe-core`, maps every accessibility checkpoint to its browser
trace and covered contract areas, and records reviewed upstream exceptions. An
exception is accepted only with a defect URL, separate compensating product test,
exact violation fingerprint, and future expiry. Each cell retains the engine-computed
ARIA snapshot and verifies its required semantics for every checkpoint.
"#;
