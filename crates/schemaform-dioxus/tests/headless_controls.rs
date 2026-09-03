//! Native contract tests for the headless text edit hook.
//!
//! Each test configures a capturing renderer through `RenderConfigurationBuilder`, mounts
//! `SchemaForm` in a native `VirtualDom`, and drives the `TextEdit` the renderer's child component
//! obtained from `use_text_edit` outside the VirtualDom. Observations go through the form handle,
//! the captured render context, and the host's `on_error` callback only.

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
    ControlMatcher, ControlRegistry, ControlRenderContext, ControlRenderer, FormHandle,
    HandleError, RenderConfiguration, SchemaForm, TextEdit, use_form, use_text_edit,
};
use serde_json::json;

/// What the capturing child component saw on its latest render.
#[derive(Clone)]
struct Captured {
    context: ControlRenderContext,
    edit: TextEdit,
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

/// The renderer's own child component: the only place a hook may be called.
#[allow(non_snake_case)]
fn CapturingControl(props: CapturingControlProps) -> Element {
    let edit = use_text_edit(&props.context);
    props.edits.borrow_mut().insert(
        props.context.control().name.clone(),
        Captured {
            context: props.context.clone(),
            edit,
        },
    );
    rsx! {}
}

struct CapturingRenderer {
    edits: CapturedEdits,
}

impl ControlRenderer for CapturingRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let edits = self.edits.clone();
        rsx! {
            CapturingControl { context, edits }
        }
    }
}

struct TextControls;

impl ControlMatcher for TextControls {
    fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
        matches!(
            definition.semantic_kind(),
            Some(SemanticKind::String | SemanticKind::Number | SemanticKind::Integer)
        )
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
    json!({ "quantity": 1, "name": "Ada", "secret": "hunter2", "note": "fixed" })
}

fn headless_app(props: HeadlessAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["quantity", "name", "secret", "note"],
            "properties": {
                "quantity": { "type": "integer", "title": "Quantity", "minimum": 0 },
                "name": { "type": "string", "title": "Name", "minLength": 2 },
                "secret": { "type": "string", "title": "Secret", "writeOnly": true },
                "note": { "type": "string", "title": "Note", "readOnly": true }
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
                Arc::new(TextControls),
                Arc::new(CapturingRenderer { edits }),
            ))
            .build()
            .bind(&form)
            .expect("the capturing renderer should bind every text control")
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
        self.captured(name).edit
    }

    fn value(&self, name: &str) -> String {
        let edit = self.edit(name);
        self.dom
            .in_scope(ScopeId::ROOT, || edit.value.read().clone())
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
