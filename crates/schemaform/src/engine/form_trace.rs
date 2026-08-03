use crate::engine::{
    ApplyExternalFindingsError, CreateFormError, ExternalFinding, FormDefinition, ParseBlocker,
};
use serde_json::json;

#[test]
fn incomplete_integer_input_blocks_submission_without_replacing_form_data() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["customer", "quantity"],
        "properties": {
            "quantity": {
                "type": "integer",
                "minimum": 1
            },
            "customer": {
                "type": "string"
            }
        }
    }))
    .expect("the data schema should compile");

    let mut form = definition
        .create_form(json!({
            "customer": "Acme",
            "quantity": 1
        }))
        .expect("the initial form data should create a form");

    let control_bindings = form
        .controls()
        .map(|control| control.binding())
        .collect::<Vec<_>>();
    assert_eq!(control_bindings, ["/customer", "/quantity"]);

    form.edit_text("/quantity", "-")
        .expect("quantity should accept an edit buffer");

    assert_eq!(
        form.form_data(),
        &json!({
            "customer": "Acme",
            "quantity": 1
        })
    );

    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), Some("-"));
    assert_eq!(quantity.parse_blocker(), Some(ParseBlocker::InvalidInteger));

    let blocked = form
        .prepare_submission()
        .expect_err("an incomplete integer must block submission");
    assert!(blocked.has_parse_blocker("/quantity"));

    form.edit_text("/quantity", "2")
        .expect("quantity should accept a corrected integer");

    assert_eq!(
        form.form_data(),
        &json!({
            "customer": "Acme",
            "quantity": 2
        })
    );

    let snapshot = form
        .prepare_submission()
        .expect("corrected form data should be submittable");
    assert_eq!(
        snapshot.form_data(),
        &json!({
            "customer": "Acme",
            "quantity": 2
        })
    );
}

#[test]
fn mathematical_integer_edits_preserve_arbitrary_precision() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");

    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");

    form.edit_text("/quantity", "184467440737095516160")
        .expect("quantity should accept an integer beyond u64");
    assert_eq!(
        form.form_data(),
        &serde_json::from_str::<serde_json::Value>(r#"{"quantity":184467440737095516160}"#)
            .expect("the expected form data should be valid JSON")
    );

    form.edit_text("/quantity", "1e3")
        .expect("quantity should accept an exponent with an integer value");

    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), Some("1e3"));
    assert_eq!(quantity.parse_blocker(), None);
    assert_eq!(form.form_data(), &json!({ "quantity": 1000 }));

    form.edit_text("/quantity", "0e9223372036854775808")
        .expect("zero should remain exact with an arbitrary exponent");
    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), Some("0e9223372036854775808"));
    assert_eq!(quantity.parse_blocker(), None);
    assert_eq!(form.form_data(), &json!({ "quantity": 0 }));
}

#[test]
fn integer_exponent_expansion_is_resource_bounded() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");

    form.edit_text("/quantity", "1e4097")
        .expect("quantity should retain an over-budget edit buffer");

    assert_eq!(form.form_data(), &json!({ "quantity": 1 }));
    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), Some("1e4097"));
    assert_eq!(
        quantity.parse_blocker(),
        Some(ParseBlocker::ResourceLimitExceeded)
    );
}

#[test]
fn mathematically_equal_integer_edit_does_not_rewrite_form_data() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let initial_form_data = serde_json::from_str::<serde_json::Value>(r#"{"quantity":1e3}"#)
        .expect("the initial form data should be valid JSON");
    let mut form = definition
        .create_form(initial_form_data.clone())
        .expect("the initial form data should create a form");

    form.edit_text("/quantity", "1e3")
        .expect("quantity should accept the equivalent integer spelling");

    assert_eq!(form.form_data(), &initial_form_data);
    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), Some("1e3"));
    assert_eq!(quantity.parse_blocker(), None);

    form.blur("/quantity")
        .expect("blurring an equivalent integer should finalize its buffer");
    assert_eq!(form.form_data(), &initial_form_data);
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        None
    );
}

#[test]
fn integer_digit_budget_applies_to_the_canonical_value() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");

    form.edit_text("/quantity", "0.001e4098")
        .expect("the canonical 4096-digit integer should fit the budget");
    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.parse_blocker(), None);
    assert_eq!(
        form.form_data()["quantity"]
            .as_number()
            .expect("quantity should be a number")
            .to_string()
            .len(),
        4096
    );

    let over_budget = format!("1{}e-1", "0".repeat(4097));
    form.edit_text("/quantity", &over_budget)
        .expect("the over-budget integer should remain an edit buffer");
    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(
        quantity.parse_blocker(),
        Some(ParseBlocker::ResourceLimitExceeded)
    );
}

#[test]
fn blur_finalizes_valid_integer_buffers_but_retains_blocked_input() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");

    form.edit_text("/quantity", "1e3")
        .expect("quantity should accept a valid focused buffer");
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        Some("1e3")
    );

    form.blur("/quantity")
        .expect("blurring a valid quantity should finalize its buffer");
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        None
    );
    assert_eq!(form.form_data(), &json!({ "quantity": 1000 }));

    form.edit_text("/quantity", "-")
        .expect("quantity should retain an incomplete focused buffer");
    form.blur("/quantity")
        .expect("blurring an incomplete quantity should preserve its buffer");

    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), Some("-"));
    assert_eq!(quantity.parse_blocker(), Some(ParseBlocker::InvalidInteger));
    assert_eq!(form.form_data(), &json!({ "quantity": 1000 }));
}

#[test]
fn submission_finalizes_valid_buffers_but_retains_blocked_input() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity", "other_quantity", "third_quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            },
            "other_quantity": {
                "type": "integer"
            },
            "third_quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let initial_form_data = serde_json::from_str::<serde_json::Value>(
        r#"{"quantity":1e3,"other_quantity":2,"third_quantity":3}"#,
    )
    .expect("the initial form data should be valid JSON");
    let mut form = definition
        .create_form(initial_form_data.clone())
        .expect("the initial form data should create a form");

    form.edit_text("/quantity", "1e3")
        .expect("quantity should accept a valid focused buffer");
    let snapshot = form
        .prepare_submission()
        .expect("a valid buffer should be submittable");

    assert_eq!(snapshot.form_data(), &initial_form_data);
    assert_eq!(form.form_data(), &initial_form_data);
    assert_eq!((form.data_revision(), form.state_revision()), (0, 2));
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        None
    );

    form.edit_text("/quantity", "1e3")
        .expect("quantity should accept an equivalent valid buffer");
    form.edit_text("/other_quantity", "-")
        .expect("other quantity should retain an incomplete buffer");
    form.edit_text("/third_quantity", "1e")
        .expect("third quantity should retain an incomplete buffer");
    let blocked = form
        .prepare_submission()
        .expect_err("incomplete integers must block submission");

    assert!(blocked.has_parse_blocker("/other_quantity"));
    assert!(blocked.has_parse_blocker("/third_quantity"));
    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), None);
    assert_eq!(quantity.parse_blocker(), None);
    let other_quantity = form
        .control("/other_quantity")
        .expect("the generated other quantity control should exist");
    assert_eq!(other_quantity.edit_buffer(), Some("-"));
    assert_eq!(
        other_quantity.parse_blocker(),
        Some(ParseBlocker::InvalidInteger)
    );
    let third_quantity = form
        .control("/third_quantity")
        .expect("the generated third quantity control should exist");
    assert_eq!(third_quantity.edit_buffer(), Some("1e"));
    assert_eq!(
        third_quantity.parse_blocker(),
        Some(ParseBlocker::InvalidInteger)
    );
    assert_eq!(form.form_data(), &initial_form_data);
}

#[test]
fn revisions_distinguish_form_data_changes_from_other_form_state() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");

    assert_eq!((form.data_revision(), form.state_revision()), (0, 0));

    form.edit_text("/quantity", "-")
        .expect("quantity should retain an incomplete buffer");
    assert_eq!((form.data_revision(), form.state_revision()), (0, 1));

    form.edit_text("/quantity", "-")
        .expect("repeating an incomplete buffer should be a no-op");
    assert_eq!((form.data_revision(), form.state_revision()), (0, 1));

    form.edit_text("/quantity", "1")
        .expect("quantity should accept an equivalent integer");
    assert_eq!((form.data_revision(), form.state_revision()), (0, 2));

    form.edit_text("/quantity", "2")
        .expect("quantity should accept a changed integer");
    assert_eq!((form.data_revision(), form.state_revision()), (1, 3));

    form.blur("/quantity")
        .expect("blur should finalize the valid buffer");
    assert_eq!((form.data_revision(), form.state_revision()), (1, 4));

    form.blur("/quantity")
        .expect("repeated blur should be a no-op");
    assert_eq!((form.data_revision(), form.state_revision()), (1, 4));

    form.edit_text("/quantity", "2")
        .expect("quantity should retain an equivalent focused buffer");
    assert_eq!((form.data_revision(), form.state_revision()), (1, 5));

    let snapshot = form
        .prepare_submission()
        .expect("the valid buffer should be submittable");
    assert_eq!(snapshot.data_revision(), 1);
    assert_eq!((form.data_revision(), form.state_revision()), (1, 6));
}

#[test]
fn failed_edits_leave_form_state_and_revisions_unchanged() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "details": {
                "type": "object",
                "properties": {
                    "quantity": {
                        "type": "integer"
                    }
                }
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({}))
        .expect("absent optional form data should create a form");

    let result = form.edit_text("/details/quantity", "2");

    assert!(matches!(
        result,
        Err(crate::engine::EditError::UnresolvedControl(binding)) if binding == "/details/quantity"
    ));
    assert_eq!(form.form_data(), &json!({}));
    let quantity = form
        .control("/details/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), None);
    assert_eq!(quantity.parse_blocker(), None);
    assert_eq!((form.data_revision(), form.state_revision()), (0, 0));
}

#[test]
fn external_findings_apply_only_to_their_form_data_revision() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");
    let finding = ExternalFinding::blocking("/quantity", "server-rejected")
        .expect("the external finding should have a valid instance pointer");
    let replacement = ExternalFinding::blocking("/quantity", "server-retry-required")
        .expect("the replacement finding should have a valid instance pointer");
    let policy = ExternalFinding::blocking("/quantity", "policy-rejected")
        .expect("the policy finding should have a valid instance pointer");

    form.apply_external_findings("server", 0, vec![finding.clone()])
        .expect("a current external finding batch should apply");
    form.apply_external_findings("policy", 0, vec![policy.clone()])
        .expect("a second source should apply independently");
    form.apply_external_findings("server", 0, vec![replacement.clone()])
        .expect("a current source should replace its prior batch");

    assert_eq!((form.data_revision(), form.state_revision()), (0, 3));
    let stale_replacement = form
        .apply_external_findings("server", 1, vec![finding.clone()])
        .expect_err("a mismatched revision must not replace a current batch");
    assert!(matches!(
        stale_replacement,
        ApplyExternalFindingsError::StaleDataRevision {
            current: 0,
            supplied: 1
        }
    ));
    assert_eq!((form.data_revision(), form.state_revision()), (0, 3));

    let blocked = form
        .prepare_submission()
        .expect_err("a current blocking external finding must block submission");
    assert!(!blocked.has_external_blocker("server", "/quantity", "server-rejected"));
    assert!(blocked.has_external_blocker("server", "/quantity", "server-retry-required"));
    assert!(blocked.has_external_blocker("policy", "/quantity", "policy-rejected"));

    form.edit_text("/quantity", "2")
        .expect("quantity should accept a changed integer");
    assert_eq!((form.data_revision(), form.state_revision()), (1, 5));
    let snapshot = form
        .prepare_submission()
        .expect("a canonical edit should invalidate the old external finding batch");
    assert_eq!(snapshot.form_data(), &json!({ "quantity": 2 }));
    assert_eq!((form.data_revision(), form.state_revision()), (1, 6));

    form.apply_external_findings("server", 1, vec![replacement])
        .expect("the current server batch should apply");
    form.apply_external_findings("policy", 1, vec![policy])
        .expect("the current policy batch should apply");
    assert_eq!((form.data_revision(), form.state_revision()), (1, 8));

    let stale = form
        .apply_external_findings("server", 0, vec![finding])
        .expect_err("an old external finding batch must be rejected");
    assert!(matches!(
        stale,
        ApplyExternalFindingsError::StaleDataRevision {
            current: 1,
            supplied: 0
        }
    ));
    assert_eq!((form.data_revision(), form.state_revision()), (1, 8));
    let blocked = form
        .prepare_submission()
        .expect_err("the current external finding batches must survive stale rejection");
    assert!(blocked.has_external_blocker("server", "/quantity", "server-retry-required"));
    assert!(blocked.has_external_blocker("policy", "/quantity", "policy-rejected"));
    assert_eq!((form.data_revision(), form.state_revision()), (1, 8));
}

#[test]
fn advisory_external_findings_are_visible_without_blocking_submission() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");
    let advisory = ExternalFinding::advisory("/quantity", "check-recommended-value")
        .expect("the advisory finding should have a valid instance pointer");

    form.apply_external_findings("server", 0, vec![advisory.clone()])
        .expect("the advisory finding batch should apply");

    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    let findings = quantity
        .external_findings()
        .map(|finding| (finding.source(), finding.code(), finding.is_blocking()))
        .collect::<Vec<_>>();
    assert_eq!(findings, [("server", "check-recommended-value", false)]);
    form.prepare_submission()
        .expect("an advisory external finding must not block submission");

    let blocking = ExternalFinding::blocking("/quantity", "server-rejected")
        .expect("the blocking finding should have a valid instance pointer");
    form.apply_external_findings("server", 0, vec![advisory, blocking])
        .expect("the mixed finding batch should replace the source");

    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    let findings = quantity
        .external_findings()
        .map(|finding| (finding.source(), finding.code(), finding.is_blocking()))
        .collect::<Vec<_>>();
    assert_eq!(
        findings,
        [
            ("server", "check-recommended-value", false),
            ("server", "server-rejected", true)
        ]
    );
    let blocked = form
        .prepare_submission()
        .expect_err("the mixed batch must block on its blocking finding");
    assert!(blocked.has_external_blocker("server", "/quantity", "server-rejected"));

    form.apply_external_findings("server", 0, Vec::new())
        .expect("an empty batch should clear the source");
    assert_eq!(form.external_findings().count(), 0);
    assert_eq!(form.state_revision(), 4);
    form.apply_external_findings("server", 0, Vec::new())
        .expect("clearing an absent source should be a no-op");
    assert_eq!(form.state_revision(), 4);
    form.prepare_submission()
        .expect("clearing the source should allow submission");
}

#[test]
fn external_finding_summaries_are_ordered_deduplicated_and_complete() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");
    let quantity = ExternalFinding::advisory("/quantity", "check-value")
        .expect("the control finding should have a valid instance pointer");
    let root = ExternalFinding::advisory("", "check-object")
        .expect("the root finding should have a valid instance pointer");
    let unmatched = ExternalFinding::blocking("/missing", "server-rejected")
        .expect("the unmatched finding should have a valid instance pointer");

    form.apply_external_findings(
        "z-source",
        0,
        vec![quantity.clone(), root.clone(), quantity.clone()],
    )
    .expect("the first finding batch should apply");
    form.apply_external_findings("a-source", 0, vec![unmatched])
        .expect("the second finding batch should apply");

    let summary = form
        .external_findings()
        .map(|finding| {
            (
                finding.source(),
                finding.instance_pointer(),
                finding.code(),
                finding.is_blocking(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            ("a-source", "/missing", "server-rejected", true),
            ("z-source", "", "check-object", false),
            ("z-source", "/quantity", "check-value", false),
        ]
    );
    let quantity_findings = form
        .control("/quantity")
        .expect("the generated quantity control should exist")
        .external_findings()
        .map(|finding| (finding.source(), finding.code()))
        .collect::<Vec<_>>();
    assert_eq!(quantity_findings, [("z-source", "check-value")]);

    form.apply_external_findings("z-source", 0, vec![quantity.clone(), quantity, root])
        .expect("equivalent reordered findings should apply");
    assert_eq!(
        form.state_revision(),
        2,
        "normalization should make an equivalent batch a no-op"
    );
    let blocked = form
        .prepare_submission()
        .expect_err("the discoverable unmatched blocking finding must block submission");
    assert!(blocked.has_external_blocker("a-source", "/missing", "server-rejected"));
}

#[test]
fn scalar_controls_track_dirty_state_against_the_initial_baseline() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["customer", "legacy_quantity", "quantity"],
        "properties": {
            "customer": {
                "type": "string"
            },
            "legacy_quantity": {
                "type": "integer"
            },
            "note": {
                "type": "string"
            },
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let baseline: serde_json::Value =
        serde_json::from_str(r#"{"customer":"Ada","legacy_quantity":"unknown","quantity":1e3}"#)
            .expect("the arbitrary-precision baseline should parse");
    let mut form = definition
        .create_form(baseline.clone())
        .expect("the initial form data should create a form");

    assert!(
        !form
            .control("/customer")
            .expect("the generated customer control should exist")
            .is_dirty()
    );
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_dirty()
    );
    assert!(
        !form
            .control("/note")
            .expect("the generated missing-value control should exist")
            .is_dirty()
    );
    assert!(
        !form
            .control("/legacy_quantity")
            .expect("the generated incompatible-value control should exist")
            .is_dirty()
    );

    form.edit_text("/quantity", "-")
        .expect("an incomplete integer should remain buffered");
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_dirty(),
        "an uncommitted buffer must not change canonical dirty state"
    );

    form.edit_text("/quantity", "1000")
        .expect("an equivalent integer spelling should parse");
    assert_eq!(form.form_data(), &baseline);
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_dirty()
    );

    form.edit_text("/quantity", "2")
        .expect("a changed integer should parse");
    assert!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .is_dirty()
    );
    form.edit_text("/quantity", "1000")
        .expect("the baseline integer should parse from plain notation");
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_dirty()
    );

    form.edit_text("/customer", "Grace")
        .expect("a changed string should edit");
    assert!(
        form.control("/customer")
            .expect("the generated customer control should exist")
            .is_dirty()
    );
    form.edit_text("/customer", "Ada")
        .expect("the baseline string should edit");
    assert!(
        !form
            .control("/customer")
            .expect("the generated customer control should exist")
            .is_dirty()
    );
}

#[test]
fn reset_restores_the_baseline_and_clears_transient_state_atomically() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["customer", "quantity"],
        "properties": {
            "customer": {
                "type": "string"
            },
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let baseline = json!({ "customer": "Ada", "quantity": 1 });
    let mut form = definition
        .create_form(baseline.clone())
        .expect("the initial form data should create a form");

    form.edit_text("/customer", "Grace")
        .expect("the customer should edit");
    form.edit_text("/quantity", "2")
        .expect("the quantity should edit");
    let finding = ExternalFinding::blocking("/quantity", "server-rejected")
        .expect("the finding should have a valid instance pointer");
    form.apply_external_findings("server", 2, vec![finding])
        .expect("the current finding batch should apply");
    form.edit_text("/quantity", "-")
        .expect("the incomplete quantity should remain buffered");
    assert_eq!((form.data_revision(), form.state_revision()), (2, 4));

    form.reset();

    assert_eq!(form.form_data(), &baseline);
    assert_eq!((form.data_revision(), form.state_revision()), (3, 5));
    assert_eq!(form.external_findings().count(), 0);
    for binding in ["/customer", "/quantity"] {
        let control = form
            .control(binding)
            .expect("the generated control should exist");
        assert_eq!(control.edit_buffer(), None);
        assert_eq!(control.parse_blocker(), None);
        assert!(!control.is_dirty());
    }
    form.reset();
    assert_eq!(
        (form.data_revision(), form.state_revision()),
        (3, 5),
        "resetting an already reset form should be a no-op"
    );
    form.prepare_submission()
        .expect("the restored baseline should be submittable");
}

#[test]
fn reset_does_not_rewrite_a_mathematically_equivalent_number() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let baseline = serde_json::from_str(r#"{"quantity":1e3}"#)
        .expect("the arbitrary-precision baseline should parse");
    let mut form = definition
        .create_form(baseline)
        .expect("the initial form data should create a form");

    form.edit_text("/quantity", "2")
        .expect("a changed quantity should edit");
    form.edit_text("/quantity", "1000")
        .expect("the equivalent baseline quantity should edit");
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_dirty()
    );
    assert_eq!((form.data_revision(), form.state_revision()), (2, 2));

    form.reset();

    assert_eq!(form.form_data(), &json!({ "quantity": 1000 }));
    assert_eq!((form.data_revision(), form.state_revision()), (2, 3));
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        None
    );
}

#[test]
fn reset_preserves_external_findings_for_unchanged_form_data() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");
    let finding = ExternalFinding::blocking("/quantity", "server-rejected")
        .expect("the finding should have a valid instance pointer");
    form.apply_external_findings("server", 0, vec![finding])
        .expect("the current finding batch should apply");
    form.edit_text("/quantity", "-")
        .expect("the incomplete quantity should remain buffered");

    form.reset();

    assert_eq!((form.data_revision(), form.state_revision()), (0, 3));
    assert_eq!(form.external_findings().count(), 1);
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        None
    );
    let blocked = form
        .prepare_submission()
        .expect_err("the still-current external finding should remain blocking");
    assert!(blocked.has_external_blocker("server", "/quantity", "server-rejected"));
}

#[test]
fn reinitialize_installs_a_new_baseline_and_clears_stale_state_atomically() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["customer", "quantity"],
        "properties": {
            "customer": {
                "type": "string"
            },
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "customer": "Ada", "quantity": 1 }))
        .expect("the initial form data should create a form");
    form.edit_text("/customer", "Grace")
        .expect("the customer should edit");
    form.edit_text("/quantity", "-")
        .expect("the incomplete quantity should remain buffered");
    let finding = ExternalFinding::blocking("/quantity", "server-rejected")
        .expect("the finding should have a valid instance pointer");
    form.apply_external_findings("server", 1, vec![finding.clone()])
        .expect("the current finding batch should apply");

    let rejected = form
        .reinitialize(json!([]))
        .expect_err("non-object replacement data must be rejected");
    assert_eq!(rejected, CreateFormError::FormDataMustBeObject);
    assert_eq!(
        form.form_data(),
        &json!({ "customer": "Grace", "quantity": 1 })
    );
    assert_eq!((form.data_revision(), form.state_revision()), (1, 3));
    assert_eq!(form.external_findings().count(), 1);
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        Some("-")
    );

    let new_baseline = json!({ "customer": "Lin", "quantity": 5 });
    form.reinitialize(new_baseline.clone())
        .expect("the replacement data should reinitialize the form");

    assert_eq!(form.form_data(), &new_baseline);
    assert_eq!((form.data_revision(), form.state_revision()), (2, 4));
    assert_eq!(form.external_findings().count(), 0);
    for binding in ["/customer", "/quantity"] {
        let control = form
            .control(binding)
            .expect("the generated control should exist");
        assert_eq!(control.edit_buffer(), None);
        assert_eq!(control.parse_blocker(), None);
        assert!(!control.is_dirty());
    }
    let snapshot = form
        .prepare_submission()
        .expect("the replacement data should be submittable");
    assert_eq!(snapshot.form_data(), &new_baseline);
    assert_eq!(snapshot.data_revision(), 2);

    form.apply_external_findings("server", 2, vec![finding.clone()])
        .expect("a finding for the replacement data should apply");
    form.reinitialize(new_baseline.clone())
        .expect("reinitializing equal data should clear lifecycle state");
    assert_eq!((form.data_revision(), form.state_revision()), (3, 7));
    assert_eq!(form.external_findings().count(), 0);
    let stale = form
        .apply_external_findings("server", 2, vec![finding])
        .expect_err("findings started before reinitialization must be rejected");
    assert!(matches!(
        stale,
        ApplyExternalFindingsError::StaleDataRevision {
            current: 3,
            supplied: 2
        }
    ));
    form.reinitialize(new_baseline.clone())
        .expect("reinitializing an already clean form should succeed");
    assert_eq!(
        (form.data_revision(), form.state_revision()),
        (4, 8),
        "each explicit reinitialization should establish a new revision token"
    );

    form.edit_text("/quantity", "6")
        .expect("the replacement quantity should edit");
    form.reset();
    assert_eq!(form.form_data(), &new_baseline);
    assert_eq!((form.data_revision(), form.state_revision()), (6, 10));
}

#[test]
fn reinitialize_can_adopt_current_data_as_the_new_baseline() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "quantity": 1 }))
        .expect("the initial form data should create a form");
    form.edit_text("/quantity", "2")
        .expect("the quantity should edit");
    assert!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .is_dirty()
    );

    form.reinitialize(form.form_data().clone())
        .expect("the current data should become the new baseline");

    assert_eq!(form.form_data(), &json!({ "quantity": 2 }));
    assert_eq!((form.data_revision(), form.state_revision()), (2, 2));
    let quantity = form
        .control("/quantity")
        .expect("the generated quantity control should exist");
    assert_eq!(quantity.edit_buffer(), None);
    assert!(!quantity.is_dirty());
    form.reset();
    assert_eq!((form.data_revision(), form.state_revision()), (2, 2));
}

#[test]
fn host_replacement_updates_current_data_without_changing_the_baseline() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["customer", "quantity"],
        "properties": {
            "customer": {
                "type": "string"
            },
            "quantity": {
                "type": "integer"
            }
        }
    }))
    .expect("the data schema should compile");
    let baseline = json!({ "customer": "Ada", "quantity": 1 });
    let mut form = definition
        .create_form(baseline.clone())
        .expect("the initial form data should create a form");
    form.edit_text("/customer", "Grace")
        .expect("the customer should edit");
    form.edit_text("/quantity", "-")
        .expect("the incomplete quantity should remain buffered");
    let finding = ExternalFinding::blocking("/quantity", "server-rejected")
        .expect("the finding should have a valid instance pointer");
    form.apply_external_findings("server", 1, vec![finding.clone()])
        .expect("the current finding batch should apply");

    let rejected = form
        .replace_form_data(json!([]))
        .expect_err("a non-object replacement must be rejected");
    assert_eq!(rejected, CreateFormError::FormDataMustBeObject);
    assert_eq!(
        form.form_data(),
        &json!({ "customer": "Grace", "quantity": 1 })
    );
    assert_eq!((form.data_revision(), form.state_revision()), (1, 3));
    assert_eq!(form.external_findings().count(), 1);
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        Some("-")
    );

    let replacement = json!({ "customer": "Lin", "quantity": 5 });
    form.replace_form_data(replacement.clone())
        .expect("the host replacement should apply");

    assert_eq!(form.form_data(), &replacement);
    assert_eq!((form.data_revision(), form.state_revision()), (2, 4));
    assert_eq!(form.external_findings().count(), 0);
    for binding in ["/customer", "/quantity"] {
        let control = form
            .control(binding)
            .expect("the generated control should exist");
        assert_eq!(control.edit_buffer(), None);
        assert_eq!(control.parse_blocker(), None);
        assert!(control.is_dirty());
    }

    let stale = form
        .apply_external_findings("server", 1, vec![finding.clone()])
        .expect_err("findings started before host replacement must be rejected");
    assert!(matches!(
        stale,
        ApplyExternalFindingsError::StaleDataRevision {
            current: 2,
            supplied: 1
        }
    ));

    form.apply_external_findings("server", 2, vec![finding])
        .expect("a finding for the replacement data should apply");
    form.edit_text("/quantity", "-")
        .expect("the incomplete replacement quantity should remain buffered");
    let equivalent_replacement = serde_json::from_str(r#"{"customer":"Lin","quantity":5e0}"#)
        .expect("the equivalent host replacement should parse");
    form.replace_form_data(equivalent_replacement)
        .expect("an equal host replacement should reconcile edit state");
    assert_eq!((form.data_revision(), form.state_revision()), (2, 7));
    assert_eq!(form.external_findings().count(), 1);
    assert_eq!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .edit_buffer(),
        None
    );
    let blocked = form
        .prepare_submission()
        .expect_err("a still-current external finding should remain blocking");
    assert!(blocked.has_external_blocker("server", "/quantity", "server-rejected"));

    form.replace_form_data(replacement)
        .expect("an already reconciled equal replacement should succeed");
    assert_eq!(
        (form.data_revision(), form.state_revision()),
        (2, 8),
        "an equal clean host replacement should be a no-op"
    );
    form.reset();
    assert_eq!(form.form_data(), &baseline);
    assert_eq!((form.data_revision(), form.state_revision()), (3, 9));
}

#[test]
fn submission_snapshots_identify_their_compiled_definition() {
    let schema = serde_json::from_str(
        r#"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["customer", "quantity"],
            "properties": {
                "quantity": { "minimum": 1e3, "type": "integer" },
                "customer": { "type": "string" }
            }
        }"#,
    )
    .expect("the first data schema should parse");
    let equivalent_schema = serde_json::from_str(
        r#"{
            "properties": {
                "customer": { "type": "string" },
                "quantity": { "type": "integer", "minimum": 1000 }
            },
            "required": ["quantity", "customer"],
            "$comment": "ignored annotation",
            "type": "object",
            "$schema": "https://json-schema.org/draft/2020-12/schema"
        }"#,
    )
    .expect("the reordered data schema should parse");
    let different_schema = serde_json::from_str(
        r#"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "customer": { "type": "string" },
                "quantity": { "type": "string" }
            }
        }"#,
    )
    .expect("the different data schema should parse");

    let definition = FormDefinition::compile(schema).expect("the data schema should compile");
    let equivalent_definition = FormDefinition::compile(equivalent_schema)
        .expect("the equivalent data schema should compile");
    let different_definition = FormDefinition::compile(different_schema)
        .expect("the different data schema should compile");
    let fingerprint = definition.fingerprint();

    assert_eq!(fingerprint, equivalent_definition.fingerprint());
    assert_ne!(fingerprint, different_definition.fingerprint());

    let mut form = definition
        .create_form(json!({ "customer": "Ada", "quantity": 1000 }))
        .expect("the initial form data should create a form");
    let snapshot = form
        .prepare_submission()
        .expect("the initial form should be submittable");
    assert_eq!(snapshot.definition_fingerprint(), fingerprint);

    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FormDefinition>();
    assert_send_sync::<crate::engine::SubmissionSnapshot>();
}

#[test]
fn submission_failures_group_structured_blockers_by_source() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "customer": { "type": "string" },
            "large_quantity": { "type": "integer" },
            "quantity": { "type": "integer" }
        }
    }))
    .expect("the data schema should compile");
    let mut form = definition
        .create_form(json!({ "customer": "Ada", "large_quantity": 1, "quantity": 1 }))
        .expect("the initial form data should create a form");
    form.edit_text("/large_quantity", "1e4096")
        .expect("the over-budget quantity should remain buffered");
    form.edit_text("/quantity", "-")
        .expect("the incomplete quantity should remain buffered");
    let advisory = ExternalFinding::advisory("/quantity", "check-value")
        .expect("the advisory should have a valid instance pointer");
    let server = ExternalFinding::blocking("/customer", "server-rejected")
        .expect("the server blocker should have a valid instance pointer");
    let policy = ExternalFinding::blocking("/quantity", "policy-rejected")
        .expect("the policy blocker should have a valid instance pointer");
    form.apply_external_findings("server", 0, vec![advisory, server])
        .expect("the server finding batch should apply");
    form.apply_external_findings("policy", 0, vec![policy])
        .expect("the policy finding batch should apply");

    let failure = form
        .prepare_submission()
        .expect_err("the parse and external blockers must prevent submission");

    let parse_blockers = failure
        .parse_blockers()
        .map(|blocker| (blocker.binding(), blocker.reason()))
        .collect::<Vec<_>>();
    assert_eq!(
        parse_blockers,
        [
            ("/large_quantity", ParseBlocker::ResourceLimitExceeded),
            ("/quantity", ParseBlocker::InvalidInteger),
        ]
    );
    let external_blockers = failure
        .external_blockers()
        .map(|blocker| (blocker.source(), blocker.instance_pointer(), blocker.code()))
        .collect::<Vec<_>>();
    assert_eq!(
        external_blockers,
        [
            ("policy", "/quantity", "policy-rejected"),
            ("server", "/customer", "server-rejected"),
        ]
    );
}

#[test]
fn submission_attempt_metadata_tracks_attempts_and_lifecycle_boundaries() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "quantity": { "type": "integer" }
        }
    }))
    .expect("the data schema should compile");
    let initial_data = json!({ "quantity": 1 });
    let mut form = definition
        .create_form(initial_data.clone())
        .expect("the initial form data should create a form");
    assert!(!form.submission_attempted());

    form.prepare_submission()
        .expect("the initial form should be submittable");
    assert!(form.submission_attempted());
    assert_eq!((form.data_revision(), form.state_revision()), (0, 1));
    form.prepare_submission()
        .expect("repeated preparation should remain successful");
    assert_eq!((form.data_revision(), form.state_revision()), (0, 1));

    form.edit_text("/quantity", "-")
        .expect("the incomplete quantity should remain buffered");
    form.prepare_submission()
        .expect_err("the parse blocker should prevent submission");
    assert!(form.submission_attempted());
    assert_eq!((form.data_revision(), form.state_revision()), (0, 2));

    form.reset();
    assert!(!form.submission_attempted());
    assert_eq!((form.data_revision(), form.state_revision()), (0, 3));

    form.edit_text("/quantity", "-")
        .expect("the incomplete quantity should remain buffered");
    form.prepare_submission()
        .expect_err("the first failed attempt should update metadata");
    assert!(form.submission_attempted());
    assert_eq!((form.data_revision(), form.state_revision()), (0, 5));

    form.reinitialize(initial_data.clone())
        .expect("reinitialization should clear attempt metadata");
    assert!(!form.submission_attempted());
    assert_eq!((form.data_revision(), form.state_revision()), (1, 6));

    form.prepare_submission()
        .expect("the reinitialized form should be submittable");
    form.replace_form_data(json!({ "quantity": 2 }))
        .expect("ordinary host replacement should preserve attempt metadata");
    assert!(form.submission_attempted());
    assert_eq!((form.data_revision(), form.state_revision()), (2, 8));
}

#[test]
fn controls_become_touched_on_blur_and_clear_at_lifecycle_boundaries() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "customer": { "type": "string" },
            "quantity": { "type": "integer" }
        }
    }))
    .expect("the data schema should compile");
    let baseline = json!({ "customer": "Ada", "quantity": 1 });
    let mut form = definition
        .create_form(baseline.clone())
        .expect("the initial form data should create a form");
    assert!(
        !form
            .control("/customer")
            .expect("the generated customer control should exist")
            .is_touched()
    );
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_touched()
    );

    form.blur("/customer")
        .expect("blurring an unedited control should succeed");
    assert!(
        form.control("/customer")
            .expect("the generated customer control should exist")
            .is_touched()
    );
    assert_eq!((form.data_revision(), form.state_revision()), (0, 1));
    form.blur("/customer")
        .expect("repeated blur should succeed");
    assert_eq!((form.data_revision(), form.state_revision()), (0, 1));

    form.edit_text("/quantity", "2")
        .expect("the quantity should edit");
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_touched()
    );
    form.blur("/quantity")
        .expect("blurring the edited quantity should succeed");
    assert!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .is_touched()
    );
    assert_eq!((form.data_revision(), form.state_revision()), (1, 3));

    form.replace_form_data(json!({ "customer": "Lin", "quantity": 5 }))
        .expect("host replacement should apply");
    assert!(
        form.control("/customer")
            .expect("the generated customer control should exist")
            .is_touched()
    );
    assert!(
        form.control("/quantity")
            .expect("the generated quantity control should exist")
            .is_touched()
    );
    assert_eq!((form.data_revision(), form.state_revision()), (2, 4));

    form.reset();
    assert_eq!(form.form_data(), &baseline);
    assert!(
        !form
            .control("/customer")
            .expect("the generated customer control should exist")
            .is_touched()
    );
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_touched()
    );
    assert_eq!((form.data_revision(), form.state_revision()), (3, 5));

    form.blur("/quantity")
        .expect("the reset quantity should become touched");
    form.reinitialize(baseline)
        .expect("reinitialization should clear touched state");
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_touched()
    );
    assert_eq!((form.data_revision(), form.state_revision()), (4, 7));

    form.prepare_submission()
        .expect("submission should not mark controls touched");
    assert!(
        !form
            .control("/customer")
            .expect("the generated customer control should exist")
            .is_touched()
    );
    assert!(
        !form
            .control("/quantity")
            .expect("the generated quantity control should exist")
            .is_touched()
    );
}
