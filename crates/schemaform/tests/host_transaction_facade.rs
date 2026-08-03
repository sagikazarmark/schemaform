use schemaform::{
    ExternalFinding, ExternalFindingBatch, Form, FormDefinition, InstanceIdentity, JsonPointer,
    SubmissionOutcome,
    form::{
        HostCommitError, ParseBlockerKind, SubmissionBlocker, TransactionError,
        ValidationOutcomeView,
    },
};
use serde_json::json;

#[test]
fn related_host_pointer_operations_publish_one_final_transition() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["first", "last", "quantity"],
        "properties": {
            "first": { "type": "string" },
            "last": { "type": "string" },
            "notes": { "type": "string" },
            "quantity": { "type": "integer", "minimum": 1 }
        }
    }))
    .expect("the transaction test schema should compile");
    let mut form = definition
        .create_form(json!({
            "first": "Ada",
            "last": "Lovelace",
            "notes": "unaffected",
            "quantity": 1
        }))
        .expect("the initial form data should create a form");
    let first = node_with_binding(&form, "/first");
    let last = node_with_binding(&form, "/last");
    let notes = node_with_binding(&form, "/notes");
    let quantity = node_with_binding(&form, "/quantity");
    let before_data = form.view().data_revision();
    let before_state = form.view().state_revision();

    let transition = form
        .transact(|draft| {
            draft.set(&pointer("/first"), json!("Grace"));
            draft.set(&pointer("/notes"), json!("unaffected"));
            draft.set(&pointer("/quantity"), json!(2));
            draft.remove(&pointer("/last"));
        })
        .expect("the related host operations should commit atomically");

    assert_eq!(
        form.form_data(),
        &json!({
            "first": "Grace",
            "notes": "unaffected",
            "quantity": 2
        })
    );
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "required")
    ));
    assert_eq!(transition.before_data_revision(), before_data);
    assert_eq!(transition.before_state_revision(), before_state);
    assert_eq!(
        transition.after_data_revision(),
        form.view().data_revision()
    );
    assert_eq!(
        transition.after_state_revision(),
        form.view().state_revision()
    );
    assert_ne!(
        transition.before_data_revision(),
        transition.after_data_revision()
    );
    assert_ne!(
        transition.before_state_revision(),
        transition.after_state_revision()
    );

    let changed = transition.changed().collect::<Vec<_>>();
    assert_eq!(changed.len(), 3);
    assert!(changed.contains(&first));
    assert!(changed.contains(&last));
    assert!(changed.contains(&quantity));
    assert!(!changed.contains(&notes));
    assert_eq!(transition.removed().count(), 0);
}

#[test]
fn failed_host_transactions_roll_back_all_observable_form_state() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "quantity"],
        "properties": {
            "name": { "type": "string" },
            "quantity": { "type": "integer" }
        }
    }))
    .expect("the rollback test schema should compile");
    let initial_data = json!({ "name": "Ada", "quantity": 1 });
    let mut form = definition
        .create_form(initial_data.clone())
        .expect("the initial form data should create a form");
    let quantity = node_with_binding(&form, "/quantity");
    form.user()
        .input_text(quantity, "-")
        .expect("the incomplete integer should remain buffered");
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "server-rejected",
            pointer("/name"),
            json!({}),
        )],
    ))
    .expect("the current external finding should apply");
    let before_data = form.view().data_revision();
    let before_state = form.view().state_revision();

    let closure_failure = form.try_transact(|draft| {
        draft.set(&pointer("/name"), json!("Grace"));
        Err("host callback failed")
    });
    assert!(matches!(
        closure_failure,
        Err(TransactionError::Closure("host callback failed"))
    ));
    assert_transaction_failure_preserved_state(
        &form,
        &initial_data,
        quantity,
        before_data,
        before_state,
    );

    let operation_failure = form.transact(|draft| {
        draft.set(&pointer("/name"), json!("Grace"));
        draft.remove(&pointer("/missing"));
    });
    assert_eq!(operation_failure, Err(HostCommitError::InvalidOperation));
    assert_transaction_failure_preserved_state(
        &form,
        &initial_data,
        quantity,
        before_data,
        before_state,
    );

    let structural_failure = form.transact(|draft| {
        draft.set(&pointer("/name"), json!("Grace"));
        draft.replace_all(json!([]));
    });
    assert_eq!(structural_failure, Err(HostCommitError::InvalidOperation));
    assert_transaction_failure_preserved_state(
        &form,
        &initial_data,
        quantity,
        before_data,
        before_state,
    );

    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Parse {
                    target,
                    kind: ParseBlockerKind::InvalidInteger
                } if *target == quantity
            )) && blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::External { source, finding }
                    if source == "server"
                        && finding.code() == "server-rejected"
                        && finding.instance_location().as_str() == "/name"
            ))
    ));
}

#[test]
fn authoritative_writes_clear_only_intersecting_edit_state() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["profile", "unrelated"],
        "properties": {
            "profile": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "quantity"],
                "properties": {
                    "name": { "type": "string" },
                    "quantity": { "type": "integer" }
                }
            },
            "unrelated": { "type": "integer" }
        }
    }))
    .expect("the intersecting-write test schema should compile");
    let mut form = definition
        .create_form(json!({
            "profile": { "name": "Ada", "quantity": 1 },
            "unrelated": 3
        }))
        .expect("the initial form data should create a form");
    let profile = node_with_binding(&form, "/profile");
    let name = node_with_binding(&form, "/profile/name");
    let quantity = node_with_binding(&form, "/profile/quantity");
    let unrelated = node_with_binding(&form, "/unrelated");

    for target in [name, quantity, unrelated] {
        form.user()
            .blur(target)
            .expect("blurring should mark each control touched");
    }
    form.user()
        .input_text(name, "Ada")
        .expect("the exact string spelling should remain buffered");
    form.user()
        .input_text(quantity, "-")
        .expect("the profile integer blocker should remain buffered");
    form.user()
        .input_text(unrelated, "-")
        .expect("the unrelated integer blocker should remain buffered");
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));
    form.user()
        .input_text(name, "Ada")
        .expect("the parseable intersecting buffer should be active at transaction time");

    let transition = form
        .transact(|draft| {
            draft.set(
                &pointer("/profile"),
                json!({ "name": "Ada", "quantity": 2 }),
            );
        })
        .expect("the authoritative parent write should commit");

    assert_eq!(
        form.form_data(),
        &json!({
            "profile": { "name": "Ada", "quantity": 2 },
            "unrelated": 3
        })
    );
    for target in [name, quantity] {
        let node = form
            .node(target)
            .expect("the intersecting control should remain instantiated");
        assert_eq!(node.edit_buffer(), None);
        assert_eq!(node.parse_blocker(), None);
        assert!(node.is_touched());
    }
    let unrelated_node = form
        .node(unrelated)
        .expect("the unrelated control should remain instantiated");
    assert_eq!(unrelated_node.edit_buffer(), Some("-"));
    assert_eq!(
        unrelated_node.parse_blocker(),
        Some(ParseBlockerKind::InvalidInteger)
    );
    assert!(unrelated_node.is_touched());
    assert!(form.view().submission_attempted());

    let changed = transition.changed().collect::<Vec<_>>();
    assert_eq!(changed.len(), 3);
    assert!(changed.contains(&profile));
    assert!(changed.contains(&name));
    assert!(changed.contains(&quantity));
    assert!(!changed.contains(&unrelated));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Parse { target, .. } if *target == unrelated
            )) && !blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Parse { target, .. } if *target == quantity
            ))
    ));
}

#[test]
fn host_transaction_operation_limit_is_enforced_atomically() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": { "type": "integer" }
        }
    }))
    .expect("the operation-limit test schema should compile");
    let initial_data = json!({ "quantity": 0 });
    let mut form = definition
        .create_form(initial_data.clone())
        .expect("the initial form data should create a form");
    let before_data = form.view().data_revision();
    let before_state = form.view().state_revision();

    let rejected = form.transact(|draft| {
        for value in 1..=257 {
            draft.set(&pointer("/quantity"), json!(value));
        }
    });

    let Err(HostCommitError::ResourceLimit(limit)) = rejected else {
        panic!("operation 257 should exceed the host transaction limit");
    };
    assert_eq!(limit.dimension(), "host_operations_per_transaction");
    assert_eq!(limit.maximum(), 256);
    assert_eq!(limit.observed(), 257);
    assert_eq!(form.form_data(), &initial_data);
    assert_eq!(form.view().data_revision(), before_data);
    assert_eq!(form.view().state_revision(), before_state);

    let accepted = form
        .transact(|draft| {
            for value in 1..=256 {
                draft.set(&pointer("/quantity"), json!(value));
            }
        })
        .expect("the documented transaction operation maximum should be accepted");
    assert_eq!(form.form_data(), &json!({ "quantity": 256 }));
    assert_ne!(
        accepted.before_data_revision(),
        accepted.after_data_revision()
    );
}

#[test]
fn mixed_host_transactions_preserve_semantically_equal_canonical_values() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["exact", "name"],
        "properties": {
            "exact": { "type": "integer" },
            "name": { "type": "string" }
        }
    }))
    .expect("the canonical-value test schema should compile");
    let initial_data = serde_json::from_str(r#"{"exact":1e3,"name":"Ada"}"#)
        .expect("the arbitrary-precision form data should parse");
    let mut form = definition
        .create_form(initial_data)
        .expect("the initial form data should create a form");
    let exact = node_with_binding(&form, "/exact");
    form.user()
        .input_text(exact, "1e3")
        .expect("the exact focused spelling should remain buffered");

    form.transact(|draft| {
        draft.set(&pointer("/exact"), json!(1000));
        draft.set(&pointer("/name"), json!("Grace"));
    })
    .expect("the mixed transaction should commit");

    assert_eq!(form.form_data()["name"], "Grace");
    assert_eq!(
        form.form_data()["exact"]
            .as_number()
            .expect("the exact value should remain numeric")
            .to_string(),
        "1e+3"
    );
    assert_eq!(
        form.node(exact)
            .expect("the exact control should remain instantiated")
            .edit_buffer(),
        None
    );
}

#[test]
fn canonical_changes_report_nodes_whose_external_findings_are_cleared() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["first", "notes"],
        "properties": {
            "first": { "type": "string" },
            "notes": { "type": "string" }
        }
    }))
    .expect("the external-finding transition schema should compile");
    let mut form = definition
        .create_form(json!({ "first": "Ada", "notes": "reviewed" }))
        .expect("the initial form data should create a form");
    let first = node_with_binding(&form, "/first");
    let notes = node_with_binding(&form, "/notes");
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "notes-rejected",
            pointer("/notes"),
            json!({}),
        )],
    ))
    .expect("the current external finding should apply");

    let transition = form
        .transact(|draft| draft.set(&pointer("/first"), json!("Grace")))
        .expect("the canonical change should commit");

    let changed = transition.changed().collect::<Vec<_>>();
    assert_eq!(changed.len(), 2);
    assert!(changed.contains(&first));
    assert!(changed.contains(&notes));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(_)
    ));
}

#[test]
fn host_sets_missing_members_and_reports_descendant_authority_changes() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "profile": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" }
                }
            }
        }
    }))
    .expect("the missing-member transaction schema should compile");
    let mut form = definition
        .create_form(json!({ "profile": "legacy" }))
        .expect("the incompatible parent data should create a form");
    let profile = node_with_binding(&form, "/profile");
    let name = node_with_binding(&form, "/profile/name");
    assert!(
        !form
            .node(name)
            .expect("the nested name control should exist")
            .allowed_operations()
            .can_input_text()
    );

    let repair = form
        .transact(|draft| draft.set(&pointer("/profile"), json!({})))
        .expect("the host should repair the incompatible parent");
    let repaired = repair.changed().collect::<Vec<_>>();
    assert_eq!(repaired.len(), 2);
    assert!(repaired.contains(&profile));
    assert!(repaired.contains(&name));
    assert!(
        form.node(name)
            .expect("the nested name control should remain")
            .allowed_operations()
            .can_input_text()
    );

    let create = form
        .transact(|draft| draft.set(&pointer("/profile/name"), json!("Grace")))
        .expect("set should create a missing member under an existing object");
    assert_eq!(form.form_data(), &json!({ "profile": { "name": "Grace" } }));
    let created = create.changed().collect::<Vec<_>>();
    assert_eq!(created.len(), 2);
    assert!(created.contains(&profile));
    assert!(created.contains(&name));
}

fn assert_transaction_failure_preserved_state(
    form: &Form,
    initial_data: &serde_json::Value,
    quantity: InstanceIdentity,
    data_revision: schemaform::DataRevision,
    state_revision: schemaform::StateRevision,
) {
    assert_eq!(form.form_data(), initial_data);
    assert_eq!(form.view().data_revision(), data_revision);
    assert_eq!(form.view().state_revision(), state_revision);
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    let quantity = form
        .node(quantity)
        .expect("the quantity control should remain instantiated");
    assert_eq!(quantity.edit_buffer(), Some("-"));
    assert_eq!(
        quantity.parse_blocker(),
        Some(ParseBlockerKind::InvalidInteger)
    );
}

fn pointer(value: &str) -> JsonPointer {
    JsonPointer::parse(value).expect("test pointers should be valid")
}

fn node_with_binding(form: &Form, binding: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).expect("the form node should exist");
        if node
            .binding()
            .is_some_and(|current| current.pointer().as_str() == binding)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("the bound form node should exist")
}
