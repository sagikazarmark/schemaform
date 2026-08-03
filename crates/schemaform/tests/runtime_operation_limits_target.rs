use std::num::NonZeroUsize;

use schemaform::{
    ExternalFinding, ExternalFindingBatch, FindingView, Form, FormDataLimits, FormDefinition,
    InstanceIdentity, JsonPointer, ResourceLimitError, ResourceLimitPhase,
    form::{
        ExternalFindingError, ExternalFindingLimits, FindingVisibility, FindingVisibilityPolicy,
        HostCommitError, ParseBlockerKind, ReinitializeError, UserOperationError,
        ValidationOutcomeView,
    },
};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn edit_buffer_and_resulting_form_data_limits_have_atomic_boundaries() {
    let definition = object_definition(json!({ "n": { "type": "string" } }));

    for maximum in [2, 3, 4] {
        let mut form = definition
            .form(json!({ "n": "A" }))
            .limits(FormDataLimits::default().max_edit_buffer_bytes(maximum))
            .build()
            .unwrap();
        let name = node_with_binding(&form, "/n");
        let before = snapshot(&form);
        let result = form.user().input_text(name, "Ada");
        assert_eq!(result.is_ok(), maximum >= 3);
        if maximum < 3 {
            assert_resource_limit(result.unwrap_err(), "edit_buffer_bytes", maximum, 3, "/n");
            assert_eq!(snapshot(&form), before);
        }
    }

    let mut form = definition
        .form(json!({ "n": "A" }))
        .limits(
            FormDataLimits::default()
                .max_edit_buffer_bytes(8)
                .max_scalar_bytes(3),
        )
        .build()
        .unwrap();
    let name = node_with_binding(&form, "/n");
    form.user().input_text(name, "Ada").unwrap();
    let before = snapshot(&form);
    let error = form.user().input_text(name, "Grace").unwrap_err();
    assert_resource_limit(error, "scalar_bytes", 3, 5, "/n");
    assert_eq!(snapshot(&form), before);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn user_values_are_bounded_before_candidate_state_is_published() {
    let definition = object_definition(json!({
        "n": { "type": "string", "enum": ["Ada", "Grace"] }
    }));
    let mut form = definition
        .form(json!({ "n": "Ada" }))
        .limits(FormDataLimits::default().max_scalar_bytes(3))
        .build()
        .unwrap();
    let name = node_with_binding(&form, "/n");
    let before = snapshot(&form);

    let error = form.user().set_value(name, json!("Grace")).unwrap_err();
    assert_resource_limit(error, "scalar_bytes", 3, 5, "/n");
    assert_eq!(snapshot(&form), before);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn active_and_total_edit_buffer_limits_reject_without_mutation() {
    let definition = object_definition(json!({
        "a": { "type": "string" },
        "b": { "type": "string" },
        "c": { "type": "string" }
    }));
    let initial = json!({ "a": "", "b": "", "c": "" });

    let mut form = definition
        .form(initial.clone())
        .limits(FormDataLimits::default().max_active_edit_buffers(2))
        .build()
        .unwrap();
    for binding in ["/a", "/b"] {
        let target = node_with_binding(&form, binding);
        form.user().input_text(target, "x").unwrap();
    }
    let before = snapshot(&form);
    let target = node_with_binding(&form, "/c");
    let error = form.user().input_text(target, "x").unwrap_err();
    assert_resource_limit(error, "active_edit_buffers", 2, 3, "/c");
    assert_eq!(snapshot(&form), before);

    let mut form = definition
        .form(initial)
        .limits(FormDataLimits::default().max_total_edit_buffer_bytes(3))
        .build()
        .unwrap();
    for (binding, text) in [("/a", "x"), ("/b", "yz")] {
        let target = node_with_binding(&form, binding);
        form.user().input_text(target, text).unwrap();
    }
    let before = snapshot(&form);
    let target = node_with_binding(&form, "/c");
    let error = form.user().input_text(target, "x").unwrap_err();
    assert_resource_limit(error, "total_edit_buffer_bytes", 3, 4, "/c");
    assert_eq!(snapshot(&form), before);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn repeated_identity_and_form_tree_growth_reject_without_mutation() {
    let definition = object_definition(json!({
        "items": { "type": "array", "items": { "type": "string", "default": "" } }
    }));
    let mut form = definition
        .form(json!({ "items": ["one"] }))
        .limits(
            FormDataLimits::default()
                .max_repeated_items(2)
                .max_form_tree_nodes(5),
        )
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/items");
    form.user().append_item(array).unwrap();
    let before = snapshot(&form);
    let identities_before = item_identities(&form, array);
    let error = form.user().append_item(array).unwrap_err();
    assert_resource_limit(error, "repeated_items", 2, 3, "");
    assert_eq!(snapshot(&form), before);
    assert_eq!(item_identities(&form, array), identities_before);
    let first = identities_before[0];
    let error = form.user().insert_item_before(array, first).unwrap_err();
    assert_resource_limit(error, "repeated_items", 2, 3, "");
    assert_eq!(snapshot(&form), before);
    assert_eq!(item_identities(&form, array), identities_before);

    let mut form = definition
        .form(json!({ "items": ["one"] }))
        .limits(
            FormDataLimits::default()
                .max_repeated_items(3)
                .max_form_tree_nodes(4),
        )
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/items");
    form.user().append_item(array).unwrap();
    let before = snapshot(&form);
    let error = form.user().append_item(array).unwrap_err();
    assert_resource_limit(error, "form_tree_nodes", 4, 5, "");
    assert_eq!(snapshot(&form), before);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn host_transaction_limits_have_below_exact_and_atomic_above_traces() {
    let definition = object_definition(json!({ "n": { "type": "string" } }));
    for operations in [1, 2, 3] {
        let mut form = definition
            .form(json!({ "n": "A" }))
            .limits(FormDataLimits::default().max_host_operations_per_transaction(2))
            .build()
            .unwrap();
        let before = snapshot(&form);
        let result = form.transact(|draft| {
            for index in 0..operations {
                draft.set(&pointer("/n"), json!(format!("name-{index}")));
            }
        });
        assert_eq!(result.is_ok(), operations <= 2);
        if operations == 3 {
            assert_commit_resource_limit(
                result.unwrap_err(),
                "host_operations_per_transaction",
                2,
                3,
                "",
            );
            assert_eq!(snapshot(&form), before);
        }
    }

    let mut form = definition
        .form(json!({ "n": "A" }))
        .limits(FormDataLimits::default().max_scalar_bytes(3))
        .build()
        .unwrap();
    let before = snapshot(&form);
    let error = form
        .transact(|draft| draft.set(&pointer("/n"), json!("Grace")))
        .unwrap_err();
    assert_commit_resource_limit(error, "scalar_bytes", 3, 5, "/n");
    assert_eq!(snapshot(&form), before);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn host_transaction_resulting_json_limits_have_boundaries() {
    assert_transaction_value_limit(
        "depth",
        2,
        json!({ "v": [] }),
        "/v",
        json!([null]),
        FormDataLimits::max_depth,
        "/v/0",
    );
    assert_transaction_value_limit(
        "nodes",
        3,
        json!({ "v": [] }),
        "/v",
        json!([null]),
        FormDataLimits::max_nodes,
        "/v/0",
    );
    assert_transaction_value_limit(
        "members",
        2,
        json!({ "v": null }),
        "/x",
        Value::Null,
        FormDataLimits::max_members,
        "",
    );
    assert_transaction_value_limit(
        "collection_length",
        2,
        json!({ "v": [null] }),
        "/v",
        json!([null, null]),
        FormDataLimits::max_collection_length,
        "/v",
    );
    assert_transaction_value_limit(
        "scalar_bytes",
        3,
        json!({ "v": "a" }),
        "/v",
        json!("abc"),
        FormDataLimits::max_scalar_bytes,
        "/v",
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn privileged_array_growth_limits_reject_without_mutation() {
    let definition = object_definition(json!({
        "items": { "type": "array", "items": { "type": "string" } }
    }));
    for replace in [false, true] {
        let mut form = definition
            .form(json!({ "items": ["one"] }))
            .limits(FormDataLimits::default().max_repeated_items(1))
            .build()
            .unwrap();
        let array = node_with_binding(&form, "/items");
        let before = snapshot(&form);
        let identities_before = item_identities(&form, array);
        let error = form
            .transact(|draft| {
                if replace {
                    draft.set(&pointer("/items"), json!(["one", "two"]));
                } else {
                    draft.append_item(&pointer("/items"), json!("two"));
                }
            })
            .unwrap_err();
        assert_commit_resource_limit(error, "repeated_items", 1, 2, "");
        assert_eq!(snapshot(&form), before);
        assert_eq!(item_identities(&form, array), identities_before);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn reinitialize_limits_reject_without_replacing_baseline_or_state() {
    let definition = object_definition(json!({ "n": { "type": "string" } }));
    let mut form = definition
        .form(json!({ "n": "Ada" }))
        .limits(FormDataLimits::default().max_scalar_bytes(4))
        .build()
        .unwrap();
    form.reinitialize(json!({ "n": "Nora" })).unwrap();
    let before = snapshot(&form);
    let error = form.reinitialize(json!({ "n": "Grace" })).unwrap_err();
    let ReinitializeError::ResourceLimit(error) = error else {
        panic!("expected a resource limit");
    };
    assert_eq!(error.path().as_str(), "/n");
    assert_limit(error, "scalar_bytes", 4, 5);
    assert_eq!(snapshot(&form), before);
    form.reset();
    assert_eq!(form.form_data(), &json!({ "n": "Nora" }));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn validation_findings_retain_the_deterministic_prefix_with_boundaries() {
    let definition = required_object_definition(
        json!({
            "a": { "type": "string" },
            "b": { "type": "string" },
            "c": { "type": "string" }
        }),
        &["a", "b", "c"],
    );

    for maximum in [2, 3, 4] {
        let form = definition
            .form(json!({}))
            .limits(FormDataLimits::default().max_retained_validation_findings(maximum))
            .build()
            .unwrap();
        let view = form.view();
        let ValidationOutcomeView::Invalid {
            findings,
            truncated,
        } = view.validation_outcome()
        else {
            panic!("missing required properties must be invalid");
        };
        assert_eq!(findings.len(), maximum.min(3));
        assert_eq!(truncated, maximum < 3);
        let properties = findings
            .iter()
            .map(|finding| finding.parameters()["property"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(properties, ["a", "b", "c"][..maximum.min(3)]);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn validation_parameter_bytes_have_below_exact_and_above_boundaries() {
    let definition = object_definition(json!({
        "v": { "type": "string", "pattern": "a" }
    }));
    for maximum in [2, 3, 4] {
        let form = definition
            .form(json!({ "v": "b" }))
            .limits(FormDataLimits::default().max_validation_parameter_bytes(maximum))
            .build()
            .unwrap();
        let view = form.view();
        let ValidationOutcomeView::Invalid { findings, .. } = view.validation_outcome() else {
            panic!("the pattern mismatch must be invalid");
        };
        assert_eq!(findings.len(), 1);
        let expected = if maximum < 3 {
            json!({ "omitted": true })
        } else {
            json!({ "pattern": "a" })
        };
        assert_eq!(findings[0].parameters(), &expected);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn canonical_integer_digit_limit_has_below_exact_and_above_traces() {
    let definition = object_definition(json!({ "v": { "type": "integer" } }));
    let mut form = definition
        .form(json!({ "v": 1 }))
        .limits(FormDataLimits::default().max_canonical_integer_digits(3))
        .build()
        .unwrap();
    let target = node_with_binding(&form, "/v");

    form.user().input_text(target, "12").unwrap();
    assert_eq!(form.form_data(), &json!({ "v": 12 }));
    form.user().input_text(target, "123").unwrap();
    assert_eq!(form.form_data(), &json!({ "v": 123 }));
    let before_data = form.form_data().clone();
    let before_data_revision = form.view().data_revision();
    form.user().input_text(target, "1234").unwrap();
    assert_eq!(form.form_data(), &before_data);
    assert_eq!(form.view().data_revision(), before_data_revision);
    let node = form.node(target).unwrap();
    assert_eq!(node.edit_buffer(), Some("1234"));
    assert_eq!(
        node.parse_blocker(),
        Some(ParseBlockerKind::ResourceLimitExceeded)
    );

    let mut zero_limited = definition
        .form(json!({ "v": 1 }))
        .limits(FormDataLimits::default().max_canonical_integer_digits(0))
        .build()
        .unwrap();
    let target = node_with_binding(&zero_limited, "/v");
    zero_limited.user().input_text(target, "0").unwrap();
    assert_eq!(zero_limited.form_data(), &json!({ "v": 1 }));
    assert_eq!(
        zero_limited.node(target).unwrap().parse_blocker(),
        Some(ParseBlockerKind::ResourceLimitExceeded)
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn external_finding_count_limit_has_below_exact_and_atomic_above_traces() {
    let definition = object_definition(json!({ "name": { "type": "string" } }));
    for count in [1, 2, 3] {
        let mut form = definition
            .form(json!({ "name": "Ada" }))
            .external_finding_limits(ExternalFindingLimits::new(
                NonZeroUsize::new(2).unwrap(),
                NonZeroUsize::new(1024).unwrap(),
            ))
            .build()
            .unwrap();
        let before = snapshot(&form);
        let findings = (0..count)
            .map(|index| {
                ExternalFinding::blocking(format!("finding-{index}"), pointer("/name"), json!({}))
            })
            .collect::<Vec<_>>();
        let result = form.apply_external_findings(ExternalFindingBatch::new(
            "server",
            form.view().data_revision(),
            findings,
        ));
        assert_eq!(result.is_ok(), count <= 2);
        if count == 3 {
            let ExternalFindingError::ResourceLimit(error) = result.unwrap_err() else {
                panic!("expected a resource limit");
            };
            assert_eq!(error.path().as_str(), "");
            assert_limit(error, "active_external_findings", 2, 3);
            assert_eq!(snapshot(&form), before);
        }
    }

    let mut duplicate_limited = definition
        .form(json!({ "name": "Ada" }))
        .finding_visibility(immediate_findings())
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(2).unwrap(),
            NonZeroUsize::new(1024).unwrap(),
        ))
        .build()
        .unwrap();
    let duplicate = ExternalFinding::blocking("duplicate", pointer("/name"), json!({}));
    duplicate_limited
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            duplicate_limited.view().data_revision(),
            vec![duplicate; 3],
        ))
        .unwrap();
    assert_eq!(
        duplicate_limited
            .node(node_with_binding(&duplicate_limited, "/name"))
            .unwrap()
            .external_findings()
            .count(),
        1
    );

    let mut form = definition
        .form(json!({ "name": "Ada" }))
        .finding_visibility(immediate_findings())
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(2).unwrap(),
            NonZeroUsize::new(1024).unwrap(),
        ))
        .build()
        .unwrap();
    let revision = form.view().data_revision();
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        revision,
        [ExternalFinding::blocking(
            "old",
            pointer("/name"),
            json!({}),
        )],
    ))
    .unwrap();
    let before = snapshot(&form);
    let error = form
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            revision,
            (0..3)
                .map(|index| {
                    ExternalFinding::blocking(format!("new-{index}"), pointer("/name"), json!({}))
                })
                .collect::<Vec<_>>(),
        ))
        .unwrap_err();
    let ExternalFindingError::ResourceLimit(error) = error else {
        panic!("expected a resource limit");
    };
    assert_eq!(error.path().as_str(), "");
    assert_limit(error, "active_external_findings", 2, 3);
    assert_eq!(snapshot(&form), before);
}

#[test]
fn external_finding_incoming_count_limit_is_independent_and_atomic() {
    let definition = object_definition(json!({ "name": { "type": "string" } }));
    let release_defaults = ExternalFindingLimits::default();
    assert_eq!(
        release_defaults.incoming_findings(),
        release_defaults.max_active_findings() * 4
    );
    assert_eq!(
        release_defaults.incoming_bytes(),
        release_defaults.max_active_bytes() * 4
    );
    let defaults =
        ExternalFindingLimits::new(NonZeroUsize::new(2).unwrap(), NonZeroUsize::new(7).unwrap());
    assert_eq!(defaults.incoming_findings(), 8);
    assert_eq!(defaults.incoming_bytes(), 28);

    for count in [1, 2, 3] {
        let limits = ExternalFindingLimits::new(
            NonZeroUsize::new(10).unwrap(),
            NonZeroUsize::new(1024).unwrap(),
        )
        .max_incoming_findings(2)
        .max_incoming_bytes(1024);
        assert_eq!(limits.incoming_findings(), 2);
        assert_eq!(limits.incoming_bytes(), 1024);
        let mut form = definition
            .form(json!({ "name": "Ada" }))
            .external_finding_limits(limits)
            .build()
            .unwrap();
        let before = snapshot(&form);
        let findings = (0..count)
            .map(|index| {
                ExternalFinding::blocking(format!("finding-{index}"), pointer("/name"), json!({}))
            })
            .collect::<Vec<_>>();

        let result = form.apply_external_findings(ExternalFindingBatch::new(
            "server",
            form.view().data_revision(),
            findings,
        ));

        assert_eq!(result.is_ok(), count <= 2);
        if count == 3 {
            let ExternalFindingError::ResourceLimit(error) = result.unwrap_err() else {
                panic!("expected a resource limit");
            };
            assert_limit(error, "incoming_external_findings", 2, 3);
            assert_eq!(snapshot(&form), before);
        }
    }

    let mut count_first = definition
        .form(json!({ "name": "Ada" }))
        .external_finding_limits(
            ExternalFindingLimits::new(
                NonZeroUsize::new(10).unwrap(),
                NonZeroUsize::new(1024).unwrap(),
            )
            .max_incoming_findings(1)
            .max_incoming_bytes(0)
            .max_parameter_depth(0),
        )
        .build()
        .unwrap();
    let error = count_first
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            count_first.view().data_revision(),
            [
                ExternalFinding::blocking("a", pointer("/name"), json!({ "deep": {} })),
                ExternalFinding::blocking("b", pointer("/name"), json!({})),
            ],
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        ExternalFindingError::ResourceLimit(limit)
            if limit.dimension() == "incoming_external_findings"
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn external_finding_byte_limit_has_below_exact_and_atomic_above_traces() {
    let definition = object_definition(json!({ "n": { "type": "string" } }));
    // One source byte, one code byte, two pointer bytes, and two parameter bytes.
    for maximum in [5, 6, 7] {
        let mut form = definition
            .form(json!({ "n": "Ada" }))
            .external_finding_limits(ExternalFindingLimits::new(
                NonZeroUsize::new(2).unwrap(),
                NonZeroUsize::new(maximum).unwrap(),
            ))
            .build()
            .unwrap();
        let before = snapshot(&form);
        let result = form.apply_external_findings(ExternalFindingBatch::new(
            "s",
            form.view().data_revision(),
            [ExternalFinding::blocking("x", pointer("/n"), json!({}))],
        ));
        assert_eq!(result.is_ok(), maximum >= 6);
        if maximum < 6 {
            let ExternalFindingError::ResourceLimit(error) = result.unwrap_err() else {
                panic!("expected a resource limit");
            };
            assert_eq!(error.path().as_str(), "");
            assert_limit(error, "active_external_finding_bytes", maximum, 6);
            assert_eq!(snapshot(&form), before);
        }
    }

    let mut duplicate_limited = definition
        .form(json!({ "n": "Ada" }))
        .finding_visibility(immediate_findings())
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(4).unwrap(),
            NonZeroUsize::new(7).unwrap(),
        ))
        .build()
        .unwrap();
    let duplicate = ExternalFinding::blocking("x", pointer("/n"), json!({}));
    duplicate_limited
        .apply_external_findings(ExternalFindingBatch::new(
            "s",
            duplicate_limited.view().data_revision(),
            vec![duplicate; 3],
        ))
        .unwrap();
    assert_eq!(
        duplicate_limited
            .node(node_with_binding(&duplicate_limited, "/n"))
            .unwrap()
            .external_findings()
            .count(),
        1
    );

    let mut form = definition
        .form(json!({ "n": "Ada" }))
        .finding_visibility(immediate_findings())
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(2).unwrap(),
            NonZeroUsize::new(8).unwrap(),
        ))
        .build()
        .unwrap();
    let revision = form.view().data_revision();
    form.apply_external_findings(ExternalFindingBatch::new(
        "s",
        revision,
        [ExternalFinding::blocking("x", pointer("/n"), json!({}))],
    ))
    .unwrap();
    let before = snapshot(&form);
    let error = form
        .apply_external_findings(ExternalFindingBatch::new(
            "s",
            revision,
            [ExternalFinding::blocking("long", pointer("/n"), json!({}))],
        ))
        .unwrap_err();
    let ExternalFindingError::ResourceLimit(error) = error else {
        panic!("expected a resource limit");
    };
    assert_eq!(error.path().as_str(), "");
    assert_limit(error, "active_external_finding_bytes", 8, 9);
    assert_eq!(snapshot(&form), before);

    let mut empty_limited = definition
        .form(json!({ "n": "Ada" }))
        .external_finding_limits(
            ExternalFindingLimits::new(
                NonZeroUsize::new(2).unwrap(),
                NonZeroUsize::new(1).unwrap(),
            )
            .max_incoming_bytes(1024),
        )
        .build()
        .unwrap();
    let before = snapshot(&empty_limited);
    let transition = empty_limited
        .apply_external_findings(ExternalFindingBatch::new(
            "source-longer-than-the-byte-limit",
            empty_limited.view().data_revision(),
            [],
        ))
        .expect("an empty replacement contributes zero active bytes");
    assert!(transition.is_empty());
    assert_eq!(snapshot(&empty_limited), before);
}

#[test]
fn external_finding_incoming_byte_limit_has_boundaries_and_source_policy() {
    let definition = object_definition(json!({ "n": { "type": "string" } }));
    // One source byte, one code byte, two pointer bytes, and two parameter bytes.
    for maximum in [5, 6, 7] {
        let mut form = definition
            .form(json!({ "n": "Ada" }))
            .external_finding_limits(
                ExternalFindingLimits::new(
                    NonZeroUsize::new(10).unwrap(),
                    NonZeroUsize::new(1024).unwrap(),
                )
                .max_incoming_bytes(maximum),
            )
            .build()
            .unwrap();
        let before = snapshot(&form);
        let result = form.apply_external_findings(ExternalFindingBatch::new(
            "s",
            form.view().data_revision(),
            [ExternalFinding::blocking("x", pointer("/n"), json!({}))],
        ));

        assert_eq!(result.is_ok(), maximum >= 6);
        if maximum < 6 {
            let ExternalFindingError::ResourceLimit(error) = result.unwrap_err() else {
                panic!("expected a resource limit");
            };
            assert_limit(error, "incoming_external_finding_bytes", maximum, 6);
            assert_eq!(snapshot(&form), before);
        }
    }

    let mut form = definition
        .form(json!({ "n": "Ada" }))
        .finding_visibility(immediate_findings())
        .external_finding_limits(
            ExternalFindingLimits::new(
                NonZeroUsize::new(10).unwrap(),
                NonZeroUsize::new(1024).unwrap(),
            )
            .max_incoming_bytes(6),
        )
        .build()
        .unwrap();
    let revision = form.view().data_revision();
    form.apply_external_findings(ExternalFindingBatch::new(
        "s",
        revision,
        [ExternalFinding::blocking("x", pointer("/n"), json!({}))],
    ))
    .unwrap();
    let before = snapshot(&form);
    let error = form
        .apply_external_findings(ExternalFindingBatch::new(
            "source-too-long",
            revision,
            [ExternalFinding::blocking("x", pointer("/n"), json!({}))],
        ))
        .unwrap_err();
    let ExternalFindingError::ResourceLimit(error) = error else {
        panic!("expected a resource limit");
    };
    assert_limit(
        error,
        "incoming_external_finding_bytes",
        6,
        "source-too-long".len(),
    );
    assert_eq!(snapshot(&form), before);

    let before = snapshot(&form);
    let error = form
        .apply_external_findings(ExternalFindingBatch::new("source-too-long", revision, []))
        .expect_err("an empty removal still bounds its incoming source identifier");
    assert!(matches!(
        error,
        ExternalFindingError::ResourceLimit(limit)
            if limit.dimension() == "incoming_external_finding_bytes"
    ));
    assert_eq!(snapshot(&form), before);

    let mut parameters_first = definition
        .form(json!({ "n": "Ada" }))
        .external_finding_limits(
            ExternalFindingLimits::new(
                NonZeroUsize::new(10).unwrap(),
                NonZeroUsize::new(1024).unwrap(),
            )
            .max_incoming_bytes(1)
            .max_parameter_depth(0),
        )
        .build()
        .unwrap();
    let error = parameters_first
        .apply_external_findings(ExternalFindingBatch::new(
            "s",
            parameters_first.view().data_revision(),
            [ExternalFinding::blocking(
                "x",
                pointer("/n"),
                json!({ "deep": {} }),
            )],
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        ExternalFindingError::ResourceLimit(limit)
            if limit.dimension() == "external_finding_parameter_depth"
    ));
}

#[test]
fn external_finding_parameter_shape_limits_are_enforced_before_publication() {
    let definition = object_definition(json!({ "name": { "type": "string" } }));
    let limits = ExternalFindingLimits::new(
        NonZeroUsize::new(4).unwrap(),
        NonZeroUsize::new(1024).unwrap(),
    )
    .max_parameter_depth(2)
    .max_parameter_nodes(4)
    .max_parameter_collection_length(2)
    .max_parameter_scalar_bytes(3);

    for (parameters, dimension) in [
        (
            json!({ "a": { "b": { "c": 1 } } }),
            "external_finding_parameter_depth",
        ),
        (
            json!({ "a": [0, 1], "b": [2, 3] }),
            "external_finding_parameter_nodes",
        ),
        (
            json!({ "a": 0, "b": 0, "c": 0, "d": 0, "e": 0 }),
            "external_finding_parameter_nodes",
        ),
        (
            json!([0, 1, 2]),
            "external_finding_parameter_collection_length",
        ),
        (json!("four"), "external_finding_parameter_scalar_bytes"),
    ] {
        let mut form = definition
            .form(json!({ "name": "Ada" }))
            .external_finding_limits(limits)
            .build()
            .unwrap();
        let before = snapshot(&form);
        let error = form
            .apply_external_findings(ExternalFindingBatch::new(
                "server",
                form.view().data_revision(),
                [ExternalFinding::blocking("x", pointer("/name"), parameters)],
            ))
            .unwrap_err();
        let ExternalFindingError::ResourceLimit(error) = error else {
            panic!("expected a resource limit")
        };
        assert_eq!(error.dimension(), dimension);
        assert_eq!(snapshot(&form), before);
    }

    let parameters_first = ExternalFindingLimits::new(
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(1024).unwrap(),
    )
    .max_parameter_depth(0);
    let mut form = definition
        .form(json!({ "name": "Ada" }))
        .external_finding_limits(parameters_first)
        .build()
        .unwrap();
    let error = form
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            form.view().data_revision(),
            [
                ExternalFinding::blocking("b", pointer("/name"), json!({})),
                ExternalFinding::blocking("a", pointer("/name"), json!({ "deep": {} })),
            ],
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        ExternalFindingError::ResourceLimit(limit)
            if limit.dimension() == "external_finding_parameter_depth"
    ));

    let stale_revision = form.view().data_revision();
    form.transact(|draft| draft.set(&pointer("/name"), json!("Grace")))
        .unwrap();
    let before = snapshot(&form);
    let error = form
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            stale_revision,
            [ExternalFinding::blocking(
                "invalid",
                pointer("/name"),
                json!({ "deep": {} }),
            )],
        ))
        .unwrap_err();
    assert!(matches!(error, ExternalFindingError::StaleRevision { .. }));
    assert_eq!(snapshot(&form), before);

    let mut stale_first = definition
        .form(json!({ "name": "Ada" }))
        .external_finding_limits(
            ExternalFindingLimits::new(
                NonZeroUsize::new(1).unwrap(),
                NonZeroUsize::new(1).unwrap(),
            )
            .max_incoming_findings(0)
            .max_incoming_bytes(0),
        )
        .build()
        .unwrap();
    let stale_revision = stale_first.view().data_revision();
    stale_first
        .transact(|draft| draft.set(&pointer("/name"), json!("Grace")))
        .unwrap();
    let before = snapshot(&stale_first);
    let error = stale_first
        .apply_external_findings(ExternalFindingBatch::new(
            "source-over-zero-byte-limit",
            stale_revision,
            [ExternalFinding::blocking(
                "over-zero-count-limit",
                pointer("/name"),
                json!({ "deep": {} }),
            )],
        ))
        .unwrap_err();
    assert!(matches!(error, ExternalFindingError::StaleRevision { .. }));
    assert_eq!(snapshot(&stale_first), before);
}

fn immediate_findings() -> FindingVisibilityPolicy {
    FindingVisibilityPolicy::new(FindingVisibility::Immediate, FindingVisibility::Immediate)
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    data: Value,
    data_revision: schemaform::form::DataRevision,
    state_revision: schemaform::form::StateRevision,
    validation: (Vec<String>, bool),
    visible_findings: Vec<String>,
    nodes: Vec<SnapshotNode>,
}

type SnapshotNode = (
    InstanceIdentity,
    Option<String>,
    Option<String>,
    Option<ParseBlockerKind>,
    bool,
    bool,
);

fn snapshot(form: &Form) -> Snapshot {
    let validation = match form.view().validation_outcome() {
        ValidationOutcomeView::Invalid {
            findings,
            truncated,
        } => (
            findings
                .iter()
                .map(|finding| {
                    format!(
                        "{}|{}|{}|{}",
                        finding.instance_location().as_str(),
                        finding.keyword_location().pointer().as_str(),
                        finding.code(),
                        finding.parameters()
                    )
                })
                .collect(),
            truncated,
        ),
        ValidationOutcomeView::Valid | ValidationOutcomeView::Indeterminate(_) => {
            (Vec::new(), false)
        }
    };
    let mut pending = vec![form.view().root()];
    let mut nodes = Vec::new();
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).unwrap();
        pending.extend(node.children());
        nodes.push((
            identity,
            node.binding().map(|binding| binding.pointer().to_string()),
            node.edit_buffer().map(str::to_owned),
            node.parse_blocker(),
            node.is_touched(),
            node.is_dirty(),
        ));
    }
    nodes.sort_by_key(|node| format!("{:?}", node.0));
    Snapshot {
        data: form.form_data().clone(),
        data_revision: form.view().data_revision(),
        state_revision: form.view().state_revision(),
        validation,
        visible_findings: form
            .view()
            .visible_findings()
            .map(|finding| match finding {
                FindingView::Validation { target, finding } => format!(
                    "validation|{target:?}|{}|{}|{}",
                    finding.instance_location().as_str(),
                    finding.code(),
                    finding.parameters()
                ),
                FindingView::ValidationFindingsTruncated { target, retained } => {
                    format!("validation-truncated|{target:?}|{retained}")
                }
                FindingView::Indeterminate { target, reason } => {
                    format!("indeterminate|{target:?}|{}", reason.code())
                }
                FindingView::Capability { target, finding } => format!(
                    "capability|{target:?}|{}|{}",
                    finding.code(),
                    finding.parameters()
                ),
                FindingView::External {
                    target,
                    source,
                    finding,
                } => format!(
                    "external|{target:?}|{source}|{}|{}|{}",
                    finding.code(),
                    finding.is_blocking(),
                    finding.parameters()
                ),
                FindingView::Parse { target, kind } => format!("parse|{target:?}|{kind:?}"),
                _ => "unknown-finding-family".to_owned(),
            })
            .collect(),
        nodes,
    }
}

fn object_definition(properties: Value) -> FormDefinition {
    required_object_definition(properties, &[])
}

fn required_object_definition(properties: Value, required: &[&str]) -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    }))
    .unwrap()
}

fn node_with_binding(form: &Form, binding: &str) -> InstanceIdentity {
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
    panic!("the bound node should exist")
}

fn item_identities(form: &Form, array: InstanceIdentity) -> Vec<schemaform::form::ItemIdentity> {
    form.node(array)
        .unwrap()
        .children()
        .map(|identity| {
            form.node(identity)
                .and_then(|node| node.binding())
                .and_then(|binding| binding.item())
                .unwrap()
        })
        .collect()
}

fn pointer(value: &str) -> JsonPointer {
    JsonPointer::parse(value).unwrap()
}

fn assert_transaction_value_limit(
    dimension: &'static str,
    observed: usize,
    initial: Value,
    pointer_value: &str,
    replacement: Value,
    configure: fn(FormDataLimits, usize) -> FormDataLimits,
    expected_path: &str,
) {
    let definition = object_definition(json!({}));
    for maximum in [observed - 1, observed, observed + 1] {
        let mut form = definition
            .form(initial.clone())
            .limits(configure(FormDataLimits::default(), maximum))
            .build()
            .unwrap();
        let before = snapshot(&form);
        let result = form.transact(|draft| {
            draft.set(&pointer(pointer_value), replacement.clone());
        });
        assert_eq!(result.is_ok(), maximum >= observed, "{dimension}");
        if maximum < observed {
            let error = result.unwrap_err();
            let HostCommitError::ResourceLimit(error) = error else {
                panic!("expected a resource limit, got {error:?}");
            };
            assert_eq!(error.path().as_str(), expected_path);
            assert_limit(error, dimension, maximum, observed);
            assert_eq!(snapshot(&form), before);
        }
    }
}

fn assert_resource_limit(
    error: UserOperationError,
    dimension: &'static str,
    maximum: usize,
    observed: usize,
    expected_path: &str,
) {
    let UserOperationError::ResourceLimit(error) = error else {
        panic!("expected a resource limit, got {error:?}");
    };
    assert_eq!(error.path().as_str(), expected_path);
    assert_limit(error, dimension, maximum, observed);
}

fn assert_commit_resource_limit(
    error: HostCommitError,
    dimension: &'static str,
    maximum: usize,
    observed: usize,
    expected_path: &str,
) {
    let HostCommitError::ResourceLimit(error) = error else {
        panic!("expected a resource limit, got {error:?}");
    };
    assert_eq!(error.path().as_str(), expected_path);
    assert_limit(error, dimension, maximum, observed);
}

fn assert_limit(
    error: ResourceLimitError,
    dimension: &'static str,
    maximum: usize,
    observed: usize,
) {
    assert_eq!(error.phase(), ResourceLimitPhase::Operation);
    assert_eq!(error.dimension(), dimension);
    assert_eq!(error.maximum(), maximum);
    assert_eq!(error.observed(), observed);
}
