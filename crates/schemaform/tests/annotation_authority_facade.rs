use schemaform::{
    Form, FormDefinition, InstanceIdentity, JsonPointer, SubmissionOutcome,
    form::{AllowedOperations, SubmissionBlocker, UserOperationError, ValidationOutcomeView},
};
use serde_json::json;

#[test]
fn read_only_rejects_user_mutation_while_host_updates_remain_atomic() {
    let definition = authority_definition();
    let mut form = definition
        .create_form(json!({
            "profile": { "name": "Ada" },
            "secret": "existing secret",
            "credentials": { "token": "nested secret" }
        }))
        .expect("the annotated form data should create a form");
    let profile = node_with_binding(&form, "/profile");
    let name = node_with_binding(&form, "/profile/name");
    let secret = node_with_binding(&form, "/secret");

    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { .. }
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Validation(_)
            ))
    ));
    assert!(form.node(profile).unwrap().is_read_only());
    assert!(form.node(name).unwrap().is_read_only());
    assert_eq!(
        form.node(profile).unwrap().allowed_operations(),
        AllowedOperations::default()
    );
    assert_eq!(
        form.node(name).unwrap().allowed_operations(),
        AllowedOperations::default()
    );
    let before_data = form.view().data_revision();
    let before_state = form.view().state_revision();

    assert_eq!(
        form.user().input_text(name, "Grace"),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(
        form.user().remove_value(profile),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(
        form.user().blur(name),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(form.view().data_revision(), before_data);
    assert_eq!(form.view().state_revision(), before_state);
    assert_eq!(
        form.form_data(),
        &json!({
            "profile": { "name": "Ada" },
            "secret": "existing secret",
            "credentials": { "token": "nested secret" }
        })
    );

    let transition = form
        .transact(|draft| {
            draft.set(&pointer("/profile/name"), json!("Grace"));
            draft.set(&pointer("/secret"), json!("host replacement"));
        })
        .expect("the host should retain privileged mutation authority");

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
    assert!(changed.contains(&name));
    assert!(changed.contains(&secret));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(snapshot)
            if snapshot.form_data() == &json!({
                "profile": { "name": "Grace" },
                "secret": "host replacement",
                "credentials": { "token": "nested secret" }
            })
    ));
}

#[test]
fn write_only_data_can_be_explicitly_replaced_and_is_preserved_for_submission() {
    let definition = authority_definition();
    let secret_definition = definition
        .node_with_binding("/secret")
        .expect("the write-only definition should exist");
    assert!(secret_definition.data_schema_annotations().is_write_only());
    let mut form = definition
        .create_form(json!({
            "profile": { "name": "Alice" },
            "secret": "existing secret",
            "credentials": { "token": "nested secret" }
        }))
        .expect("the annotated form data should create a form");
    let secret = node_with_binding(&form, "/secret");
    let nested_secret = node_with_binding(&form, "/credentials/token");

    assert!(form.node(secret).unwrap().is_write_only());
    assert!(form.node(nested_secret).unwrap().is_write_only());
    assert!(
        form.node(secret)
            .unwrap()
            .allowed_operations()
            .can_replace_value()
    );
    let transition = form
        .user()
        .replace_value(secret, json!("user replacement"))
        .expect("a write-only text control should accept an explicit replacement");
    assert_ne!(
        transition.before_data_revision(),
        transition.after_data_revision()
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(snapshot)
            if snapshot.form_data() == &json!({
                "profile": { "name": "Alice" },
                "secret": "user replacement",
                "credentials": { "token": "nested secret" }
            })
    ));
}

#[test]
fn root_read_only_annotation_restricts_every_user_control_but_not_the_host() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "readOnly": true,
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string" }
        }
    }))
    .expect("a root read-only annotation should compile");
    let mut form = definition
        .create_form(json!({ "name": "Ada" }))
        .expect("the root read-only form data should create a form");
    let name = node_with_binding(&form, "/name");

    assert!(form.node(name).unwrap().is_read_only());
    assert_eq!(
        form.user().input_text(name, "Grace"),
        Err(UserOperationError::OperationNotAllowed)
    );
    form.transact(|draft| draft.set(&pointer("/name"), json!("Grace")))
        .expect("the host should replace data below a root read-only annotation");
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(snapshot)
            if snapshot.form_data() == &json!({ "name": "Grace" })
    ));
}

fn authority_definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["profile", "secret", "credentials"],
        "properties": {
            "profile": {
                "type": "object",
                "readOnly": true,
                "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "minLength": 5 }
                }
            },
            "secret": {
                "type": "string",
                "title": "Secret",
                "writeOnly": true
            },
            "credentials": {
                "type": "object",
                "writeOnly": true,
                "additionalProperties": false,
                "required": ["token"],
                "properties": {
                    "token": { "type": "string" }
                }
            }
        }
    }))
    .expect("read-only and write-only annotations should compile")
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

trait DefinitionWithBinding {
    fn node_with_binding(
        &self,
        binding: &str,
    ) -> Option<schemaform::definition::DefinitionNodeView<'_>>;
}

impl DefinitionWithBinding for FormDefinition {
    fn node_with_binding(
        &self,
        binding: &str,
    ) -> Option<schemaform::definition::DefinitionNodeView<'_>> {
        let root = self.node(self.root())?;
        let mut pending = root.children().collect::<Vec<_>>();
        while let Some(identity) = pending.pop() {
            let node = self.node(identity)?;
            if node
                .binding()
                .is_some_and(|current| current.as_str() == binding)
            {
                return Some(node);
            }
            pending.extend(node.children());
        }
        None
    }
}
