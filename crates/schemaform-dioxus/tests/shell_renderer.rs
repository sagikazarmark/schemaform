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

use dioxus::prelude::{Element, Props, Signal, WritableExt, rsx, use_hook, use_signal};
use dioxus_core::{NoOpMutations, ScopeId, VirtualDom};
use schemaform::{AdvisorySubmission, FormDefinition, SubmissionSnapshot, form::SubmissionBlocker};
use schemaform_dioxus::{
    Affordance, AffordanceKind, BoundForm, BuiltinShell, FormHandle, HandleError, Localizer,
    RenderConfiguration, SchemaForm, ShellContext, ShellRenderer, StructureRenderers,
    SubmissionMode, render::MessageDescriptor, use_form,
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
                onclick: move |_| submit.invoke(),
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
    advisory_submissions: Rc<RefCell<Vec<AdvisorySubmission>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
    /// The submission mode the app mounts in; the app exposes the signal behind it through
    /// `mode` so a test can switch the prop on the mounted form.
    initial_mode: SubmissionMode,
    mode: Rc<RefCell<Option<Signal<SubmissionMode>>>>,
}

impl PartialEq for ShellAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.capture, &other.capture)
            && Rc::ptr_eq(&self.calls, &other.calls)
            && Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.bound, &other.bound)
            && Rc::ptr_eq(&self.submitted, &other.submitted)
            && Rc::ptr_eq(&self.advisory_submissions, &other.advisory_submissions)
            && Rc::ptr_eq(&self.errors, &other.errors)
            && self.initial_mode == other.initial_mode
            && Rc::ptr_eq(&self.mode, &other.mode)
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
    let mode = use_signal(|| props.initial_mode);
    props.mode.borrow_mut().get_or_insert(mode);
    let submitted = props.submitted.clone();
    let advisory_submissions = props.advisory_submissions.clone();
    let errors = props.errors.clone();
    rsx! {
        SchemaForm {
            form: bound,
            submission_mode: mode(),
            on_submit: move |snapshot| *submitted.borrow_mut() = Some(snapshot),
            on_advisory_submit: move |submission| advisory_submissions.borrow_mut().push(submission),
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
    advisory_submissions: Rc<RefCell<Vec<AdvisorySubmission>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
    mode: Signal<SubmissionMode>,
}

impl MountedShell {
    fn mount() -> Self {
        Self::mount_in(SubmissionMode::default())
    }

    fn mount_in(initial_mode: SubmissionMode) -> Self {
        let capture: Capture = Rc::default();
        let calls = Rc::new(Cell::new(0));
        let handle = Rc::new(RefCell::new(None));
        let bound = Rc::new(RefCell::new(None));
        let submitted = Rc::new(RefCell::new(None));
        let advisory_submissions = Rc::new(RefCell::new(Vec::new()));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let mode = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new_with_props(
            shell_app,
            ShellAppProps {
                capture: capture.clone(),
                calls: calls.clone(),
                handle: handle.clone(),
                bound: bound.clone(),
                submitted: submitted.clone(),
                advisory_submissions: advisory_submissions.clone(),
                errors: errors.clone(),
                initial_mode,
                mode: mode.clone(),
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
        let mode = mode
            .borrow()
            .expect("the shell app should expose its submission mode signal");
        Self {
            dom,
            capture,
            calls,
            handle,
            bound,
            submitted,
            advisory_submissions,
            errors,
            mode,
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

    /// Switches the mounted form's `submission_mode` prop and settles the DOM.
    fn switch_mode(&mut self, mode: SubmissionMode) {
        let mut signal = self.mode;
        self.drive(move || signal.set(mode));
    }

    /// Reinitializes the form with a name too short for the data schema, so the next submission
    /// carries one `minLength` finding.
    fn make_invalid(&mut self) {
        self.handle
            .reinitialize(json!({ "name": "A" }))
            .expect("reinitialization with a too-short name should be accepted");
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

    mounted.drive(|| submit.invoke());

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
    mounted.make_invalid();
    let submit = mounted.captured().submit;

    mounted.drive(|| submit.invoke());

    assert!(mounted.submitted.borrow().is_none());
    assert!(
        mounted.advisory_submissions.borrow().is_empty(),
        "a gated form never hands out advisory submissions"
    );
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
fn an_advisory_form_hands_the_shell_an_advisory_submit_affordance() {
    let mounted = MountedShell::mount_in(SubmissionMode::Advisory);
    let shell = mounted.captured();

    assert_eq!(shell.submit.kind, AffordanceKind::AdvisorySubmit);
    assert_eq!(shell.submit.label, "Send");
    assert_eq!(shell.submit.id, format!("{}-submit", shell.form_id));
    assert_eq!(shell.submit.accessible_name, None);
}

#[test]
fn an_advisory_submit_with_findings_hands_the_host_the_data_and_the_findings_it_carried() {
    let mut mounted = MountedShell::mount_in(SubmissionMode::Advisory);
    mounted.make_invalid();
    let submit = mounted.captured().submit;

    mounted.drive(|| submit.invoke());

    let advisory_submissions = mounted.advisory_submissions.borrow();
    let submission = advisory_submissions
        .first()
        .expect("an advisory submit should reach on_advisory_submit");
    assert_eq!(advisory_submissions.len(), 1);
    assert_eq!(submission.form_data(), &json!({ "name": "A" }));
    let findings = submission.findings().collect::<Vec<_>>();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(matches!(
        findings[0],
        SubmissionBlocker::Validation(finding) if finding.code() == "minLength"
    ));
    assert!(
        mounted.submitted.borrow().is_none(),
        "an advisory submission never reaches on_submit"
    );
    assert!(mounted.errors.borrow().is_empty());
    let projection = mounted
        .handle
        .reader()
        .read()
        .expect("the form should be readable after an advisory submit");
    assert!(projection.submission_attempted);
    assert!(
        !projection.findings.is_empty(),
        "an advisory submit still presents the findings it sent past"
    );
}

#[test]
fn an_advisory_submit_of_a_valid_form_hands_the_host_the_data_and_no_findings() {
    let mut mounted = MountedShell::mount_in(SubmissionMode::Advisory);
    let submit = mounted.captured().submit;

    mounted.drive(|| submit.invoke());

    let advisory_submissions = mounted.advisory_submissions.borrow();
    assert_eq!(advisory_submissions.len(), 1);
    assert_eq!(advisory_submissions[0].form_data(), &initial_form_data());
    assert_eq!(advisory_submissions[0].findings().count(), 0);
    assert!(mounted.submitted.borrow().is_none());
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn a_held_form_borrow_during_an_advisory_submit_surfaces_borrow_conflict_through_on_error() {
    let mut mounted = MountedShell::mount_in(SubmissionMode::Advisory);
    let submit = mounted.captured().submit;
    let handle = mounted.handle.clone();

    mounted.drive(|| {
        handle
            .try_transact(|_| {
                submit.invoke();
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });

    assert_eq!(*mounted.errors.borrow(), vec![HandleError::BorrowConflict]);
    assert!(mounted.advisory_submissions.borrow().is_empty());
    assert!(mounted.submitted.borrow().is_none());
}

#[test]
fn switching_the_submission_mode_prop_rewires_the_affordance_and_the_callback() {
    let mut mounted = MountedShell::mount();
    mounted.make_invalid();
    assert_eq!(mounted.captured().submit.kind, AffordanceKind::Submit);

    // Gated → advisory: the shell is handed the advisory affordance and the very next submit
    // takes the advisory path.
    mounted.switch_mode(SubmissionMode::Advisory);
    let advisory_submit = mounted.captured().submit;
    assert_eq!(advisory_submit.kind, AffordanceKind::AdvisorySubmit);
    mounted.drive(|| advisory_submit.invoke());
    assert_eq!(mounted.advisory_submissions.borrow().len(), 1);
    assert!(mounted.submitted.borrow().is_none());

    // Advisory → gated: the affordance reverts, the same findings now block, and the advisory
    // channel stays quiet.
    mounted.switch_mode(SubmissionMode::Gated);
    let submit = mounted.captured().submit;
    assert_eq!(submit.kind, AffordanceKind::Submit);
    mounted.drive(|| submit.invoke());
    assert_eq!(mounted.advisory_submissions.borrow().len(), 1);
    assert!(mounted.submitted.borrow().is_none());

    // An affordance a shell retained from the advisory render belongs to the same live form
    // scope, so it is not stale, and its behaviour is fixed by its kind as every affordance's
    // is: it still performs the advisory submission it was handed out as, whatever the form's
    // current mode.
    mounted.drive(|| advisory_submit.invoke());
    assert_eq!(mounted.advisory_submissions.borrow().len(), 2);
    assert!(mounted.submitted.borrow().is_none());
    assert!(mounted.errors.borrow().is_empty());
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
                submit.invoke();
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
