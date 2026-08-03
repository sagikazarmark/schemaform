use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("browser workload pack crate has a parent directory")
        .join("workload-pack");
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("generate") => browser_workload_pack::generate(&root),
        Some("check") => browser_workload_pack::check(&root),
        Some("verify-runner") => arguments
            .next()
            .ok_or_else(|| "verify-runner requires an observation JSON path".to_owned())
            .and_then(|path| {
                browser_workload_pack::verify_runner_file(&root, &PathBuf::from(path))
            }),
        Some("verify-interactions") => arguments
            .next()
            .ok_or_else(|| "verify-interactions requires an observation JSON path".to_owned())
            .and_then(|path| {
                browser_workload_pack::verify_interaction_file(&root, &PathBuf::from(path))
            }),
        Some("verify-artifacts") => browser_workload_pack::verify_artifact_files(&root),
        Some("verify-latency") => arguments
            .next()
            .ok_or_else(|| "verify-latency requires an observation JSON path".to_owned())
            .and_then(|path| {
                browser_workload_pack::verify_latency_file(&root, &PathBuf::from(path))
            }),
        Some("calibrate-latency") => arguments
            .next()
            .zip(arguments.next())
            .ok_or_else(|| {
                "calibrate-latency requires observation and output JSON paths".to_owned()
            })
            .and_then(|(observation, output)| {
                browser_workload_pack::calibrate_latency_file(
                    &root,
                    &PathBuf::from(observation),
                    &PathBuf::from(output),
                )
            }),
        Some("verify-resources") => arguments
            .next()
            .ok_or_else(|| "verify-resources requires an observation JSON path".to_owned())
            .and_then(|path| {
                browser_workload_pack::verify_resource_file(&root, &PathBuf::from(path))
            }),
        Some("calibrate-resources") => arguments
            .next()
            .zip(arguments.next())
            .zip(arguments.next())
            .ok_or_else(|| {
                "calibrate-resources requires resource observation, latency observation, and output JSON paths".to_owned()
            })
            .and_then(|((resources, latency), output)| {
                browser_workload_pack::calibrate_resource_file(
                    &root,
                    &PathBuf::from(resources),
                    &PathBuf::from(latency),
                    &PathBuf::from(output),
                )
            }),
        Some("archive-evidence") => arguments
            .next()
            .ok_or_else(|| "archive-evidence requires an output directory".to_owned())
            .and_then(|path| {
                browser_workload_pack::archive_browser_evidence(&root, &PathBuf::from(path))
            }),
        Some("verify-evidence") => arguments
            .next()
            .ok_or_else(|| "verify-evidence requires an archive directory".to_owned())
            .and_then(|path| {
                browser_workload_pack::verify_evidence_archive(&PathBuf::from(path))
            }),
        _ => {
            eprintln!(
                "usage: cargo run -p browser-workload-pack -- <generate|check|verify-runner OBSERVATION.json|verify-interactions OBSERVATION.json|verify-artifacts|verify-latency OBSERVATION.json|calibrate-latency OBSERVATION.json OUTPUT.json|verify-resources OBSERVATION.json|calibrate-resources RESOURCE.json LATENCY.json OUTPUT.json|archive-evidence OUTPUT_DIR|verify-evidence ARCHIVE_DIR>"
            );
            return ExitCode::FAILURE;
        }
    }
    .map_or_else(
        |error| {
            eprintln!("{error}");
            ExitCode::FAILURE
        },
        |()| ExitCode::SUCCESS,
    )
}
