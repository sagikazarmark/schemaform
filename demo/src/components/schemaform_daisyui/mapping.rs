//! The mapping between `schemaform-dioxus` edit hooks and presentation and the
//! `dioxus-field` convention the registry widgets speak.
//!
//! Everything correctness-critical about editing (IME composition, lifecycle
//! discard, DOM resynchronisation after a rejected write, error reporting)
//! stays in `schemaform-dioxus`. This module only translates shapes, and is
//! compiled against the demo's own `dioxus-field` so a version mismatch with the
//! copied registry widgets is a compile error rather than an unbound field.

use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_field::{Binding, ChangeOrigin, FieldMetaValues};
use schemaform_dioxus::{
    ControlFacets, NodePresentation, TextEdit,
    render::{FindingDescriptor, FindingKind},
};

/// The comparable identity of a binding built from one text control.
///
/// A `TextEdit`'s own equality also compares `read_only`, which is presentation state the core
/// may toggle while the control's handles stay the same. Only the handles decide whether two
/// bindings behave interchangeably, so only they make up the identity: a binding built from the
/// same control on a later render compares equal even after such a toggle.
#[derive(Clone, PartialEq)]
struct TextBindingIdentity {
    value: ReadSignal<String>,
    input: Callback<String>,
    blur: Callback<()>,
}

/// Adapts a [`TextEdit`] to the `dioxus-field` binding contract.
///
/// This is a hook: call it unconditionally in the renderer's child component, after
/// [`schemaform_dioxus::use_text_edit`]. Write applies the text through `TextEdit::input`
/// regardless of its change origin; Commit is a no-op because the core applies every keystroke
/// and has no separate interaction unit; Focus Exit marks the control touched through
/// `TextEdit::blur`.
///
/// The binding's identity is the edit's hook-stable handles, so bindings built from the same
/// control on different renders compare equal and the registry widgets that receive them
/// neither re-render per keystroke nor re-register focus.
pub fn use_text_binding(edit: TextEdit) -> Binding<String> {
    let write = use_callback(move |(text, _origin): (String, ChangeOrigin)| {
        edit.input.call(text);
    });
    let commit = use_callback(|()| {});
    let focus_exit = use_callback(move |()| edit.blur.call(()));
    let identity = TextBindingIdentity {
        value: edit.value,
        input: edit.input,
        blur: edit.blur,
    };
    Binding::new_with_identity(edit.value, write, commit, identity)
        .with_focus_exit_using_identity(focus_exit)
}

/// Field metadata for one control, from its node presentation and control facets.
///
/// `invalid` is the adapter's verdict rather than derived from `errors`, and `errors` holds
/// only the findings [`is_field_error`] accepts. The remaining findings are the renderer's to
/// present separately.
pub fn field_meta_values(
    presentation: &NodePresentation,
    control: &ControlFacets,
) -> FieldMetaValues {
    FieldMetaValues {
        id: Some(Rc::from(presentation.element_id.as_str())),
        name: Some(Rc::from(control.name.as_str())),
        required: control.required,
        disabled: control.disabled,
        invalid: Some(presentation.invalid),
        errors: presentation
            .findings
            .iter()
            .filter(|finding| is_field_error(finding))
            .map(|finding| Rc::from(finding.text.as_str()))
            .collect(),
        touched: control.touched,
        dirty: control.dirty,
    }
}

/// Whether a finding belongs in `FieldMetaValues::errors`: it blocks submission and it is about
/// the value at the control, that is a parse blocker, a validation finding, or a blocking
/// external finding.
///
/// The registry's `FieldError` presents errors only while the field is invalid, and these are
/// the findings whose text belongs in that region. Advisory external findings do not make the
/// node invalid. Capability findings describe what the form can present rather than the value
/// the user entered, so even a blocking one is presented as a description instead; the node is
/// still exposed as invalid through `FieldMetaValues::invalid`.
pub fn is_field_error(finding: &FindingDescriptor) -> bool {
    finding.blocking
        && matches!(
            finding.kind,
            FindingKind::Parse | FindingKind::Validation | FindingKind::External
        )
}

#[cfg(test)]
mod tests {
    // The registry takes `Arc<dyn ControlRenderer>`; the capturing renderer holds
    // single-threaded Dioxus state, which is the supported browser-CSR shape.
    #![allow(clippy::arc_with_non_send_sync)]

    use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

    use dioxus::core::{NoOpMutations, ScopeId, VirtualDom};
    use dioxus::prelude::*;
    use dioxus_field::{Binding, ChangeOrigin, FieldMetaValues};
    use schemaform::{
        ExternalFinding, ExternalFindingBatch, FormDefinition, JsonPointer,
        definition::{DefinitionNodeView, SemanticKind},
    };
    use schemaform_dioxus::{
        ControlMatcher, ControlRegistry, ControlRenderContext, ControlRenderer, FormHandle,
        HandleError, RenderConfiguration, SchemaForm, TextEdit, use_form, use_text_edit,
    };
    use serde_json::json;

    use super::{field_meta_values, use_text_binding};

    /// What the capturing child component obtained from the mapping on its latest render.
    #[derive(Clone)]
    struct Captured {
        context: ControlRenderContext,
        edit: TextEdit,
        binding: Binding<String>,
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

    /// The renderer's child component: the only hook-safe place to build the binding.
    #[allow(non_snake_case)]
    fn CapturingTextControl(props: CapturingControlProps) -> Element {
        let edit = use_text_edit(&props.context);
        let binding = use_text_binding(edit);
        let meta = field_meta_values(props.context.presentation(), props.context.control());
        props.captured.borrow_mut().insert(
            props.context.control().name.clone(),
            Captured {
                context: props.context.clone(),
                edit,
                binding,
                meta,
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
            rsx! {
                CapturingTextControl { context, captured }
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
        json!({ "quantity": 1, "name": "Ada", "secret": "hunter2" })
    }

    fn mapping_app(props: MappingAppProps) -> Element {
        let definition = use_hook(|| {
            FormDefinition::compile(json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "required": ["quantity", "name"],
                "properties": {
                    "quantity": { "type": "integer", "title": "Quantity", "minimum": 0 },
                    "name": { "type": "string", "title": "Name", "minLength": 2 },
                    "secret": { "type": "string", "title": "Secret", "writeOnly": true }
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
                    Arc::new(TextControls),
                    Arc::new(CapturingRenderer { captured }),
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

        /// The text the binding currently reads, as a widget would read it.
        fn read(&self, name: &str) -> String {
            let binding = self.captured(name).binding;
            self.dom.in_scope(ScopeId::ROOT, || (binding.read)())
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
        assert_eq!(mounted.read("/quantity"), "1");
        assert_eq!(mounted.read("/name"), "Ada");

        let binding = mounted.captured("/quantity").binding;
        mounted.drive(|| binding.write("3".to_owned(), ChangeOrigin::User));

        assert_eq!(mounted.read("/quantity"), "3");
        assert_eq!(mounted.form_data()["quantity"], json!(3));
        assert!(mounted.errors.borrow().is_empty());
    }

    #[test]
    fn a_write_the_core_cannot_parse_is_read_back_as_typed_without_changing_form_data() {
        let mut mounted = MountedMapping::mount();
        let binding = mounted.captured("/quantity").binding;

        mounted.drive(|| binding.write("-".to_owned(), ChangeOrigin::User));

        assert_eq!(mounted.read("/quantity"), "-");
        assert_eq!(mounted.form_data(), baseline_form_data());
        assert!(mounted.errors.borrow().is_empty());
    }

    #[test]
    fn a_programmatic_write_is_applied_like_a_user_write() {
        let mut mounted = MountedMapping::mount();
        let binding = mounted.captured("/name").binding;

        mounted.drive(|| binding.write("Grace".to_owned(), ChangeOrigin::Programmatic));

        assert_eq!(mounted.form_data()["name"], json!("Grace"));
    }

    #[test]
    fn commit_changes_nothing() {
        let mut mounted = MountedMapping::mount();
        let before = mounted.revisions();
        let binding = mounted.captured("/quantity").binding;

        mounted.drive(|| binding.commit());

        assert_eq!(mounted.revisions(), before);
        assert!(!mounted.captured("/quantity").meta.touched);
    }

    #[test]
    fn focus_exit_marks_the_control_touched() {
        let mut mounted = MountedMapping::mount();
        assert!(!mounted.captured("/name").meta.touched);
        let binding = mounted.captured("/name").binding;

        mounted.drive(|| binding.focus_exit());

        assert!(mounted.captured("/name").meta.touched);
        assert!(mounted.errors.borrow().is_empty());
    }

    #[test]
    fn the_binding_keeps_its_identity_across_renders() {
        let mut mounted = MountedMapping::mount();
        let before = mounted.captured("/quantity").binding;

        mounted.drive(|| before.write("7".to_owned(), ChangeOrigin::User));
        mounted.drive(|| before.focus_exit());

        let after = mounted.captured("/quantity").binding;
        assert_eq!(after, before);
        assert_eq!(mounted.read("/quantity"), "7");
    }

    #[test]
    fn a_write_only_value_is_not_echoed_through_the_binding() {
        let mounted = MountedMapping::mount();
        assert_eq!(mounted.read("/secret"), "");
        assert_eq!(mounted.form_data()["secret"], json!("hunter2"));
    }

    #[test]
    fn a_write_while_the_form_is_borrowed_reaches_on_error() {
        let mut mounted = MountedMapping::mount();
        let binding = mounted.captured("/quantity").binding;
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
        let edit = mounted.captured("/quantity").edit;

        mounted.drive(|| edit.input.call("-".to_owned()));

        let meta = mounted.captured("/quantity").meta;
        assert_eq!(meta.invalid, Some(true));
        assert_eq!(meta.errors, vec![Rc::from("Enter a valid integer.")]);
    }

    #[test]
    fn a_validation_finding_is_an_error_and_makes_the_field_invalid() {
        let mut mounted = MountedMapping::mount();
        let edit = mounted.captured("/name").edit;

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
        let edit = mounted.captured("/name").edit;
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
        let edit = mounted.captured("/name").edit;
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
}
