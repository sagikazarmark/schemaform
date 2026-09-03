//! Native contract tests for the headless edit hooks.
//!
//! Each test configures a capturing renderer through `RenderConfigurationBuilder`, mounts
//! `SchemaForm` in a native `VirtualDom`, and drives the `TextEdit`, `BooleanEdit`, or
//! `ChoiceEdit` the renderer's child component obtained from its hook outside the VirtualDom.
//! Observations go through the form handle, the captured render context, and the host's
//! `on_error` callback only.

// The registry takes `Arc<dyn ControlRenderer>`; the capturing renderer holds single-threaded
// Dioxus state, which is the supported browser-CSR shape.
#![allow(clippy::arc_with_non_send_sync)]

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use dioxus::prelude::{Element, Props, ReadableExt, rsx, use_hook};
use dioxus_core::{NoOpMutations, ScopeId, VirtualDom};
use schemaform::{
    FormDefinition,
    definition::{DefinitionNodeView, SemanticKind},
    form::ParseBlockerKind,
};
use schemaform_dioxus::{
    BooleanEdit, ChoiceEdit, ChoiceIdentity, ControlKind, ControlMatcher, ControlRegistry,
    ControlRenderContext, ControlRenderer, FormHandle, HandleError, Localizer, RenderConfiguration,
    SchemaForm, TextEdit, render::MessageDescriptor, use_boolean_edit, use_choice_edit, use_form,
    use_text_edit,
};
use serde_json::json;

/// The hook result a capturing child component obtained for its control kind.
#[derive(Clone)]
enum CapturedEdit {
    Text(TextEdit),
    Boolean(BooleanEdit),
    Choice(ChoiceEdit),
}

/// What the capturing child component saw on its latest render.
#[derive(Clone)]
struct Captured {
    context: ControlRenderContext,
    edit: CapturedEdit,
}

type CapturedEdits = Rc<RefCell<HashMap<String, Captured>>>;

#[derive(Clone, Props)]
struct CapturingControlProps {
    context: ControlRenderContext,
    edits: CapturedEdits,
}

impl PartialEq for CapturingControlProps {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && Rc::ptr_eq(&self.edits, &other.edits)
    }
}

fn capture(props: &CapturingControlProps, edit: CapturedEdit) {
    props.edits.borrow_mut().insert(
        props.context.control().name.clone(),
        Captured {
            context: props.context.clone(),
            edit,
        },
    );
}

/// The renderer's own child components: the only place a hook may be called. One component per
/// control kind keeps each hook call unconditional.
#[allow(non_snake_case)]
fn CapturingTextControl(props: CapturingControlProps) -> Element {
    let edit = use_text_edit(&props.context);
    capture(&props, CapturedEdit::Text(edit));
    rsx! {}
}

#[allow(non_snake_case)]
fn CapturingBooleanControl(props: CapturingControlProps) -> Element {
    let edit = use_boolean_edit(&props.context);
    capture(&props, CapturedEdit::Boolean(edit));
    rsx! {}
}

#[allow(non_snake_case)]
fn CapturingChoiceControl(props: CapturingControlProps) -> Element {
    let edit = use_choice_edit(&props.context);
    capture(&props, CapturedEdit::Choice(edit));
    rsx! {}
}

struct CapturingRenderer {
    edits: CapturedEdits,
}

impl ControlRenderer for CapturingRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let edits = self.edits.clone();
        match context.control().kind {
            ControlKind::Boolean => rsx! {
                CapturingBooleanControl { context, edits }
            },
            ControlKind::Choice => rsx! {
                CapturingChoiceControl { context, edits }
            },
            _ => rsx! {
                CapturingTextControl { context, edits }
            },
        }
    }
}

struct HookedControls;

impl ControlMatcher for HookedControls {
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

/// Localizes exactly one keyless authored label, proving choice labels pass through the
/// configured localizer; everything else falls back.
struct HeadlessLocalizer;

impl Localizer for HeadlessLocalizer {
    fn localize(&self, message: &MessageDescriptor) -> String {
        if message.key.is_none() && message.fallback == "public" {
            return "Public".to_owned();
        }
        message.fallback.clone()
    }
}

#[derive(Clone, Props)]
struct HeadlessAppProps {
    edits: CapturedEdits,
    handle: Rc<RefCell<Option<FormHandle>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl PartialEq for HeadlessAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.edits, &other.edits)
            && Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.errors, &other.errors)
    }
}

fn initial_form_data() -> serde_json::Value {
    json!({
        "quantity": 1,
        "name": "Ada",
        "secret": "hunter2",
        "note": "fixed",
        "enabled": false,
        "flag": true,
        "secret_flag": true,
        "mode": "private",
        "fixed_mode": "a",
        "secret_mode": "a"
    })
}

fn headless_app(props: HeadlessAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": [
                "quantity", "name", "secret", "note", "enabled", "secret_flag", "mode",
                "fixed_mode", "secret_mode"
            ],
            "properties": {
                "quantity": { "type": "integer", "title": "Quantity", "minimum": 0 },
                "name": { "type": "string", "title": "Name", "minLength": 2 },
                "secret": { "type": "string", "title": "Secret", "writeOnly": true },
                "note": { "type": "string", "title": "Note", "readOnly": true },
                "enabled": { "type": "boolean", "title": "Enabled" },
                "flag": { "type": ["boolean", "null"], "title": "Flag" },
                "secret_flag": { "type": "boolean", "title": "Secret flag", "writeOnly": true },
                "mode": {
                    "type": ["string", "null"],
                    "title": "Mode",
                    "enum": ["private", "public", null]
                },
                "fixed_mode": {
                    "type": "string",
                    "title": "Fixed mode",
                    "enum": ["a", "b"],
                    "readOnly": true
                },
                "secret_mode": {
                    "type": "string",
                    "title": "Secret mode",
                    "enum": ["a", "b"],
                    "writeOnly": true
                }
            }
        }))
        .expect("the headless data schema should compile")
    });
    let form =
        use_form(definition, initial_form_data()).expect("the headless form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let edits = props.edits.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(ControlRegistry::with_builtins().matcher(
                10,
                Arc::new(HookedControls),
                Arc::new(CapturingRenderer { edits }),
            ))
            .localizer(Arc::new(HeadlessLocalizer))
            .build()
            .bind(&form)
            .expect("the capturing renderer should bind every hooked control")
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

struct MountedHeadless {
    dom: VirtualDom,
    edits: CapturedEdits,
    handle: FormHandle,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl MountedHeadless {
    fn mount() -> Self {
        let edits: CapturedEdits = Rc::default();
        let handle = Rc::new(RefCell::new(None));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(
            headless_app,
            HeadlessAppProps {
                edits: edits.clone(),
                handle: handle.clone(),
                errors: errors.clone(),
            },
        );
        dom.rebuild_in_place();
        let handle = handle
            .borrow()
            .clone()
            .expect("the headless app should expose its form handle");
        Self {
            dom,
            edits,
            handle,
            errors,
        }
    }

    /// Runs the hook-returned callback the way an event handler would, then settles the DOM.
    fn drive(&mut self, callback: impl FnOnce()) {
        self.dom.in_scope(ScopeId::ROOT, callback);
        self.settle();
    }

    /// Renders enough passes for a change to propagate: each pass first runs pending tasks and
    /// effects, then re-renders dirty scopes, so a node change, the memo recompute it queues, the
    /// child render, and the lifecycle effect settle within three passes; the fourth is headroom.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.dom.render_immediate(&mut NoOpMutations);
        }
    }

    fn captured(&self, name: &str) -> Captured {
        self.edits
            .borrow()
            .get(name)
            .unwrap_or_else(|| panic!("the child component should have rendered {name}"))
            .clone()
    }

    fn edit(&self, name: &str) -> TextEdit {
        match self.captured(name).edit {
            CapturedEdit::Text(edit) => edit,
            _ => panic!("{name} should be a text control"),
        }
    }

    fn boolean_edit(&self, name: &str) -> BooleanEdit {
        match self.captured(name).edit {
            CapturedEdit::Boolean(edit) => edit,
            _ => panic!("{name} should be a boolean control"),
        }
    }

    fn choice_edit(&self, name: &str) -> ChoiceEdit {
        match self.captured(name).edit {
            CapturedEdit::Choice(edit) => edit,
            _ => panic!("{name} should be a choice control"),
        }
    }

    /// The identity of the option labelled `label` (or the null option for `None`).
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

    fn selected(&self, name: &str) -> Option<ChoiceIdentity> {
        let edit = self.choice_edit(name);
        self.dom
            .in_scope(ScopeId::ROOT, || edit.selected.read().clone())
    }

    fn value(&self, name: &str) -> String {
        let edit = self.edit(name);
        self.dom
            .in_scope(ScopeId::ROOT, || edit.value.read().clone())
    }

    fn checked(&self, name: &str) -> Option<bool> {
        let edit = self.boolean_edit(name);
        self.dom.in_scope(ScopeId::ROOT, || *edit.checked.read())
    }

    fn revisions(&self) -> (schemaform::DataRevision, schemaform::StateRevision) {
        let form = self
            .handle
            .reader()
            .read()
            .expect("the form should be readable");
        (form.data_revision, form.state_revision)
    }

    fn form_data(&self) -> serde_json::Value {
        self.handle
            .reader()
            .form_data()
            .expect("the form should be readable")
    }

    fn projection(&self, name: &str) -> schemaform_dioxus::NodeProjection {
        let identity = self.captured(name).context.node().identity();
        self.handle
            .node(identity)
            .expect("the form should be readable")
            .expect("the node should exist")
            .read()
            .expect("the node should be readable")
            .expect("the node should still be part of the form tree")
    }
}

#[test]
fn input_produces_an_edit_buffer_parse_blocker_and_invalid_presentation() {
    let mut mounted = MountedHeadless::mount();
    assert_eq!(mounted.value("/quantity"), "1");
    assert!(!mounted.captured("/quantity").context.presentation().invalid);

    let edit = mounted.edit("/quantity");
    mounted.drive(|| edit.input.call("-".to_owned()));

    assert_eq!(mounted.value("/quantity"), "-");
    let projection = mounted.projection("/quantity");
    assert_eq!(projection.edit_buffer.as_deref(), Some("-"));
    assert_eq!(
        projection.parse_blocker,
        Some(ParseBlockerKind::InvalidInteger)
    );
    assert!(mounted.captured("/quantity").context.presentation().invalid);
    assert_eq!(mounted.form_data(), initial_form_data());

    mounted.drive(|| edit.input.call("3".to_owned()));
    assert_eq!(mounted.value("/quantity"), "3");
    assert!(!mounted.captured("/quantity").context.presentation().invalid);
    assert_eq!(mounted.form_data()["quantity"], json!(3));
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn the_edit_handle_is_hook_stable_across_renders() {
    let mut mounted = MountedHeadless::mount();
    let before = mounted.edit("/quantity");

    mounted.drive(|| before.input.call("-".to_owned()));
    mounted.drive(|| before.blur.call(()));

    let after = mounted.edit("/quantity");
    assert_eq!(after, before);
}

#[test]
fn blur_marks_the_control_touched() {
    let mut mounted = MountedHeadless::mount();
    assert!(!mounted.captured("/name").context.control().touched);

    let edit = mounted.edit("/name");
    mounted.drive(|| edit.blur.call(()));

    assert!(mounted.captured("/name").context.control().touched);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn a_held_form_borrow_surfaces_borrow_conflict_through_on_error() {
    let mut mounted = MountedHeadless::mount();
    let edit = mounted.edit("/quantity");
    let handle = mounted.handle.clone();

    mounted.drive(|| {
        handle
            .try_transact(|_| {
                // The host holds the form borrow: the hook's input cannot reach the core.
                edit.input.call("5".to_owned());
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });

    assert_eq!(*mounted.errors.borrow(), vec![HandleError::BorrowConflict]);
    assert_eq!(mounted.form_data(), initial_form_data());
    assert_eq!(mounted.value("/quantity"), "1");
}

#[test]
fn composition_buffers_input_locally_until_it_ends() {
    let mut mounted = MountedHeadless::mount();
    let before = mounted
        .handle
        .reader()
        .read()
        .expect("the form should be readable");

    let edit = mounted.edit("/quantity");
    mounted.drive(|| edit.composition_start.call(()));
    mounted.drive(|| edit.input.call("-".to_owned()));

    assert_eq!(mounted.value("/quantity"), "-");
    let composing = mounted
        .handle
        .reader()
        .read()
        .expect("the form should be readable");
    assert_eq!(composing.data_revision, before.data_revision);
    assert_eq!(composing.state_revision, before.state_revision);
    assert_eq!(mounted.projection("/quantity").edit_buffer, None);
    assert_eq!(mounted.form_data(), initial_form_data());

    mounted.drive(|| edit.input.call("3".to_owned()));
    mounted.drive(|| edit.composition_end.call(()));

    assert_eq!(mounted.form_data()["quantity"], json!(3));
    assert_eq!(mounted.value("/quantity"), "3");
    let committed = mounted
        .handle
        .reader()
        .read()
        .expect("the form should be readable");
    assert_ne!(committed.data_revision, before.data_revision);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn reinitialize_discards_an_in_flight_composition() {
    let mut mounted = MountedHeadless::mount();
    let edit = mounted.edit("/quantity");
    mounted.drive(|| edit.composition_start.call(()));
    mounted.drive(|| edit.input.call("9".to_owned()));
    assert_eq!(mounted.value("/quantity"), "9");

    let mut reinitialized = initial_form_data();
    reinitialized["quantity"] = json!(7);
    mounted
        .handle
        .reinitialize(reinitialized.clone())
        .expect("reinitialization should establish a new lifecycle");
    mounted.settle();
    assert_eq!(mounted.value("/quantity"), "7");

    mounted.drive(|| edit.composition_end.call(()));

    assert_eq!(mounted.form_data(), reinitialized);
    assert_eq!(mounted.value("/quantity"), "7");
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn write_only_values_are_not_echoed_and_read_only_controls_are_read_only() {
    let mounted = MountedHeadless::mount();

    assert_eq!(mounted.value("/secret"), "");
    assert!(!mounted.edit("/secret").read_only);

    assert!(mounted.edit("/note").read_only);
    assert_eq!(mounted.value("/note"), "fixed");

    assert!(!mounted.edit("/quantity").read_only);
    assert!(!mounted.edit("/name").read_only);
}

#[test]
fn boolean_set_none_on_a_nullable_target_sets_null() {
    let mut mounted = MountedHeadless::mount();
    assert_eq!(mounted.checked("/flag"), Some(true));

    let edit = mounted.boolean_edit("/flag");
    mounted.drive(|| edit.set.call(None));

    assert_eq!(mounted.form_data()["flag"], json!(null));
    assert_eq!(mounted.checked("/flag"), None);
    assert!(mounted.captured("/flag").context.control().dirty);

    mounted.drive(|| edit.set.call(Some(false)));

    assert_eq!(mounted.form_data()["flag"], json!(false));
    assert_eq!(mounted.checked("/flag"), Some(false));
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn boolean_set_chooses_set_value_or_replace_value_at_event_time() {
    let mut mounted = MountedHeadless::mount();
    let edit = mounted.boolean_edit("/enabled");
    assert_eq!(mounted.checked("/enabled"), Some(false));

    // Compatible data: a plain set.
    mounted.drive(|| edit.set.call(Some(true)));
    assert_eq!(mounted.form_data()["enabled"], json!(true));
    assert_eq!(mounted.checked("/enabled"), Some(true));

    // Incompatible data installed by the host: the same callback must replace instead.
    let mut incompatible = initial_form_data();
    incompatible["enabled"] = json!("yes");
    mounted
        .handle
        .reinitialize(incompatible)
        .expect("reinitialization should accept incompatible boolean data");
    mounted.settle();
    assert_eq!(mounted.checked("/enabled"), None);

    mounted.drive(|| edit.set.call(Some(false)));
    assert_eq!(mounted.form_data()["enabled"], json!(false));
    assert_eq!(mounted.checked("/enabled"), Some(false));
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn boolean_blur_marks_the_control_touched_and_the_handle_is_hook_stable() {
    let mut mounted = MountedHeadless::mount();
    let before = mounted.boolean_edit("/enabled");
    assert!(!mounted.captured("/enabled").context.control().touched);

    mounted.drive(|| before.blur.call(()));
    mounted.drive(|| before.set.call(Some(true)));

    assert!(mounted.captured("/enabled").context.control().touched);
    assert_eq!(mounted.boolean_edit("/enabled"), before);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn a_rejected_boolean_write_surfaces_borrow_conflict_and_leaves_checked_unchanged() {
    let mut mounted = MountedHeadless::mount();
    let edit = mounted.boolean_edit("/enabled");
    let handle = mounted.handle.clone();

    mounted.drive(|| {
        handle
            .try_transact(|_| {
                edit.set.call(Some(true));
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });

    assert_eq!(*mounted.errors.borrow(), vec![HandleError::BorrowConflict]);
    assert_eq!(mounted.form_data(), initial_form_data());
    assert_eq!(mounted.checked("/enabled"), Some(false));
}

#[test]
fn write_only_boolean_values_are_not_echoed() {
    let mut mounted = MountedHeadless::mount();
    assert_eq!(mounted.checked("/secret_flag"), None);

    let edit = mounted.boolean_edit("/secret_flag");
    mounted.drive(|| edit.set.call(Some(false)));

    assert_eq!(mounted.form_data()["secret_flag"], json!(false));
    assert_eq!(mounted.checked("/secret_flag"), None);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn selecting_the_null_choice_option_sets_null() {
    let mut mounted = MountedHeadless::mount();
    let private = mounted.option("/mode", Some("private"));
    let null = mounted.option("/mode", None);
    assert_eq!(mounted.selected("/mode"), Some(private));

    let edit = mounted.choice_edit("/mode");
    mounted.drive(|| edit.select.call(Some(null.clone())));

    assert_eq!(mounted.form_data()["mode"], json!(null));
    assert_eq!(mounted.selected("/mode"), Some(null));

    let public = mounted.option("/mode", Some("Public"));
    mounted.drive(|| edit.select.call(Some(public.clone())));

    assert_eq!(mounted.form_data()["mode"], json!("public"));
    assert_eq!(mounted.selected("/mode"), Some(public));
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn choice_options_carry_localized_labels_the_null_flag_and_disabled_state() {
    let mounted = MountedHeadless::mount();

    let describe = |name: &str| {
        mounted
            .choice_edit(name)
            .options
            .iter()
            .map(|option| (option.label.clone(), option.is_null, option.disabled))
            .collect::<Vec<_>>()
    };
    // The core lists the null option first; "public" is the one label the localizer maps.
    assert_eq!(
        describe("/mode"),
        [
            ("null".to_owned(), true, false),
            ("private".to_owned(), false, false),
            ("Public".to_owned(), false, false),
        ]
    );
    // A read-only choice allows no operation, so every option but the current one is disabled.
    assert_eq!(
        describe("/fixed_mode"),
        [
            ("a".to_owned(), false, false),
            ("b".to_owned(), false, true),
        ]
    );
}

#[test]
fn reselecting_the_current_choice_option_and_selecting_nothing_change_no_state() {
    let mut mounted = MountedHeadless::mount();
    let private = mounted.option("/mode", Some("private"));
    let before = mounted.revisions();

    let edit = mounted.choice_edit("/mode");
    mounted.drive(|| edit.select.call(Some(private.clone())));
    mounted.drive(|| edit.select.call(None));

    assert_eq!(mounted.revisions(), before);
    assert_eq!(mounted.selected("/mode"), Some(private));
    assert_eq!(mounted.form_data(), initial_form_data());
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn choice_select_chooses_set_value_or_replace_value_at_event_time() {
    let mut mounted = MountedHeadless::mount();
    let edit = mounted.choice_edit("/mode");
    let public = mounted.option("/mode", Some("Public"));

    let mut incompatible = initial_form_data();
    incompatible["mode"] = json!(42);
    mounted
        .handle
        .reinitialize(incompatible)
        .expect("reinitialization should accept an incompatible choice value");
    mounted.settle();
    assert_eq!(mounted.selected("/mode"), None);

    mounted.drive(|| edit.select.call(Some(public.clone())));

    assert_eq!(mounted.form_data()["mode"], json!("public"));
    assert_eq!(mounted.selected("/mode"), Some(public));
    assert_eq!(mounted.choice_edit("/mode"), edit);
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn selecting_a_disabled_choice_option_is_rejected_by_the_core_and_reported() {
    let mut mounted = MountedHeadless::mount();
    let b = mounted.option("/fixed_mode", Some("b"));

    let edit = mounted.choice_edit("/fixed_mode");
    mounted.drive(|| edit.select.call(Some(b)));

    assert_eq!(mounted.form_data()["fixed_mode"], json!("a"));
    assert_eq!(mounted.errors.borrow().len(), 1);
}

#[test]
fn write_only_choice_selections_are_not_echoed() {
    let mut mounted = MountedHeadless::mount();
    assert_eq!(mounted.selected("/secret_mode"), None);
    let b = mounted.option("/secret_mode", Some("b"));

    let edit = mounted.choice_edit("/secret_mode");
    mounted.drive(|| edit.select.call(Some(b)));

    assert_eq!(mounted.form_data()["secret_mode"], json!("b"));
    assert_eq!(mounted.selected("/secret_mode"), None);
    assert!(mounted.errors.borrow().is_empty());
}
