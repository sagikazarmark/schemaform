//! Native contract tests for the form shell seam.
//!
//! Each test configures a capturing `ShellRenderer` through `RenderConfigurationBuilder`, mounts
//! `SchemaForm` in a native `VirtualDom`, and drives the submit affordance the shell received
//! outside the VirtualDom. Observations go through the form handle, the captured shell context,
//! and the host's `on_submit` and `on_error` callbacks only.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use dioxus::prelude::{Element, Props, rsx, use_hook};
use dioxus_core::{NoOpMutations, ScopeId, VirtualDom};
use schemaform::{FormDefinition, SubmissionSnapshot};
use schemaform_dioxus::{
    Affordance, AffordanceKind, BoundForm, BuiltinShell, FormHandle, HandleError, Localizer,
    RenderConfiguration, SchemaForm, ShellContext, ShellRenderer, StructureRenderers,
    render::MessageDescriptor, use_form,
};
use serde_json::json;

/// The non-element parts of the latest `ShellContext` the capturing shell received.
#[derive(Clone)]
struct CapturedShell {
    form_id: String,
    submit: Affordance,
    summary_rendered: bool,
    body_rendered: bool,
}

type Capture = Rc<RefCell<Option<CapturedShell>>>;

struct CapturingShell {
    capture: Capture,
    calls: Rc<Cell<usize>>,
}

impl ShellRenderer for CapturingShell {
    fn shell(&self, context: ShellContext) -> Element {
        self.calls.set(self.calls.get() + 1);
        *self.capture.borrow_mut() = Some(CapturedShell {
            form_id: context.form_id.clone(),
            submit: context.submit.clone(),
            summary_rendered: context.summary.is_ok(),
            body_rendered: context.body.is_ok(),
        });
        let submit = context.submit;
        rsx! {
            div { class: "shell-summary", {context.summary} }
            div { class: "shell-body", {context.body} }
            button {
                id: submit.id.clone(),
                r#type: "button",
                onclick: move |_| submit.invoke.call(()),
                "{submit.label}"
            }
        }
    }
}

/// Localizes the built-in submit label by its stable key; everything else falls back.
struct ShellLocalizer;

impl Localizer for ShellLocalizer {
    fn localize(&self, message: &MessageDescriptor) -> String {
        if message.key.as_deref() == Some("schemaform.submit.label") {
            return "Send".to_owned();
        }
        message.fallback.clone()
    }
}

#[derive(Clone, Props)]
struct ShellAppProps {
    capture: Capture,
    calls: Rc<Cell<usize>>,
    handle: Rc<RefCell<Option<FormHandle>>>,
    bound: Rc<RefCell<Option<BoundForm>>>,
    submitted: Rc<RefCell<Option<SubmissionSnapshot>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl PartialEq for ShellAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.capture, &other.capture)
            && Rc::ptr_eq(&self.calls, &other.calls)
            && Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.bound, &other.bound)
            && Rc::ptr_eq(&self.submitted, &other.submitted)
            && Rc::ptr_eq(&self.errors, &other.errors)
    }
}

fn initial_form_data() -> serde_json::Value {
    json!({ "name": "Ada" })
}

fn shell_app(props: ShellAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "title": "Name", "minLength": 2 }
            }
        }))
        .expect("the shell data schema should compile")
    });
    let form = use_form(definition, initial_form_data()).expect("the shell form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let capture = props.capture.clone();
    let calls = props.calls.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(StructureRenderers::default().with_shell(CapturingShell { capture, calls }))
            .localizer(Arc::new(ShellLocalizer))
            .build()
            .bind(&form)
            .expect("the built-in control should bind under a custom shell")
    });
    props
        .bound
        .borrow_mut()
        .get_or_insert_with(|| bound.clone());
    let submitted = props.submitted.clone();
    let errors = props.errors.clone();
    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *submitted.borrow_mut() = Some(snapshot),
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

struct MountedShell {
    dom: VirtualDom,
    capture: Capture,
    calls: Rc<Cell<usize>>,
    handle: FormHandle,
    bound: BoundForm,
    submitted: Rc<RefCell<Option<SubmissionSnapshot>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl MountedShell {
    fn mount() -> Self {
        let capture: Capture = Rc::default();
        let calls = Rc::new(Cell::new(0));
        let handle = Rc::new(RefCell::new(None));
        let bound = Rc::new(RefCell::new(None));
        let submitted = Rc::new(RefCell::new(None));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(
            shell_app,
            ShellAppProps {
                capture: capture.clone(),
                calls: calls.clone(),
                handle: handle.clone(),
                bound: bound.clone(),
                submitted: submitted.clone(),
                errors: errors.clone(),
            },
        );
        dom.rebuild_in_place();
        let handle = handle
            .borrow()
            .clone()
            .expect("the shell app should expose its form handle");
        let bound = bound
            .borrow()
            .clone()
            .expect("the shell app should expose its bound form");
        Self {
            dom,
            capture,
            calls,
            handle,
            bound,
            submitted,
            errors,
        }
    }

    fn captured(&self) -> CapturedShell {
        self.capture
            .borrow()
            .clone()
            .expect("the shell renderer should have been called")
    }

    /// Runs `callback` the way an event handler would, then settles the DOM.
    fn drive(&mut self, callback: impl FnOnce()) {
        self.dom.in_scope(ScopeId::ROOT, callback);
        self.settle();
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.dom.render_immediate(&mut NoOpMutations);
        }
    }
}

#[test]
fn the_shell_receives_the_form_id_the_regions_and_a_localized_submit_affordance() {
    let mounted = MountedShell::mount();
    let shell = mounted.captured();

    assert_eq!(mounted.calls.get(), 1);
    assert!(shell.form_id.starts_with("schemaform-"));
    assert!(shell.summary_rendered);
    assert!(shell.body_rendered);
    assert_eq!(shell.submit.kind, AffordanceKind::Submit);
    assert_eq!(shell.submit.label, "Send");
    assert_eq!(shell.submit.id, format!("{}-submit", shell.form_id));
    assert_eq!(shell.submit.accessible_name, None);
}

#[test]
fn a_ready_submit_through_the_shell_affordance_yields_a_submission_snapshot() {
    let mut mounted = MountedShell::mount();
    let submit = mounted.captured().submit;

    mounted.drive(|| submit.invoke.call(()));

    let snapshot = mounted
        .submitted
        .borrow()
        .clone()
        .expect("a ready submit should reach on_submit");
    assert_eq!(snapshot.form_data(), &initial_form_data());
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn a_blocked_submit_through_the_shell_affordance_yields_no_snapshot_and_records_the_attempt() {
    let mut mounted = MountedShell::mount();
    mounted
        .handle
        .reinitialize(json!({ "name": "A" }))
        .expect("reinitialization with a too-short name should be accepted");
    mounted.settle();
    let submit = mounted.captured().submit;

    mounted.drive(|| submit.invoke.call(()));

    assert!(mounted.submitted.borrow().is_none());
    assert!(mounted.errors.borrow().is_empty());
    let projection = mounted
        .handle
        .reader()
        .read()
        .expect("the form should be readable after a blocked submit");
    assert!(projection.submission_attempted);
    assert!(
        !projection.findings.is_empty(),
        "a blocked submit should present its findings"
    );
}

#[test]
fn a_held_form_borrow_during_submit_surfaces_borrow_conflict_through_on_error() {
    let mut mounted = MountedShell::mount();
    let submit = mounted.captured().submit;
    let handle = mounted.handle.clone();

    mounted.drive(|| {
        handle
            .try_transact(|_| {
                // The host holds the form borrow: submission cannot reach the core.
                submit.invoke.call(());
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });

    assert_eq!(*mounted.errors.borrow(), vec![HandleError::BorrowConflict]);
    assert!(mounted.submitted.borrow().is_none());
}

#[test]
fn the_shell_is_fixed_at_bind_and_a_later_presentation_rebind_does_not_replace_it() {
    let mut mounted = MountedShell::mount();
    assert_eq!(mounted.calls.get(), 1);

    // Rebinding presentation with a different localizer swaps the localizer signal (the submit
    // label follows) but leaves the structure renderers alone: the same capturing shell is
    // called again rather than the built-in taking over.
    let rebound = RenderConfiguration::builder()
        .structure(StructureRenderers::default().with_shell(BuiltinShell))
        .localizer(Arc::new(FallbackOnlyLocalizer))
        .build();
    let bound = mounted.bound.clone();
    mounted.drive(move || rebound.rebind_presentation(&bound));

    let shell = mounted.captured();
    assert_eq!(
        mounted.calls.get(),
        2,
        "the shell renderer chosen at bind is re-rendered"
    );
    assert_eq!(
        shell.submit.label, "Submit",
        "the localizer swap took effect"
    );
    assert_eq!(shell.submit.kind, AffordanceKind::Submit);
}

/// Localizes nothing: every message falls back, so the submit label reads `Submit`.
struct FallbackOnlyLocalizer;

impl Localizer for FallbackOnlyLocalizer {
    fn localize(&self, message: &MessageDescriptor) -> String {
        message.fallback.clone()
    }
}
