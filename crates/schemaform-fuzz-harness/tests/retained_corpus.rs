use schemaform_fuzz_harness::{Target, retained_cases, run_deterministically};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn target_specific_seed_classes_replay_deterministically() {
    let cases = retained_cases();
    for target in Target::ALL {
        let target_cases = cases
            .iter()
            .filter(|case| case.target == target)
            .collect::<Vec<_>>();
        assert_eq!(
            target_cases.len(),
            3,
            "each target retains all three seed classes"
        );
        let primary_class = match target {
            Target::UserCommands | Target::HostTransactions | Target::ExternalFindings => "model",
            _ => "official",
        };
        assert!(target_cases.iter().any(|case| case.source == primary_class));
        assert!(target_cases.iter().any(|case| case.source == "corpus"));
        assert!(target_cases.iter().any(|case| case.source == "regression"));
        for case in target_cases {
            let outcome = run_deterministically(case.target, case.input);
            assert_eq!(
                outcome.kind(),
                case.expected_outcome,
                "{} changed its contracted outcome",
                case.name
            );
            assert_eq!(
                outcome.normalized_digest(),
                case.expected_digest,
                "{} changed its complete normalized outcome",
                case.name
            );
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn oversized_inputs_stop_at_the_decoder_boundary() {
    let oversized = vec![0; schemaform_fuzz_harness::MAX_INPUT_BYTES + 1];
    for target in Target::ALL {
        assert_eq!(
            schemaform_fuzz_harness::run(target, &oversized),
            schemaform_fuzz_harness::Outcome::InputTooLarge
        );
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn runtime_decoding_stops_after_the_contracted_command_count() {
    let prefix = vec![0; schemaform_fuzz_harness::MAX_RUNTIME_COMMANDS * 12];
    let mut extended = prefix.clone();
    extended.extend_from_slice(&[u8::MAX; 12]);
    for target in [
        Target::UserCommands,
        Target::HostTransactions,
        Target::ExternalFindings,
    ] {
        assert_eq!(
            schemaform_fuzz_harness::run(target, &prefix),
            schemaform_fuzz_harness::run(target, &extended),
            "{target:?} decoded more than the contracted command count"
        );
    }
}
