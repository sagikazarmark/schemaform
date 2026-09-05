// The registry takes `Arc<dyn ControlRenderer>`; the capturing renderer holds
// single-threaded Dioxus state, which is the supported browser-CSR shape.
#![allow(clippy::arc_with_non_send_sync)]

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use dioxus::core::{NoOpMutations, ScopeId, VirtualDom};
use dioxus::prelude::*;
use dioxus_field::{Binding, ChangeOrigin, FieldMetaValues};
use dioxus_primitives::checkbox::CheckboxState;
use schemaform::{
    ExternalFinding, ExternalFindingBatch, FormDefinition, JsonPointer,
    definition::{DefinitionNodeView, SemanticKind},
};
use schemaform_dioxus::{
    ChoiceEdit, ChoiceIdentity, ControlKind, ControlMatcher, ControlRegistry, ControlRenderContext,
    ControlRenderer, FormHandle, HandleError, RenderConfiguration, SchemaForm, TextEdit,
    use_boolean_edit, use_choice_edit, use_form, use_text_edit,
};
use serde_json::json;

use demo::components::schemaform_daisyui::{
    field_meta_values, use_boolean_binding, use_checkbox_binding, use_choice_binding,
    use_radio_binding, use_text_binding,
};

/// The hook result and the bindings the mapping built from it, per control kind.
#[derive(Clone)]
enum CapturedEdit {
    Text {
        edit: TextEdit,
        binding: Binding<String>,
    },
    Boolean {
        binding: Binding<Option<bool>>,
        checkbox: Binding<CheckboxState>,
    },
    Choice {
        edit: ChoiceEdit,
        binding: Binding<Option<ChoiceIdentity>>,
        radio: Binding<String>,
    },
}

/// What the capturing child component obtained from the mapping on its latest render.
#[derive(Clone)]
struct Captured {
    context: ControlRenderContext,
    edit: CapturedEdit,
    meta: FieldMetaValues,
}

type CapturedByName = Rc<RefCell<HashMap<String, Captured>>>;

#[derive(Clone, Props)]
struct CapturingControlProps {
    context: ControlRenderContext,
    captured: CapturedByName,
}

impl PartialEq for CapturingControlProps {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && Rc::ptr_eq(&self.captured, &other.captured)
    }
}

fn capture(props: &CapturingControlProps, edit: CapturedEdit) {
    let meta = field_meta_values(props.context.presentation(), props.context.control());
    props.captured.borrow_mut().insert(
        props.context.control().name.clone(),
        Captured {
            context: props.context.clone(),
            edit,
            meta,
        },
    );
}

/// The renderer's child components: the only hook-safe place to build a binding. One
/// component per control kind keeps every hook call unconditional.
#[allow(non_snake_case)]
fn CapturingTextControl(props: CapturingControlProps) -> Element {
    let edit = use_text_edit(&props.context);
    let binding = use_text_binding(edit);
    capture(&props, CapturedEdit::Text { edit, binding });
    rsx! {}
}

#[allow(non_snake_case)]
fn CapturingBooleanControl(props: CapturingControlProps) -> Element {
    let edit = use_boolean_edit(&props.context);
    let binding = use_boolean_binding(edit);
    let checkbox = use_checkbox_binding(edit);
    capture(&props, CapturedEdit::Boolean { binding, checkbox });
    rsx! {}
}

#[allow(non_snake_case)]
fn CapturingChoiceControl(props: CapturingControlProps) -> Element {
    let edit = use_choice_edit(&props.context);
    let binding = use_choice_binding(edit.clone());
    let radio = use_radio_binding(edit.clone());
    capture(
        &props,
        CapturedEdit::Choice {
            edit,
            binding,
            radio,
        },
    );
    rsx! {}
}

struct CapturingRenderer {
    captured: CapturedByName,
}

impl ControlRenderer for CapturingRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let captured = self.captured.clone();
        match context.control().kind {
            ControlKind::Boolean => rsx! {
                CapturingBooleanControl { context, captured }
            },
            ControlKind::Choice => rsx! {
                CapturingChoiceControl { context, captured }
            },
            _ => rsx! {
                CapturingTextControl { context, captured }
            },
        }
    }
}

struct MappedControls;

impl ControlMatcher for MappedControls {
    fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
        matches!(
            definition.semantic_kind(),
            Some(
                SemanticKind::String
                    | SemanticKind::Number
                    | SemanticKind::Integer
                    | SemanticKind::Boolean
                    | SemanticKind::Choice
            )
        )
    }
}

#[derive(Clone, Props)]
struct MappingAppProps {
    captured: CapturedByName,
    handle: Rc<RefCell<Option<FormHandle>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl PartialEq for MappingAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.captured, &other.captured)
            && Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.errors, &other.errors)
    }
}

fn baseline_form_data() -> serde_json::Value {
    json!({
        "quantity": 1,
        "name": "Ada",
        "secret": "hunter2",
        "enabled": true,
        "flag": null,
        "secret_flag": true,
        "locked": false,
        "mode": "private",
        "secret_mode": "a"
    })
}

fn mapping_app(props: MappingAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["quantity", "name", "enabled", "secret_flag", "mode", "secret_mode"],
            "properties": {
                "quantity": { "type": "integer", "title": "Quantity", "minimum": 0 },
                "name": { "type": "string", "title": "Name", "minLength": 2 },
                "secret": { "type": "string", "title": "Secret", "writeOnly": true },
                "enabled": { "type": "boolean", "title": "Enabled" },
                "flag": { "type": ["boolean", "null"], "title": "Flag" },
                "secret_flag": {
                    "type": "boolean",
                    "title": "Secret flag",
                    "writeOnly": true
                },
                "locked": { "type": "boolean", "title": "Locked", "readOnly": true },
                "mode": {
                    "type": ["string", "null"],
                    "title": "Mode",
                    "enum": ["private", "public", null]
                },
                "secret_mode": {
                    "type": "string",
                    "title": "Secret mode",
                    "enum": ["a", "b"],
                    "writeOnly": true
                }
            }
        }))
        .expect("the mapping data schema should compile")
    });
    let form =
        use_form(definition, baseline_form_data()).expect("the mapping form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let captured = props.captured.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(ControlRegistry::with_builtins().matcher(
                10,
                Arc::new(MappedControls),
                Arc::new(CapturingRenderer { captured }),
            ))
            .build()
            .bind(&form)
            .expect("the capturing renderer should bind every mapped control")
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

struct MountedMapping {
    dom: VirtualDom,
    captured: CapturedByName,
    handle: FormHandle,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl MountedMapping {
    fn mount() -> Self {
        let captured: CapturedByName = Rc::default();
        let handle = Rc::new(RefCell::new(None));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(
            mapping_app,
            MappingAppProps {
                captured: captured.clone(),
                handle: handle.clone(),
                errors: errors.clone(),
            },
        );
        dom.rebuild_in_place();
        let handle = handle
            .borrow()
            .clone()
            .expect("the mapping app should expose its form handle");
        Self {
            dom,
            captured,
            handle,
            errors,
        }
    }

    /// Runs a widget-side call the way an event handler would, then settles the DOM.
    fn drive(&mut self, callback: impl FnOnce()) {
        self.dom.in_scope(ScopeId::ROOT, callback);
        self.settle();
    }

    /// Renders enough passes for a node change, the memo recompute it queues, the child
    /// render, and the lifecycle effect to settle; the fourth pass is headroom.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.dom.render_immediate(&mut NoOpMutations);
        }
    }

    fn captured(&self, name: &str) -> Captured {
        self.captured
            .borrow()
            .get(name)
            .unwrap_or_else(|| panic!("the child component should have rendered {name}"))
            .clone()
    }

    fn text(&self, name: &str) -> (TextEdit, Binding<String>) {
        match self.captured(name).edit {
            CapturedEdit::Text { edit, binding } => (edit, binding),
            _ => panic!("{name} should be a text control"),
        }
    }

    fn text_edit(&self, name: &str) -> TextEdit {
        self.text(name).0
    }

    fn text_binding(&self, name: &str) -> Binding<String> {
        self.text(name).1
    }

    fn boolean_binding(&self, name: &str) -> Binding<Option<bool>> {
        match self.captured(name).edit {
            CapturedEdit::Boolean { binding, .. } => binding,
            _ => panic!("{name} should be a boolean control"),
        }
    }

    fn checkbox_binding(&self, name: &str) -> Binding<CheckboxState> {
        match self.captured(name).edit {
            CapturedEdit::Boolean { checkbox, .. } => checkbox,
            _ => panic!("{name} should be a boolean control"),
        }
    }

    fn choice_edit(&self, name: &str) -> ChoiceEdit {
        match self.captured(name).edit {
            CapturedEdit::Choice { edit, .. } => edit,
            _ => panic!("{name} should be a choice control"),
        }
    }

    fn choice_binding(&self, name: &str) -> Binding<Option<ChoiceIdentity>> {
        match self.captured(name).edit {
            CapturedEdit::Choice { binding, .. } => binding,
            _ => panic!("{name} should be a choice control"),
        }
    }

    fn radio_binding(&self, name: &str) -> Binding<String> {
        match self.captured(name).edit {
            CapturedEdit::Choice { radio, .. } => radio,
            _ => panic!("{name} should be a choice control"),
        }
    }

    /// The identity of the option labelled `label`, or of the null option for `None`.
    fn option(&self, name: &str, label: Option<&str>) -> ChoiceIdentity {
        self.choice_edit(name)
            .options
            .iter()
            .find(|option| match label {
                Some(label) => option.label == label,
                None => option.is_null,
            })
            .unwrap_or_else(|| panic!("{name} should offer the {label:?} option"))
            .identity
            .clone()
    }

    /// What a binding currently reads, as a widget would read it.
    fn read<T: Clone + 'static>(&self, binding: &Binding<T>) -> T {
        self.dom.in_scope(ScopeId::ROOT, || (binding.read)())
    }

    /// The text the text binding at `name` currently reads.
    fn read_text(&self, name: &str) -> String {
        self.read(&self.text_binding(name))
    }

    fn form_data(&self) -> serde_json::Value {
        self.handle
            .reader()
            .form_data()
            .expect("the form should be readable")
    }

    fn revisions(&self) -> (schemaform::DataRevision, schemaform::StateRevision) {
        let form = self
            .handle
            .reader()
            .read()
            .expect("the form should be readable");
        (form.data_revision, form.state_revision)
    }

    fn apply_external_findings(&mut self, findings: Vec<ExternalFinding>) {
        let (revision, _) = self.revisions();
        self.handle
            .apply_external_findings(ExternalFindingBatch::new("server", revision, findings))
            .expect("the external batch should apply");
        self.settle();
    }
}

fn name_pointer() -> JsonPointer {
    JsonPointer::parse("/name").expect("the name pointer should parse")
}

#[test]
fn the_binding_reads_the_display_text_and_a_write_reaches_form_data() {
    let mut mounted = MountedMapping::mount();
    assert_eq!(mounted.read_text("/quantity"), "1");
    assert_eq!(mounted.read_text("/name"), "Ada");

    let binding = mounted.text_binding("/quantity");
    mounted.drive(|| binding.write("3".to_owned(), ChangeOrigin::User));

    assert_eq!(mounted.read_text("/quantity"), "3");
    assert_eq!(mounted.form_data()["quantity"], json!(3));
    assert!(mounted.errors.borrow().is_empty());
    assert!(
        mounted.captured("/quantity").meta.dirty,
        "a value that differs from the baseline is dirty"
    );
    assert!(
        !mounted.captured("/name").meta.dirty,
        "an untouched control is not"
    );
}

#[test]
fn a_control_the_core_accepts_no_selection_for_is_disabled() {
    let mounted = MountedMapping::mount();

    let locked = mounted.captured("/locked").meta;
    assert!(
        locked.disabled,
        "a read-only boolean offers no operation, so it is disabled: {locked:?}"
    );
    assert!(!mounted.captured("/enabled").meta.disabled);
    assert!(!mounted.captured("/mode").meta.disabled);
}

#[test]
fn a_write_the_core_cannot_parse_is_read_back_as_typed_without_changing_form_data() {
    let mut mounted = MountedMapping::mount();
    let binding = mounted.text_binding("/quantity");

    mounted.drive(|| binding.write("-".to_owned(), ChangeOrigin::User));

    assert_eq!(mounted.read_text("/quantity"), "-");
    assert_eq!(mounted.form_data(), baseline_form_data());
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn a_programmatic_write_is_applied_like_a_user_write() {
    let mut mounted = MountedMapping::mount();
    let binding = mounted.text_binding("/name");

    mounted.drive(|| binding.write("Grace".to_owned(), ChangeOrigin::Programmatic));

    assert_eq!(mounted.form_data()["name"], json!("Grace"));
}

#[test]
fn commit_changes_nothing() {
    let mut mounted = MountedMapping::mount();
    let before = mounted.revisions();
    let binding = mounted.text_binding("/quantity");

    mounted.drive(|| binding.commit());

    assert_eq!(mounted.revisions(), before);
    assert!(!mounted.captured("/quantity").meta.touched);
}

#[test]
fn focus_exit_marks_the_control_touched() {
    let mut mounted = MountedMapping::mount();
    assert!(!mounted.captured("/name").meta.touched);
    let binding = mounted.text_binding("/name");

    mounted.drive(|| binding.focus_exit());

    assert!(mounted.captured("/name").meta.touched);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn the_binding_keeps_its_identity_across_renders() {
    let mut mounted = MountedMapping::mount();
    let before = mounted.text_binding("/quantity");

    mounted.drive(|| before.write("7".to_owned(), ChangeOrigin::User));
    mounted.drive(|| before.focus_exit());

    let after = mounted.text_binding("/quantity");
    assert_eq!(after, before);
    assert_eq!(mounted.read_text("/quantity"), "7");
}

#[test]
fn a_write_only_value_is_not_echoed_through_the_binding() {
    let mounted = MountedMapping::mount();
    assert_eq!(mounted.read_text("/secret"), "");
    assert_eq!(mounted.form_data()["secret"], json!("hunter2"));
}

#[test]
fn a_write_while_the_form_is_borrowed_reaches_on_error() {
    let mut mounted = MountedMapping::mount();
    let binding = mounted.text_binding("/quantity");
    let handle = mounted.handle.clone();

    mounted.drive(|| {
        handle
            .try_transact(|_| {
                binding.write("5".to_owned(), ChangeOrigin::User);
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });

    assert_eq!(*mounted.errors.borrow(), vec![HandleError::BorrowConflict]);
    assert_eq!(mounted.form_data(), baseline_form_data());
}

#[test]
fn metadata_mirrors_the_control_and_reports_no_errors_while_valid() {
    let mounted = MountedMapping::mount();
    let captured = mounted.captured("/quantity");

    assert_eq!(
        captured.meta.id.as_deref(),
        Some(captured.context.presentation().element_id.as_str())
    );
    assert_eq!(captured.meta.name.as_deref(), Some("/quantity"));
    assert!(captured.meta.required);
    assert!(!captured.meta.disabled);
    assert!(!captured.meta.touched);
    assert!(!captured.meta.dirty);
    assert_eq!(captured.meta.invalid, Some(false));
    assert!(captured.meta.errors.is_empty());
}

#[test]
fn a_parse_blocker_is_an_error_and_makes_the_field_invalid() {
    let mut mounted = MountedMapping::mount();
    let edit = mounted.text_edit("/quantity");

    mounted.drive(|| edit.input.call("-".to_owned()));

    let meta = mounted.captured("/quantity").meta;
    assert_eq!(meta.invalid, Some(true));
    assert_eq!(meta.errors, vec![Rc::from("Enter a valid integer.")]);
}

#[test]
fn a_validation_finding_is_an_error_and_makes_the_field_invalid() {
    let mut mounted = MountedMapping::mount();
    let edit = mounted.text_edit("/name");

    // Validation findings become visible once the control is touched.
    mounted.drive(|| edit.input.call("A".to_owned()));
    mounted.drive(|| edit.blur.call(()));

    let meta = mounted.captured("/name").meta;
    assert_eq!(meta.invalid, Some(true));
    assert_eq!(
        meta.errors,
        vec![Rc::from("Value does not satisfy minLength.")]
    );
}

#[test]
fn only_blocking_external_findings_are_errors() {
    let mut mounted = MountedMapping::mount();
    let edit = mounted.text_edit("/name");
    mounted.drive(|| edit.blur.call(()));

    mounted.apply_external_findings(vec![
        ExternalFinding::blocking("review-name", name_pointer(), json!({})),
        ExternalFinding::advisory("suggest-name", name_pointer(), json!({})),
    ]);

    let captured = mounted.captured("/name");
    assert_eq!(captured.context.presentation().findings.len(), 2);
    assert_eq!(captured.meta.invalid, Some(true));
    assert_eq!(
        captured.meta.errors,
        vec![Rc::from("server reported review-name.")]
    );
}

#[test]
fn an_advisory_finding_alone_leaves_the_field_valid_with_no_errors() {
    let mut mounted = MountedMapping::mount();
    let edit = mounted.text_edit("/name");
    mounted.drive(|| edit.blur.call(()));

    mounted.apply_external_findings(vec![ExternalFinding::advisory(
        "suggest-name",
        name_pointer(),
        json!({}),
    )]);

    let captured = mounted.captured("/name");
    assert_eq!(captured.context.presentation().findings.len(), 1);
    assert_eq!(captured.meta.invalid, Some(false));
    assert!(captured.meta.errors.is_empty());
}

#[test]
fn the_checkbox_binding_reads_null_as_indeterminate_and_booleans_as_their_state() {
    let mounted = MountedMapping::mount();

    assert_eq!(
        mounted.read(&mounted.checkbox_binding("/flag")),
        CheckboxState::Indeterminate
    );
    assert_eq!(
        mounted.read(&mounted.checkbox_binding("/enabled")),
        CheckboxState::Checked
    );
}

#[test]
fn writing_a_checkbox_state_sets_the_boolean_and_indeterminate_sets_null() {
    let mut mounted = MountedMapping::mount();
    let flag = mounted.checkbox_binding("/flag");

    mounted.drive(|| flag.write(CheckboxState::Checked, ChangeOrigin::User));
    assert_eq!(mounted.form_data()["flag"], json!(true));
    assert_eq!(mounted.read(&flag), CheckboxState::Checked);

    mounted.drive(|| flag.write(CheckboxState::Unchecked, ChangeOrigin::User));
    assert_eq!(mounted.form_data()["flag"], json!(false));

    mounted.drive(|| flag.write(CheckboxState::Indeterminate, ChangeOrigin::User));
    assert_eq!(mounted.form_data()["flag"], json!(null));
    assert_eq!(mounted.read(&flag), CheckboxState::Indeterminate);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn the_boolean_binding_reads_the_tri_state_and_writes_reach_form_data() {
    let mut mounted = MountedMapping::mount();
    assert_eq!(
        mounted.read(&mounted.boolean_binding("/enabled")),
        Some(true)
    );
    assert_eq!(mounted.read(&mounted.boolean_binding("/flag")), None);
    let enabled = mounted.boolean_binding("/enabled");

    mounted.drive(|| enabled.write(Some(false), ChangeOrigin::User));

    assert_eq!(mounted.form_data()["enabled"], json!(false));
    assert_eq!(mounted.read(&enabled), Some(false));
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn a_write_only_boolean_is_written_but_never_read_back() {
    let mut mounted = MountedMapping::mount();
    let secret = mounted.boolean_binding("/secret_flag");
    assert_eq!(mounted.read(&secret), None);

    mounted.drive(|| secret.write(Some(false), ChangeOrigin::User));

    assert_eq!(mounted.form_data()["secret_flag"], json!(false));
    assert_eq!(mounted.read(&secret), None);
    assert_eq!(
        mounted.read(&mounted.checkbox_binding("/secret_flag")),
        CheckboxState::Indeterminate
    );
}

#[test]
fn boolean_focus_exit_marks_the_control_touched_and_commit_changes_nothing() {
    let mut mounted = MountedMapping::mount();
    assert!(!mounted.captured("/enabled").meta.touched);
    let before = mounted.revisions();
    let binding = mounted.boolean_binding("/enabled");

    mounted.drive(|| binding.commit());
    assert_eq!(mounted.revisions(), before);

    mounted.drive(|| binding.focus_exit());
    assert!(mounted.captured("/enabled").meta.touched);

    let checkbox = mounted.checkbox_binding("/flag");
    mounted.drive(|| checkbox.focus_exit());
    assert!(mounted.captured("/flag").meta.touched);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn boolean_bindings_keep_their_identity_across_renders() {
    let mut mounted = MountedMapping::mount();
    let binding = mounted.boolean_binding("/flag");
    let checkbox = mounted.checkbox_binding("/flag");

    mounted.drive(|| checkbox.write(CheckboxState::Checked, ChangeOrigin::User));
    mounted.drive(|| binding.focus_exit());

    assert_eq!(mounted.boolean_binding("/flag"), binding);
    assert_eq!(mounted.checkbox_binding("/flag"), checkbox);
    assert_eq!(mounted.read(&binding), Some(true));
}

#[test]
fn the_choice_binding_reads_the_selected_identity_and_a_write_selects_an_option() {
    let mut mounted = MountedMapping::mount();
    let private = mounted.option("/mode", Some("private"));
    let public = mounted.option("/mode", Some("public"));
    let binding = mounted.choice_binding("/mode");
    assert_eq!(mounted.read(&binding), Some(private));

    mounted.drive(|| binding.write(Some(public.clone()), ChangeOrigin::User));

    assert_eq!(mounted.form_data()["mode"], json!("public"));
    assert_eq!(mounted.read(&binding), Some(public));
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn writing_the_null_option_through_the_choice_binding_sets_null() {
    let mut mounted = MountedMapping::mount();
    let null = mounted.option("/mode", None);
    let binding = mounted.choice_binding("/mode");

    mounted.drive(|| binding.write(Some(null.clone()), ChangeOrigin::User));

    assert_eq!(mounted.form_data()["mode"], json!(null));
    assert_eq!(mounted.read(&binding), Some(null));
}

#[test]
fn the_radio_binding_speaks_identity_strings_in_both_directions() {
    let mut mounted = MountedMapping::mount();
    let private = mounted.option("/mode", Some("private"));
    let null = mounted.option("/mode", None);
    let radio = mounted.radio_binding("/mode");
    assert_eq!(mounted.read(&radio), private.as_str());

    mounted.drive(|| radio.write(null.as_str().to_owned(), ChangeOrigin::User));
    assert_eq!(mounted.form_data()["mode"], json!(null));
    assert_eq!(mounted.read(&radio), null.as_str());

    let before = mounted.revisions();
    mounted.drive(|| radio.write("not-an-option".to_owned(), ChangeOrigin::User));
    assert_eq!(mounted.revisions(), before);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn a_write_only_choice_is_written_but_never_read_back() {
    let mut mounted = MountedMapping::mount();
    let b = mounted.option("/secret_mode", Some("b"));
    let binding = mounted.choice_binding("/secret_mode");
    assert_eq!(mounted.read(&binding), None);
    assert_eq!(mounted.read(&mounted.radio_binding("/secret_mode")), "");

    mounted.drive(|| binding.write(Some(b), ChangeOrigin::User));

    assert_eq!(mounted.form_data()["secret_mode"], json!("b"));
    assert_eq!(mounted.read(&binding), None);
}

#[test]
fn choice_focus_exit_marks_the_control_touched_and_commit_changes_nothing() {
    let mut mounted = MountedMapping::mount();
    let before = mounted.revisions();
    let binding = mounted.choice_binding("/mode");

    mounted.drive(|| binding.commit());
    assert_eq!(mounted.revisions(), before);
    assert!(!mounted.captured("/mode").meta.touched);

    mounted.drive(|| binding.focus_exit());
    assert!(mounted.captured("/mode").meta.touched);

    let radio = mounted.radio_binding("/secret_mode");
    mounted.drive(|| radio.focus_exit());
    assert!(mounted.captured("/secret_mode").meta.touched);
}

#[test]
fn choice_bindings_keep_their_identity_across_renders() {
    let mut mounted = MountedMapping::mount();
    let public = mounted.option("/mode", Some("public"));
    let binding = mounted.choice_binding("/mode");
    let radio = mounted.radio_binding("/mode");

    mounted.drive(|| binding.write(Some(public.clone()), ChangeOrigin::User));
    mounted.drive(|| radio.focus_exit());

    assert_eq!(mounted.choice_binding("/mode"), binding);
    assert_eq!(mounted.radio_binding("/mode"), radio);
    assert_eq!(mounted.read(&radio), public.as_str());
}
