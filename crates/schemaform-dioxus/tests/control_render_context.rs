//! Native contract tests for the control render context handed to custom renderers.
//!
//! Each test configures a capturing renderer through `RenderConfigurationBuilder`, mounts
//! `SchemaForm` in a native `VirtualDom`, and observes only the captured context and the form
//! handle.

// The registry takes `Arc<dyn ControlRenderer>`; the capturing renderer holds single-threaded
// Dioxus state, which is the supported browser-CSR shape.
#![allow(clippy::arc_with_non_send_sync)]

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use dioxus::prelude::{Element, Props, rsx, use_hook};
use dioxus_core::{ScopeId, VirtualDom};
use schemaform::{FormDefinition, definition::DefinitionNodeView};
use schemaform_dioxus::{
    AffordanceKind, ControlFacets, ControlKind, ControlMatcher, ControlRegistry,
    ControlRenderContext, ControlRenderer, FormHandle, HandleError, RenderConfiguration,
    SchemaForm, render::FindingKind, use_form,
};
use serde_json::json;

type CapturedContexts = Rc<RefCell<HashMap<String, ControlRenderContext>>>;

struct CapturingRenderer {
    contexts: CapturedContexts,
}

impl ControlRenderer for CapturingRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        self.contexts
            .borrow_mut()
            .insert(context.control().name.clone(), context);
        rsx! {}
    }
}

struct EveryControl;

impl ControlMatcher for EveryControl {
    fn matches(&self, _definition: DefinitionNodeView<'_>) -> bool {
        true
    }
}

#[derive(Clone, Props)]
struct ContractAppProps {
    contexts: CapturedContexts,
    handle: Rc<RefCell<Option<FormHandle>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl PartialEq for ContractAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.contexts, &other.contexts)
            && Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.errors, &other.errors)
    }
}

fn contract_app(props: ContractAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["name", "agree", "secret"],
            "properties": {
                "name": {
                    "type": "string",
                    "title": "Full name",
                    "description": "Enter a full name",
                    "minLength": 2
                },
                "agree": { "type": "boolean", "title": "Agree" },
                "secret": { "type": "string", "title": "Secret", "writeOnly": true },
                "level": { "type": ["integer", "null"], "title": "Level" }
            }
        }))
        .expect("the contract data schema should compile")
    });
    let form = use_form(
        definition,
        json!({ "name": "Ada", "agree": false, "secret": "hunter2", "level": null }),
    )
    .expect("the contract form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let contexts = props.contexts.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(ControlRegistry::with_builtins().matcher(
                10,
                Arc::new(EveryControl),
                Arc::new(CapturingRenderer { contexts }),
            ))
            .build()
            .bind(&form)
            .expect("the capturing renderer should bind every control")
    });
    let errors = props.errors.clone();
    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |_| {},
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

struct MountedContract {
    dom: VirtualDom,
    contexts: CapturedContexts,
    handle: FormHandle,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

fn mount_contract_app() -> MountedContract {
    let contexts: CapturedContexts = Rc::default();
    let handle = Rc::new(RefCell::new(None));
    let errors = Rc::new(RefCell::new(Vec::new()));
    let mut dom = VirtualDom::new_with_props(
        contract_app,
        ContractAppProps {
            contexts: contexts.clone(),
            handle: handle.clone(),
            errors: errors.clone(),
        },
    );
    dom.rebuild_in_place();
    let handle = handle
        .borrow()
        .clone()
        .expect("the contract app should expose its form handle");
    MountedContract {
        dom,
        contexts,
        handle,
        errors,
    }
}

fn captured(contexts: &CapturedContexts, name: &str) -> ControlRenderContext {
    contexts
        .borrow()
        .get(name)
        .unwrap_or_else(|| panic!("the renderer should have been called for {name}"))
        .clone()
}

fn facets(contexts: &CapturedContexts, name: &str) -> ControlFacets {
    captured(contexts, name).control().clone()
}

fn presence_kinds(contexts: &CapturedContexts, name: &str) -> Vec<AffordanceKind> {
    captured(contexts, name)
        .presentation()
        .presence
        .iter()
        .map(|affordance| affordance.kind)
        .collect()
}

fn invoke_presence(
    dom: &mut VirtualDom,
    contexts: &CapturedContexts,
    name: &str,
    kind: AffordanceKind,
) {
    let context = captured(contexts, name);
    let affordance = context
        .presentation()
        .presence
        .iter()
        .find(|affordance| affordance.kind == kind)
        .unwrap_or_else(|| panic!("{name} should offer the {kind:?} affordance right now"))
        .clone();
    dom.in_scope(ScopeId::ROOT, || affordance.invoke.call(()));
    dom.render_immediate(&mut dioxus_core::NoOpMutations);
}

fn form_data(handle: &FormHandle) -> serde_json::Value {
    handle
        .reader()
        .form_data()
        .expect("the form should be readable")
}

#[test]
fn presence_affordances_list_only_the_allowed_operations_and_perform_them_when_invoked() {
    let MountedContract {
        mut dom,
        contexts,
        handle,
        errors,
    } = mount_contract_app();

    // A required, non-nullable string with compatible data offers no presence operation.
    assert!(presence_kinds(&contexts, "/name").is_empty());
    // A required boolean with a compatible value neither needs setting nor allows removal.
    assert!(presence_kinds(&contexts, "/agree").is_empty());

    // An optional nullable integer that is currently null can be set to its seed or removed.
    let level = captured(&contexts, "/level");
    let element_id = level.presentation().element_id.clone();
    let presence = &level.presentation().presence;
    assert_eq!(
        presence
            .iter()
            .map(|affordance| (
                affordance.kind,
                affordance.label.as_str(),
                affordance.id.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                AffordanceKind::Set,
                "Set Level",
                format!("{element_id}-set-value").as_str()
            ),
            (
                AffordanceKind::RemoveValue,
                "Remove Level",
                format!("{element_id}-remove-value").as_str()
            ),
        ]
    );
    assert_eq!(presence, &presence.clone());

    invoke_presence(&mut dom, &contexts, "/level", AffordanceKind::Set);
    assert_eq!(
        form_data(&handle),
        json!({ "name": "Ada", "agree": false, "secret": "hunter2", "level": 0 })
    );
    assert_eq!(
        presence_kinds(&contexts, "/level"),
        vec![AffordanceKind::SetNull, AffordanceKind::RemoveValue]
    );

    invoke_presence(&mut dom, &contexts, "/level", AffordanceKind::SetNull);
    assert_eq!(
        form_data(&handle),
        json!({ "name": "Ada", "agree": false, "secret": "hunter2", "level": null })
    );

    invoke_presence(&mut dom, &contexts, "/level", AffordanceKind::RemoveValue);
    assert_eq!(
        form_data(&handle),
        json!({ "name": "Ada", "agree": false, "secret": "hunter2" })
    );
    // A missing nullable value can be set to its seed or explicitly to null, but not removed.
    assert_eq!(
        presence_kinds(&contexts, "/level"),
        vec![AffordanceKind::Set, AffordanceKind::SetNull]
    );

    handle
        .reinitialize(json!({ "name": "Ada", "agree": false, "secret": "hunter2", "level": "x" }))
        .expect("incompatible data should be preserved for explicit repair");
    dom.render_immediate(&mut dioxus_core::NoOpMutations);
    assert_eq!(
        presence_kinds(&contexts, "/level"),
        vec![
            AffordanceKind::SetNull,
            AffordanceKind::RemoveValue,
            AffordanceKind::Replace
        ]
    );
    invoke_presence(&mut dom, &contexts, "/level", AffordanceKind::Replace);
    assert_eq!(
        form_data(&handle),
        json!({ "name": "Ada", "agree": false, "secret": "hunter2", "level": 0 })
    );

    assert!(errors.borrow().is_empty());
}

#[test]
fn report_routes_failures_to_on_error_and_returns_the_success_value() {
    let MountedContract {
        dom,
        contexts,
        handle,
        errors,
    } = mount_contract_app();
    let level = captured(&contexts, "/level");
    let actions = level.actions().clone();

    dom.in_scope(ScopeId::ROOT, || {
        assert_eq!(level.report(Ok(7)), Some(7));
        assert!(errors.borrow().is_empty());

        let set = level
            .presentation()
            .presence
            .iter()
            .find(|affordance| affordance.kind == AffordanceKind::Set)
            .expect("the null level should offer set")
            .clone();
        handle
            .try_transact(|_| {
                // The host holds the form borrow: every node operation fails with a conflict.
                assert_eq!(level.report(actions.set_null()), None);
                set.invoke.call(());
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });

    assert_eq!(
        *errors.borrow(),
        vec![HandleError::BorrowConflict, HandleError::BorrowConflict]
    );
    assert_eq!(
        form_data(&handle),
        json!({ "name": "Ada", "agree": false, "secret": "hunter2", "level": null })
    );
}

#[test]
fn custom_renderer_receives_localized_node_presentation_and_control_facets() {
    let MountedContract {
        contexts, errors, ..
    } = mount_contract_app();

    let name = captured(&contexts, "/name");
    let presentation = name.presentation();
    assert!(!presentation.element_id.is_empty());
    assert_eq!(presentation.label, "Full name");
    assert!(presentation.label_visible);
    let help = presentation
        .help
        .as_ref()
        .expect("the description should become localized help");
    assert_eq!(help.id, format!("{}-help", presentation.element_id));
    assert_eq!(help.text, "Enter a full name");
    assert!(presentation.findings.is_empty());
    assert!(!presentation.invalid);
    assert_eq!(
        presentation.described_by().as_deref(),
        Some(help.id.as_str())
    );

    let control = name.control();
    assert_eq!(control.kind, ControlKind::String);
    assert_eq!(control.name, "/name");
    assert!(control.required);
    assert!(!control.disabled);
    assert!(!control.read_only);
    assert!(!control.write_only);
    assert!(!control.touched);
    assert!(!control.dirty);
    assert!(!control.nullable);
    assert_eq!(control.write_only_replacement, None);
    assert_eq!(control.write_only_status, None);
    assert_eq!(control.boolean_labels, None);

    let agree = facets(&contexts, "/agree");
    assert_eq!(agree.kind, ControlKind::Boolean);
    let labels = agree
        .boolean_labels
        .as_ref()
        .expect("boolean controls should carry localized value labels");
    assert_eq!(labels.false_label, "False");
    assert_eq!(labels.true_label, "True");
    assert_eq!(agree.write_only_replacement, None);

    let secret = facets(&contexts, "/secret");
    assert_eq!(secret.kind, ControlKind::String);
    assert!(secret.write_only);
    let replacement = secret
        .write_only_replacement
        .as_ref()
        .expect("an editable write-only control should carry replacement chrome");
    assert_eq!(replacement.label, "Replace Secret");
    assert_eq!(replacement.placeholder, "Choose replacement");
    assert_eq!(secret.write_only_status.as_deref(), Some("Value is set"));

    let level = facets(&contexts, "/level");
    assert_eq!(level.kind, ControlKind::Integer);
    assert!(level.nullable);
    assert!(!level.required);

    let level_presentation = captured(&contexts, "/level").presentation().clone();
    assert_eq!(level_presentation.help, None);
    assert_eq!(level_presentation.described_by(), None);

    assert!(errors.borrow().is_empty());
}

#[test]
fn presentation_reflects_visible_findings_and_the_context_compares_by_value() {
    let MountedContract {
        mut dom,
        contexts,
        handle,
        errors,
    } = mount_contract_app();
    let before = captured(&contexts, "/name");
    assert_eq!(before, before.clone());

    let actions = before.actions().clone();
    dom.in_scope(ScopeId::ROOT, || {
        actions
            .input_text("A")
            .expect("the short name should be accepted as an edit");
        actions
            .blur()
            .expect("blur should mark the control touched");
    });
    dom.render_immediate(&mut dioxus_core::NoOpMutations);

    let after = captured(&contexts, "/name");
    assert_ne!(
        after, before,
        "a context with new findings must not compare equal to the stale one"
    );
    assert_eq!(after.node(), before.node());
    assert_eq!(after.actions(), before.actions());

    let presentation = after.presentation();
    assert!(presentation.invalid);
    assert_eq!(presentation.findings.len(), 1);
    let finding = &presentation.findings[0];
    assert_eq!(finding.kind, FindingKind::Validation);
    assert_eq!(finding.code, "minLength");
    assert!(finding.blocking);
    assert!(finding.stable_id.starts_with(&presentation.element_id));
    let help_id = presentation
        .help
        .as_ref()
        .map(|help| help.id.clone())
        .expect("help should remain present");
    assert_eq!(
        presentation.described_by(),
        Some(format!("{help_id} {}", finding.stable_id))
    );

    let control = after.control();
    assert!(control.touched);
    assert!(control.dirty);
    assert_eq!(
        handle
            .reader()
            .form_data()
            .expect("the form should be readable"),
        json!({ "name": "A", "agree": false, "secret": "hunter2", "level": null })
    );
    assert!(errors.borrow().is_empty());
}
