//! Accessible Dioxus browser rendering for [`schemaform::FormDefinition`].
//!
//! The crate keeps Dioxus state out of the core engine and provides explicit
//! renderer, finding presenter, localization, and extension seams, plus headless
//! [`edit`] hooks that give custom renderers the built-in editing behaviour.
//! [`SchemaForm`] renders unstyled semantic HTML and submits immutable
//! [`schemaform::SubmissionSnapshot`] values.
#![deny(rustdoc::broken_intra_doc_links)]
#![forbid(unsafe_code)]

use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
};

#[cfg(schemaform_test_validation_faults)]
use dioxus::prelude::use_hook;
use dioxus::prelude::{
    Callback, Element, EventHandler, Props, ReadableExt, Signal, WritableExt, rsx, use_callback,
    use_context_provider, use_effect, use_signal,
};
#[cfg(schemaform_test_validation_faults)]
use dioxus_core::use_drop;
use schemaform::{SubmissionOutcome, SubmissionSnapshot};
use serde_json::Value;

#[cfg(schemaform_test_validation_faults)]
mod render_observation {
    use schemaform::InstanceIdentity;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum RenderEvent {
        RendererEntered,
        Mounted,
        Dropped,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum RenderNodeKind {
        Control,
        StaticLayout,
        Collection,
        Unsupported,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    #[non_exhaustive]
    pub struct RenderObservation {
        pub event: RenderEvent,
        pub identity: InstanceIdentity,
        pub node_kind: RenderNodeKind,
        pub dom_id: String,
    }

    pub trait RenderObserver {
        fn observe(&self, observation: RenderObservation);
    }
}

/// Browser-local form ownership, reactive readers, and node-scoped actions.
pub mod handle {
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
        error::Error,
        fmt,
        rc::Rc,
    };

    use dioxus::prelude::{ReadableExt, Signal, WritableExt, use_hook};
    use dioxus_core::{ScopeId, current_scope_id, use_drop};
    use schemaform::{
        CapabilityFinding, DataRevision, DataSchemaAnnotations, ExternalFinding,
        ExternalFindingBatch, Form, FormBuildError, FormDefinition, InstanceIdentity, ItemIdentity,
        JsonPointer, StateRevision, SubmissionPreparation, Transition, ValidationFinding,
        definition::SemanticKind,
        form::{AllowedOperations, ParseBlockerKind, ScalarValueState},
    };
    use serde_json::Value;

    /// Cloneable browser-local ownership and operation handle for one core form.
    ///
    /// Clones share the same synchronous form state and compare equal only when they share that
    /// state. Public reads and operations use non-panicking dynamic borrows: re-entering this
    /// handle while a transaction or other borrow is active returns [`HandleError::BorrowConflict`].
    /// Retaining a handle after its creating Dioxus scope is removed is supported, but subsequent
    /// state access returns [`HandleError::Disposed`].
    /// Successful operations publish non-empty [`Transition`] values after releasing the core
    /// borrow so subscribed Dioxus scopes observe form- and changed-node updates.
    #[derive(Clone)]
    pub struct FormHandle {
        pub(crate) inner: Rc<HandleInner>,
    }

    pub(crate) struct HandleInner {
        pub(crate) form: RefCell<Form>,
        form_version: Signal<u64>,
        summary_projection: Signal<SummaryProjection>,
        lifecycle_version: Signal<u64>,
        node_versions: RefCell<HashMap<InstanceIdentity, Signal<u64>>>,
        owner_scope: ScopeId,
        disposed: Cell<bool>,
    }

    #[derive(Clone, PartialEq, Eq)]
    pub(crate) struct SummaryProjection {
        pub(crate) root: InstanceIdentity,
        pub(crate) findings: Vec<FindingProjection>,
    }

    impl PartialEq for FormHandle {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.inner, &other.inner)
        }
    }

    impl FormHandle {
        fn new(form: Form) -> Self {
            let root = form.view().root();
            let summary_projection = project_summary(&form);
            let mut identities = Vec::new();
            let mut pending = vec![root];
            while let Some(identity) = pending.pop() {
                identities.push(identity);
                let mut children = form
                    .node(identity)
                    .expect("definition nodes always have form instances")
                    .children()
                    .collect::<Vec<_>>();
                children.reverse();
                pending.extend(children);
            }
            let node_versions = identities
                .into_iter()
                .map(|identity| (identity, Signal::new(0)))
                .collect();
            Self {
                inner: Rc::new(HandleInner {
                    form: RefCell::new(form),
                    form_version: Signal::new(0),
                    summary_projection: Signal::new(summary_projection),
                    lifecycle_version: Signal::new(0),
                    node_versions: RefCell::new(node_versions),
                    owner_scope: current_scope_id(),
                    disposed: Cell::new(false),
                }),
            }
        }

        pub(crate) fn ensure_live(&self) -> Result<(), HandleError> {
            if self.inner.disposed.get() {
                Err(HandleError::Disposed)
            } else {
                Ok(())
            }
        }

        /// Creates a reader subscribed to form-wide state changes.
        ///
        /// Prefer [`FormHandle::node`] when a component needs only one node; its narrower
        /// subscription avoids rerendering for unrelated transitions.
        pub fn reader(&self) -> FormReader {
            FormReader {
                handle: self.clone(),
            }
        }

        /// Resolves a node-scoped reader for `identity`.
        ///
        /// Returns `Ok(None)` when the identity is not in the current form tree, including after
        /// collection removal. Re-entry while the form is borrowed returns
        /// [`HandleError::BorrowConflict`].
        pub fn node(&self, identity: InstanceIdentity) -> Result<Option<NodeReader>, HandleError> {
            self.ensure_live()?;
            self.inner
                .form
                .try_borrow()
                .map_err(|_| HandleError::BorrowConflict)
                .map(|form| {
                    form.node(identity).map(|_| NodeReader {
                        handle: self.clone(),
                        identity,
                    })
                })
        }

        /// Finalizes edit buffers and atomically prepares the current submission outcome.
        ///
        /// The returned preparation contains the transition plus either a ready snapshot or the
        /// blockers. Re-entry while the form is borrowed returns [`HandleError::BorrowConflict`].
        pub fn prepare_submission(&self) -> Result<SubmissionPreparation, HandleError> {
            self.ensure_live()?;
            let mut form = self
                .inner
                .form
                .try_borrow_mut()
                .map_err(|_| HandleError::BorrowConflict)?;
            let preparation = form.prepare_submission();
            drop(form);
            self.apply_transition(preparation.transition());
            Ok(preparation)
        }

        /// Restores the baseline, publishes the transition, and invalidates transient UI state.
        ///
        /// Re-entry while the form is borrowed returns [`HandleError::BorrowConflict`].
        pub fn reset(&self) -> Result<Transition, HandleError> {
            self.ensure_live()?;
            let mut form = self
                .inner
                .form
                .try_borrow_mut()
                .map_err(|_| HandleError::BorrowConflict)?;
            let transition = form.reset();
            drop(form);
            self.apply_transition(&transition);
            self.advance_lifecycle();
            Ok(transition)
        }

        /// Replaces form data and the reset baseline, then publishes the resulting transition.
        ///
        /// This also advances the adapter lifecycle used to discard stale browser edit state.
        /// Core rejection is returned as [`HandleError::Reinitialize`], and re-entry as
        /// [`HandleError::BorrowConflict`].
        pub fn reinitialize(&self, form_data: Value) -> Result<Transition, HandleError> {
            self.ensure_live()?;
            let transition = self
                .inner
                .form
                .try_borrow_mut()
                .map_err(|_| HandleError::BorrowConflict)?
                .reinitialize(form_data)
                .map_err(HandleError::Reinitialize)?;
            self.apply_transition(&transition);
            self.advance_lifecycle();
            Ok(transition)
        }

        /// Replaces revision-scoped host findings and publishes the resulting transition.
        ///
        /// Core batch errors remain available as [`HandleError::ExternalFindings`]; re-entry while
        /// the form is borrowed returns [`HandleError::BorrowConflict`].
        pub fn apply_external_findings(
            &self,
            batch: ExternalFindingBatch,
        ) -> Result<Transition, HandleError> {
            self.ensure_live()?;
            let transition = self
                .inner
                .form
                .try_borrow_mut()
                .map_err(|_| HandleError::BorrowConflict)?
                .apply_external_findings(batch)
                .map_err(HandleError::ExternalFindings)?;
            self.apply_transition(&transition);
            Ok(transition)
        }

        /// Changes which current findings are presented without changing their blocking effect.
        ///
        /// Re-entry while the form is borrowed returns [`HandleError::BorrowConflict`].
        pub fn set_finding_visibility(
            &self,
            policy: schemaform::FindingVisibilityPolicy,
        ) -> Result<Transition, HandleError> {
            self.ensure_live()?;
            let transition = self
                .inner
                .form
                .try_borrow_mut()
                .map_err(|_| HandleError::BorrowConflict)?
                .set_finding_visibility(policy);
            self.apply_transition(&transition);
            Ok(transition)
        }

        /// Runs one privileged core transaction and publishes its transition after commit.
        ///
        /// The closure runs synchronously while this handle is mutably borrowed and must not call
        /// back into the same handle. Such re-entry returns a handle-side borrow conflict to the
        /// nested call. Closure and commit failures are preserved in
        /// [`HandleTransactionError::Transaction`].
        pub fn try_transact<E, F>(
            &self,
            transaction: F,
        ) -> Result<Transition, HandleTransactionError<E>>
        where
            F: FnOnce(&mut schemaform::form::HostTransaction<'_>) -> Result<(), E>,
        {
            self.ensure_live().map_err(HandleTransactionError::Handle)?;
            let mut form = self
                .inner
                .form
                .try_borrow_mut()
                .map_err(|_| HandleTransactionError::Handle(HandleError::BorrowConflict))?;
            let transition = form
                .try_transact(transaction)
                .map_err(HandleTransactionError::Transaction)?;
            drop(form);
            self.apply_transition(&transition);
            Ok(transition)
        }

        pub(crate) fn input_text(
            &self,
            target: InstanceIdentity,
            text: &str,
        ) -> Result<Transition, HandleError> {
            self.apply_user_operation(|form| form.user().input_text(target, text))
        }

        pub(crate) fn set_value(
            &self,
            target: InstanceIdentity,
            value: Value,
        ) -> Result<Transition, HandleError> {
            self.apply_user_operation(|form| form.user().set_value(target, value))
        }

        pub(crate) fn replace_value(
            &self,
            target: InstanceIdentity,
            value: Value,
        ) -> Result<Transition, HandleError> {
            self.apply_user_operation(|form| form.user().replace_value(target, value))
        }

        pub(crate) fn set_null(&self, target: InstanceIdentity) -> Result<Transition, HandleError> {
            self.apply_user_operation(|form| form.user().set_null(target))
        }

        pub(crate) fn remove_value(
            &self,
            target: InstanceIdentity,
        ) -> Result<Transition, HandleError> {
            self.apply_user_operation(|form| form.user().remove_value(target))
        }

        pub(crate) fn materialize(
            &self,
            target: InstanceIdentity,
        ) -> Result<Transition, HandleError> {
            self.apply_user_operation(|form| form.user().materialize(target))
        }

        pub(crate) fn blur(&self, target: InstanceIdentity) -> Result<Transition, HandleError> {
            self.apply_user_operation(|form| form.user().blur(target))
        }

        fn apply_user_operation(
            &self,
            operation: impl FnOnce(
                &mut Form,
            )
                -> Result<Transition, schemaform::form::UserOperationError>,
        ) -> Result<Transition, HandleError> {
            self.ensure_live()?;
            let mut form = self
                .inner
                .form
                .try_borrow_mut()
                .map_err(|_| HandleError::BorrowConflict)?;
            let transition = operation(&mut form).map_err(HandleError::UserOperation)?;
            drop(form);
            self.apply_transition(&transition);
            Ok(transition)
        }

        fn apply_transition(&self, transition: &Transition) {
            if transition.is_empty() {
                return;
            }
            let mut form_version = self.inner.form_version;
            *form_version.write() += 1;
            let next_summary = project_summary(
                &self
                    .inner
                    .form
                    .try_borrow()
                    .expect("form mutations release their borrow before publishing"),
            );
            let mut summary_projection = self.inner.summary_projection;
            if summary_projection.peek().ne(&next_summary) {
                summary_projection.set(next_summary);
            }
            let (changed_versions, removed_versions) = {
                let mut node_versions = self.inner.node_versions.borrow_mut();
                let mut changed_versions = Vec::new();
                for identity in transition.changed() {
                    if !node_versions.contains_key(&identity)
                        && self
                            .inner
                            .form
                            .try_borrow()
                            .is_ok_and(|form| form.node(identity).is_some())
                    {
                        node_versions
                            .insert(identity, Signal::new_in_scope(0, self.inner.owner_scope));
                    }
                    if let Some(version) = node_versions.get(&identity) {
                        changed_versions.push(*version);
                    }
                }
                let removed_versions = transition
                    .removed()
                    .filter_map(|identity| node_versions.remove(&identity))
                    .collect::<Vec<_>>();
                (changed_versions, removed_versions)
            };
            for mut version in changed_versions {
                *version.write() += 1;
            }
            for mut version in removed_versions {
                {
                    let mut value = version.write();
                    *value += 1;
                }
                version.manually_drop();
            }
        }

        fn advance_lifecycle(&self) {
            let mut lifecycle_version = self.inner.lifecycle_version;
            *lifecycle_version.write() += 1;
        }

        /// Reads the adapter lifecycle version and subscribes the current reactive context to it.
        ///
        /// The version advances on [`FormHandle::reset`] and [`FormHandle::reinitialize`] so
        /// browser-local edit state started under an earlier lifecycle can be discarded.
        pub(crate) fn observe_lifecycle(&self) -> u64 {
            let lifecycle_version = self.inner.lifecycle_version;
            *lifecycle_version.read()
        }

        /// Reads the adapter lifecycle version without subscribing, for event handlers.
        pub(crate) fn peek_lifecycle(&self) -> u64 {
            let lifecycle_version = self.inner.lifecycle_version;
            *lifecycle_version.peek()
        }

        pub(crate) fn observe_form(&self) {
            let version = self.inner.form_version;
            let _ = *version.read();
        }

        pub(crate) fn summary_projection(&self) -> SummaryProjection {
            self.inner.summary_projection.read().clone()
        }

        pub(crate) fn observe_node(&self, identity: InstanceIdentity) {
            if let Some(version) = self.inner.node_versions.borrow().get(&identity) {
                let version = *version;
                let _ = *version.read();
            }
        }
    }

    /// Creates one browser-local [`FormHandle`] for the current Dioxus component scope.
    ///
    /// This is a Dioxus hook for browser client-side rendering. Call it unconditionally in a
    /// stable hook order; construction runs only on the initial hook invocation, so later input
    /// changes must be applied explicitly through [`FormHandle::reinitialize`]. SSR, hydration,
    /// desktop/WebView runtimes, and cross-thread ownership are outside this adapter's contract.
    pub fn use_form(
        definition: FormDefinition,
        form_data: Value,
    ) -> Result<FormHandle, FormBuildError> {
        let result = use_hook(move || definition.create_form(form_data).map(FormHandle::new));
        let inner = result.as_ref().ok().map(|handle| handle.inner.clone());
        use_drop(move || {
            if let Some(inner) = inner {
                inner.disposed.set(true);
            }
        });
        result
    }

    /// Cloneable form-wide reactive reader that returns owned projection snapshots.
    ///
    /// A successful [`FormReader::read`] subscribes the calling Dioxus scope to any published form
    /// transition. No core borrow escapes a reader method.
    #[derive(Clone)]
    pub struct FormReader {
        handle: FormHandle,
    }

    impl FormReader {
        /// Reads an owned form-wide projection and registers a form-level subscription.
        ///
        /// Re-entry while the form is borrowed returns [`HandleError::BorrowConflict`].
        pub fn read(&self) -> Result<FormProjection, HandleError> {
            self.handle.ensure_live()?;
            self.handle.observe_form();
            let form = self
                .handle
                .inner
                .form
                .try_borrow()
                .map_err(|_| HandleError::BorrowConflict)?;
            let view = form.view();
            let findings = project_visible_findings(&form);
            Ok(FormProjection {
                root: view.root(),
                data_revision: view.data_revision(),
                state_revision: view.state_revision(),
                submission_attempted: view.submission_attempted(),
                findings,
            })
        }

        /// Clones the canonical form data without registering a reactive subscription.
        ///
        /// Re-entry while the form is borrowed returns [`HandleError::BorrowConflict`].
        pub fn form_data(&self) -> Result<Value, HandleError> {
            self.handle.ensure_live()?;
            self.handle
                .inner
                .form
                .try_borrow()
                .map(|form| form.form_data().clone())
                .map_err(|_| HandleError::BorrowConflict)
        }
    }

    /// Cloneable reader and action authority for one form-tree node identity.
    ///
    /// Reads subscribe only to transitions for this identity. The reader can outlive a dynamic
    /// collection node; [`NodeReader::read`] then returns `Ok(None)`.
    ///
    /// Two readers compare equal when they observe the same node of the same form handle; the
    /// comparison is identity-based and does not read form state.
    #[derive(Clone)]
    pub struct NodeReader {
        handle: FormHandle,
        identity: InstanceIdentity,
    }

    impl PartialEq for NodeReader {
        fn eq(&self, other: &Self) -> bool {
            self.handle == other.handle && self.identity == other.identity
        }
    }

    impl fmt::Debug for NodeReader {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("NodeReader")
                .field("identity", &self.identity)
                .finish_non_exhaustive()
        }
    }

    impl NodeReader {
        /// Returns the stable form-tree identity scoped by this reader.
        pub fn identity(&self) -> InstanceIdentity {
            self.identity
        }

        pub(crate) fn handle(&self) -> &FormHandle {
            &self.handle
        }

        /// Reads an owned projection of this node and registers a node-level subscription.
        ///
        /// Returns `Ok(None)` if the node no longer exists. Re-entry while the form is borrowed
        /// returns [`HandleError::BorrowConflict`].
        pub fn read(&self) -> Result<Option<NodeProjection>, HandleError> {
            self.handle.ensure_live()?;
            self.handle.observe_node(self.identity);
            self.read_untracked()
        }

        pub(crate) fn read_untracked(&self) -> Result<Option<NodeProjection>, HandleError> {
            self.handle.ensure_live()?;
            let form = self
                .handle
                .inner
                .form
                .try_borrow()
                .map_err(|_| HandleError::BorrowConflict)?;
            let Some(node) = form.node(self.identity) else {
                return Ok(None);
            };
            let definition = node.definition();
            let selected_choice = node.selected_choice().map(|option| option.value().clone());
            let choice_options = definition
                .choice_options()
                .enumerate()
                .map(|(index, option)| ChoiceOptionProjection {
                    identity: ChoiceIdentity(format!("choice-{index}")),
                    value: option.value().clone(),
                    label: option.label().to_owned(),
                    selected: selected_choice
                        .as_ref()
                        .is_some_and(|selected| selected == option.value()),
                })
                .collect();
            let children = node.children().collect::<Vec<_>>();
            let collection_items = if matches!(
                definition.semantic_kind(),
                Some(SemanticKind::HomogeneousArray)
            ) {
                children
                    .iter()
                    .filter_map(|identity| {
                        form.node(*identity).and_then(|item| {
                            item.item_identity().map(|item| CollectionItemProjection {
                                identity: *identity,
                                item,
                            })
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Ok(Some(NodeProjection {
                identity: self.identity,
                binding: node.binding().map(|binding| binding.pointer().clone()),
                item: node.item_identity(),
                children,
                collection_items,
                label: definition.label().to_owned(),
                label_reference: definition.label_reference().cloned(),
                label_visible: definition.is_label_visible(),
                help: definition.help().map(str::to_owned),
                help_reference: definition.help_reference().cloned(),
                data_schema_annotations: definition.data_schema_annotations().clone(),
                creation_seed: definition.creation_seed().cloned(),
                value: node.display_text(),
                current_data: node.current_data().cloned(),
                value_state: node.value_state(),
                allowed_operations: node.allowed_operations(),
                read_only: node.is_read_only(),
                write_only: node.is_write_only(),
                required: definition.is_required(),
                nullable: definition.accepts_null(),
                edit_buffer: node.edit_buffer().map(str::to_owned),
                parse_blocker: node.parse_blocker(),
                validation_findings: node.validation_findings().cloned().collect(),
                capability_findings: node.capability_findings().cloned().collect(),
                external_findings: node
                    .external_findings()
                    .map(|(source, finding)| (source.to_owned(), finding.clone()))
                    .collect(),
                touched: node.is_touched(),
                dirty: node.is_dirty(),
                choice_options,
                choice_selectable: definition.is_choice_selectable(),
            }))
        }

        /// Returns the approved scalar-control actions scoped to this node.
        ///
        /// The core still validates whether each operation is allowed for the target.
        pub fn actions(&self) -> ControlActions {
            ControlActions {
                handle: self.handle.clone(),
                target: self.identity,
            }
        }

        /// Returns collection actions scoped to this node as the target array.
        ///
        /// The core rejects calls when this identity is not an applicable collection.
        pub fn collection_actions(&self) -> CollectionActions {
            CollectionActions {
                handle: self.handle.clone(),
                array: self.identity,
            }
        }
    }

    /// Owned form-wide state used by summaries and other aggregate presentation.
    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct FormProjection {
        /// Root identity of the current form tree.
        pub root: InstanceIdentity,
        /// Revision of canonical form data.
        pub data_revision: DataRevision,
        /// Revision of all observable form state.
        pub state_revision: StateRevision,
        /// Whether submission has been attempted since initialization or reset.
        pub submission_attempted: bool,
        /// Currently visible findings in core presentation order.
        pub findings: Vec<FindingProjection>,
    }

    /// Owned, presentation-visible finding projected from the core form view.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum FindingProjection {
        /// A data-schema validation finding.
        Validation {
            /// Form-tree node associated with the finding.
            target: InstanceIdentity,
            /// Structured validation details.
            finding: ValidationFinding,
        },
        /// Root marker that validation findings exceeded the form-wide retained limit.
        ValidationFindingsTruncated {
            /// Root form-tree node carrying the form-wide truncation marker.
            target: InstanceIdentity,
            /// Number of validation findings retained for the form.
            retained: usize,
        },
        /// Validation could not determine validity reliably.
        Indeterminate {
            /// Form-tree node associated with the outcome.
            target: InstanceIdentity,
            /// Structured reason evaluation was indeterminate.
            reason: schemaform::form::IndeterminateReason,
        },
        /// A form-capability limitation.
        Capability {
            /// Form-tree node associated with the limitation.
            target: InstanceIdentity,
            /// Structured capability details.
            finding: CapabilityFinding,
        },
        /// A revision-scoped finding supplied by the host.
        External {
            /// Form-tree node associated with the finding.
            target: InstanceIdentity,
            /// Host-defined source identifier.
            source: String,
            /// Structured external finding details.
            finding: ExternalFinding,
        },
        /// An edit buffer that cannot currently become canonical form data.
        Parse {
            /// Form-tree node containing the edit buffer.
            target: InstanceIdentity,
            /// Structured parse-blocker category.
            kind: ParseBlockerKind,
        },
    }

    fn project_summary(form: &Form) -> SummaryProjection {
        SummaryProjection {
            root: form.view().root(),
            findings: project_visible_findings(form),
        }
    }

    fn project_visible_findings(form: &Form) -> Vec<FindingProjection> {
        form.view()
            .visible_findings()
            .map(|finding| match finding {
                schemaform::FindingView::Validation { target, finding } => {
                    FindingProjection::Validation {
                        target,
                        finding: finding.clone(),
                    }
                }
                schemaform::FindingView::ValidationFindingsTruncated { target, retained } => {
                    FindingProjection::ValidationFindingsTruncated { target, retained }
                }
                schemaform::FindingView::Indeterminate { target, reason } => {
                    FindingProjection::Indeterminate {
                        target,
                        reason: reason.clone(),
                    }
                }
                schemaform::FindingView::Capability { target, finding } => {
                    FindingProjection::Capability {
                        target,
                        finding: finding.clone(),
                    }
                }
                schemaform::FindingView::External {
                    target,
                    source,
                    finding,
                } => FindingProjection::External {
                    target,
                    source: source.to_owned(),
                    finding: finding.clone(),
                },
                schemaform::FindingView::Parse { target, kind } => {
                    FindingProjection::Parse { target, kind }
                }
                _ => unreachable!("the adapter must cover every core finding family"),
            })
            .collect()
    }

    /// Owned observable state and definition annotations for one form-tree node.
    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct NodeProjection {
        /// Stable identity of this form-tree node.
        pub identity: InstanceIdentity,
        /// Current root-origin control binding, if the node is bound.
        pub binding: Option<JsonPointer>,
        /// Stable collection-item identity when this node is an item root or template descendant.
        pub item: Option<ItemIdentity>,
        /// Child identities in presentation order.
        pub children: Vec<InstanceIdentity>,
        /// Direct item-root children paired with stable identities when this node
        /// is a homogeneous array; empty for every other node kind.
        pub collection_items: Vec<CollectionItemProjection>,
        /// Resolved fallback label before adapter localization.
        pub label: String,
        /// Authored localizable label reference, when present.
        pub label_reference: Option<schemaform::ui::v1::TextReference>,
        /// Whether presentation should render the label visibly.
        pub label_visible: bool,
        /// Resolved fallback help text before adapter localization.
        pub help: Option<String>,
        /// Authored localizable help reference, when present.
        pub help_reference: Option<schemaform::ui::v1::TextReference>,
        /// Normalized data-schema annotations applicable to this node.
        pub data_schema_annotations: DataSchemaAnnotations,
        /// Value used to materialize an absent optional location or repair a required missing
        /// container.
        pub creation_seed: Option<Value>,
        /// Current scalar display text, preferring a retained edit buffer and then canonical
        /// formatting or a choice label.
        pub value: Option<String>,
        /// Canonical JSON value currently present at this node.
        pub current_data: Option<Value>,
        /// Scalar presence and nullability state, when applicable.
        pub value_state: Option<ScalarValueState>,
        /// Operations currently accepted by the core for this node.
        pub allowed_operations: AllowedOperations,
        /// Whether this node is effectively read-only.
        pub read_only: bool,
        /// Whether this node is write-only.
        pub write_only: bool,
        /// Whether the bound value is required by its parent shape.
        pub required: bool,
        /// Whether the bound scalar accepts JSON null independently of being required.
        pub nullable: bool,
        /// Exact in-progress textual edit, if one exists.
        pub edit_buffer: Option<String>,
        /// Parse blocker associated with the edit buffer, if any.
        pub parse_blocker: Option<ParseBlockerKind>,
        /// Validation findings local to this node.
        pub validation_findings: Vec<ValidationFinding>,
        /// Capability findings local to this node.
        pub capability_findings: Vec<CapabilityFinding>,
        /// Host findings local to this node, paired with source identifiers.
        pub external_findings: Vec<(String, ExternalFinding)>,
        /// Whether this node has been blurred.
        pub touched: bool,
        /// Whether this node's canonical data differs from its baseline.
        pub dirty: bool,
        /// Choice options in definition order, when applicable.
        pub choice_options: Vec<ChoiceOptionProjection>,
        /// Whether a choice control permits selecting among its options.
        pub choice_selectable: bool,
    }

    /// One current collection child and its stable item identity.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub struct CollectionItemProjection {
        /// Form-tree identity of the item root.
        pub identity: InstanceIdentity,
        /// Stable identity used by collection actions across reordering.
        pub item: ItemIdentity,
    }

    /// Adapter-owned stable identity for one projected choice option.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct ChoiceIdentity(String);

    impl ChoiceIdentity {
        /// Returns the identity as a DOM-key-friendly string.
        pub fn as_str(&self) -> &str {
            &self.0
        }
    }

    /// Owned presentation state for one choice option.
    #[derive(Debug, Clone, PartialEq)]
    #[non_exhaustive]
    pub struct ChoiceOptionProjection {
        /// Adapter identity stable for the definition's option order.
        pub identity: ChoiceIdentity,
        /// Canonical JSON value selected by this option.
        pub value: Value,
        /// Authored fallback label.
        pub label: String,
        /// Whether this option matches the node's current value.
        pub selected: bool,
    }

    /// Approved user-operation authority scoped to one control node.
    ///
    /// These methods are intended for browser event callbacks, not execution during rendering.
    /// Every operation is revalidated by the core and can report a borrow conflict on re-entry.
    ///
    /// Two action sets compare equal when they target the same node of the same form handle.
    #[derive(Clone)]
    pub struct ControlActions {
        handle: FormHandle,
        target: InstanceIdentity,
    }

    impl PartialEq for ControlActions {
        fn eq(&self, other: &Self) -> bool {
            self.handle == other.handle && self.target == other.target
        }
    }

    impl fmt::Debug for ControlActions {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("ControlActions")
                .field("target", &self.target)
                .finish_non_exhaustive()
        }
    }

    impl ControlActions {
        /// Applies textual input, preserving an edit buffer when parsing cannot complete.
        pub fn input_text(&self, text: impl AsRef<str>) -> Result<Transition, HandleError> {
            self.handle.input_text(self.target, text.as_ref())
        }

        /// Marks the target touched and finalizes blur-sensitive state.
        pub fn blur(&self) -> Result<Transition, HandleError> {
            self.handle.blur(self.target)
        }

        /// Sets the target to a core-accepted JSON value.
        pub fn set_value(&self, value: Value) -> Result<Transition, HandleError> {
            self.handle.set_value(self.target, value)
        }

        /// Sets an applicable nullable scalar target to JSON null.
        pub fn set_null(&self) -> Result<Transition, HandleError> {
            self.handle.set_null(self.target)
        }

        /// Removes an applicable optional value from canonical form data.
        pub fn remove_value(&self) -> Result<Transition, HandleError> {
            self.handle.remove_value(self.target)
        }

        /// Replaces an incompatible or write-only value with a new JSON value.
        pub fn replace_value(&self, value: Value) -> Result<Transition, HandleError> {
            self.handle.replace_value(self.target, value)
        }

        /// Materializes an absent optional target or repairs a required missing container from its
        /// definition seed.
        pub fn materialize(&self) -> Result<Transition, HandleError> {
            self.handle.materialize(self.target)
        }
    }

    /// Approved user-operation authority scoped to one collection node.
    ///
    /// Item-targeted methods use stable [`ItemIdentity`] values rather than array indexes. Calls
    /// are intended for browser event callbacks and are revalidated by the core.
    #[derive(Clone)]
    pub struct CollectionActions {
        handle: FormHandle,
        array: InstanceIdentity,
    }

    impl CollectionActions {
        /// Appends one seeded item to the target collection.
        pub fn append(&self) -> Result<Transition, HandleError> {
            self.handle
                .apply_user_operation(|form| form.user().append_item(self.array))
        }

        /// Inserts one seeded item immediately before `before`.
        pub fn insert_before(&self, before: ItemIdentity) -> Result<Transition, HandleError> {
            self.handle
                .apply_user_operation(|form| form.user().insert_item_before(self.array, before))
        }

        /// Removes the item identified by `item`.
        pub fn remove(&self, item: ItemIdentity) -> Result<Transition, HandleError> {
            self.handle
                .apply_user_operation(|form| form.user().remove_item(self.array, item))
        }

        /// Moves `item` one position toward the start of the collection.
        pub fn move_up(&self, item: ItemIdentity) -> Result<Transition, HandleError> {
            self.handle
                .apply_user_operation(|form| form.user().move_item_up(self.array, item))
        }

        /// Moves `item` one position toward the end of the collection.
        pub fn move_down(&self, item: ItemIdentity) -> Result<Transition, HandleError> {
            self.handle
                .apply_user_operation(|form| form.user().move_item_down(self.array, item))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    /// Failure to access or operate on a browser-local form handle.
    pub enum HandleError {
        /// The core rejected an approved user operation.
        UserOperation(schemaform::form::UserOperationError),
        /// The core rejected replacement form data.
        Reinitialize(schemaform::form::ReinitializeError),
        /// The core rejected a host finding batch.
        ExternalFindings(schemaform::form::ExternalFindingError),
        /// The Dioxus scope that created the handle has been disposed.
        Disposed,
        /// The handle was re-entered while its synchronous form state was already borrowed.
        BorrowConflict,
    }

    impl fmt::Display for HandleError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "form handle operation failed: {self:?}")
        }
    }

    impl Error for HandleError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::UserOperation(error) => Some(error),
                Self::Reinitialize(error) => Some(error),
                Self::ExternalFindings(error) => Some(error),
                Self::Disposed => None,
                Self::BorrowConflict => None,
            }
        }
    }

    #[derive(Debug)]
    #[non_exhaustive]
    /// Adapter or core failure from [`FormHandle::try_transact`].
    pub enum HandleTransactionError<E> {
        /// The adapter could not acquire the form borrow.
        Handle(HandleError),
        /// The transaction closure failed or the core rejected its commit.
        Transaction(schemaform::form::TransactionError<E>),
    }

    impl<E: fmt::Debug> fmt::Display for HandleTransactionError<E> {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "form handle transaction failed: {self:?}")
        }
    }

    impl<E: Error + 'static> Error for HandleTransactionError<E> {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::Handle(error) => Some(error),
                Self::Transaction(error) => Some(error),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::{
            cell::Cell,
            rc::Rc,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
        };

        use dioxus::prelude::{Element, Props, rsx};
        use dioxus_core::{ReactiveContext, ScopeId, VNode, VirtualDom};
        use schemaform::{
            ExtensionNamespace, FindingVisibility, FindingVisibilityPolicy, JsonPointer,
            WidgetSymbol,
            definition::DefinitionNodeView,
            ui::v1::{Binding, Control, Element as UiElement, ElementMeta, UiSchema},
        };
        use serde_json::json;

        use super::*;
        use crate::render::{
            BindFinding, BoundNode, BoundTemplateNode, ControlMatcher, ControlRegistry,
            ControlRenderContext, ControlRenderer, RenderConfiguration,
        };

        thread_local! {
            static REENTRANT_READER: RefCell<Option<NodeReader>> = const { RefCell::new(None) };
        }

        struct TestRenderer;

        impl ControlRenderer for TestRenderer {
            fn render(&self, _context: ControlRenderContext) -> dioxus::prelude::Element {
                dioxus::prelude::rsx! {}
            }
        }

        struct CountingMatcher(Arc<AtomicUsize>);

        impl ControlMatcher for CountingMatcher {
            fn matches(&self, _definition: DefinitionNodeView<'_>) -> bool {
                self.0.fetch_add(1, Ordering::Relaxed);
                true
            }
        }

        fn empty_app() -> dioxus_core::Element {
            VNode::empty()
        }

        #[derive(Clone, Props)]
        struct DisposalAppProps {
            mounted: Rc<Cell<bool>>,
            handle: Rc<RefCell<Option<FormHandle>>>,
        }

        impl PartialEq for DisposalAppProps {
            fn eq(&self, other: &Self) -> bool {
                Rc::ptr_eq(&self.mounted, &other.mounted) && Rc::ptr_eq(&self.handle, &other.handle)
            }
        }

        #[derive(Clone, Props)]
        struct DisposalOwnerProps {
            handle: Rc<RefCell<Option<FormHandle>>>,
        }

        impl PartialEq for DisposalOwnerProps {
            fn eq(&self, other: &Self) -> bool {
                Rc::ptr_eq(&self.handle, &other.handle)
            }
        }

        fn disposal_app(props: DisposalAppProps) -> Element {
            rsx! {
                if props.mounted.get() {
                    DisposalOwner { handle: props.handle }
                }
            }
        }

        #[allow(non_snake_case)]
        fn DisposalOwner(props: DisposalOwnerProps) -> Element {
            let definition = FormDefinition::compile(json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "rows": { "type": "array", "items": { "type": "string" } }
                }
            }))
            .expect("the disposal schema should compile");
            let handle = use_form(definition, json!({ "name": "Ada", "rows": ["first"] }))
                .expect("the disposal form should build");
            props.handle.borrow_mut().get_or_insert(handle);
            rsx! {}
        }

        fn identity_with_binding(handle: &FormHandle, binding: &str) -> InstanceIdentity {
            let form = handle.inner.form.borrow();
            let mut pending = vec![form.view().root()];
            while let Some(identity) = pending.pop() {
                let node = form.node(identity).expect("traversed nodes should exist");
                if node
                    .binding()
                    .is_some_and(|pointer| pointer.pointer().as_str() == binding)
                {
                    return identity;
                }
                pending.extend(node.children());
            }
            panic!("missing form node for {binding}");
        }

        #[test]
        fn retained_handle_rejects_public_access_after_owner_scope_is_disposed() {
            macro_rules! assert_disposed {
                ($result:expr) => {
                    assert!(matches!($result, Err(HandleError::Disposed)))
                };
            }

            let mounted = Rc::new(Cell::new(true));
            let retained = Rc::new(RefCell::new(None));
            let mut dom = VirtualDom::new_with_props(
                disposal_app,
                DisposalAppProps {
                    mounted: mounted.clone(),
                    handle: retained.clone(),
                },
            );
            dom.rebuild_in_place();

            let handle = retained
                .borrow()
                .clone()
                .expect("the child scope should expose its form handle");
            let reader = handle.reader();
            let revision = reader.read().unwrap().data_revision;
            let name_identity = identity_with_binding(&handle, "/name");
            let name = handle.node(name_identity).unwrap().unwrap();
            let name_actions = name.actions();
            let rows_identity = identity_with_binding(&handle, "/rows");
            let rows = handle.node(rows_identity).unwrap().unwrap();
            let rows_actions = rows.collection_actions();
            let item = rows.read().unwrap().unwrap().collection_items[0].item;
            let external_findings = ExternalFindingBatch::new(
                "server",
                revision,
                [ExternalFinding::advisory(
                    "check-name",
                    JsonPointer::parse("/name").unwrap(),
                    json!({}),
                )],
            );
            let (baseline_data, baseline_data_revision, baseline_state_revision) = {
                let form = handle.inner.form.borrow();
                (
                    form.form_data().clone(),
                    form.view().data_revision(),
                    form.view().state_revision(),
                )
            };

            let borrow = handle.inner.form.borrow_mut();
            assert!(matches!(
                name_actions.input_text("blocked by live borrow"),
                Err(HandleError::BorrowConflict)
            ));
            drop(borrow);

            mounted.set(false);
            dom.mark_dirty(ScopeId::APP);
            dom.render_immediate(&mut dioxus_core::NoOpMutations);

            assert_disposed!(handle.node(name_identity));
            assert_disposed!(reader.read());
            assert_disposed!(reader.form_data());
            assert_disposed!(name.read());
            assert_disposed!(handle.prepare_submission());
            assert_disposed!(handle.reset());
            assert_disposed!(handle.reinitialize(json!({ "name": "Grace", "rows": [] })));
            assert_disposed!(handle.apply_external_findings(external_findings));
            assert_disposed!(handle.set_finding_visibility(FindingVisibilityPolicy::default()));

            let transaction_called = Cell::new(false);
            assert!(matches!(
                handle.try_transact(|_| {
                    transaction_called.set(true);
                    Ok::<_, ()>(())
                }),
                Err(HandleTransactionError::Handle(HandleError::Disposed))
            ));
            assert!(!transaction_called.get());

            assert_disposed!(name_actions.input_text("Grace"));
            assert_disposed!(name_actions.blur());
            assert_disposed!(name_actions.set_value(json!("Grace")));
            assert_disposed!(name_actions.set_null());
            assert_disposed!(name_actions.remove_value());
            assert_disposed!(name_actions.replace_value(json!("Grace")));
            assert_disposed!(name_actions.materialize());
            assert_disposed!(rows_actions.append());
            assert_disposed!(rows_actions.insert_before(item));
            assert_disposed!(rows_actions.remove(item));
            assert_disposed!(rows_actions.move_up(item));
            assert_disposed!(rows_actions.move_down(item));

            let bind_error = RenderConfiguration::default()
                .bind(&handle)
                .err()
                .expect("a disposed handle must not produce a render plan");
            assert_eq!(
                bind_error.findings().collect::<Vec<_>>(),
                [&BindFinding::Disposed]
            );
            let form = handle.inner.form.borrow();
            assert_eq!(form.form_data(), &baseline_data);
            assert_eq!(form.view().data_revision(), baseline_data_revision);
            assert_eq!(form.view().state_revision(), baseline_state_revision);
        }

        #[test]
        fn removing_dynamic_node_invalidates_retained_reader_before_dropping_signal() {
            let dom = VirtualDom::new(empty_app);
            dom.in_scope(ScopeId::ROOT, || {
                let definition = FormDefinition::compile(json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "rows": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": { "name": { "type": "string" } }
                            }
                        }
                    }
                }))
                .expect("the array schema should compile");
                let handle = FormHandle::new(
                    definition
                        .create_form(json!({ "rows": [{ "name": "Ada" }] }))
                        .expect("the array form should build"),
                );
                let array = identity_with_binding(&handle, "/rows");
                let array_projection = handle
                    .node(array)
                    .unwrap()
                    .unwrap()
                    .read()
                    .unwrap()
                    .unwrap();
                let item = array_projection.collection_items[0];
                let retained = handle.node(item.identity).unwrap().unwrap();
                REENTRANT_READER.with(|reader| reader.replace(Some(retained.clone())));
                let invalidations = Arc::new(AtomicUsize::new(0));
                let count = invalidations.clone();
                let subscriber = ReactiveContext::new_with_callback(
                    move || {
                        REENTRANT_READER.with(|reader| {
                            assert!(reader.borrow().as_ref().unwrap().read().unwrap().is_none());
                        });
                        count.fetch_add(1, Ordering::Relaxed);
                    },
                    ScopeId::ROOT,
                    std::panic::Location::caller(),
                );
                assert!(subscriber.run_in(|| retained.read().unwrap()).is_some());

                handle
                    .node(array)
                    .unwrap()
                    .unwrap()
                    .collection_actions()
                    .remove(item.item)
                    .expect("the item should be removable");

                assert_eq!(invalidations.load(Ordering::Relaxed), 1);
                assert!(retained.read().unwrap().is_none());
                REENTRANT_READER.with(|reader| reader.replace(None));
            });
        }

        #[test]
        fn summary_signal_changes_only_with_visible_summary_projection() {
            let dom = VirtualDom::new(empty_app);
            dom.in_scope(ScopeId::ROOT, || {
                let definition = FormDefinition::compile(json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["quantity"],
                    "properties": {
                        "quantity": { "type": "integer", "minimum": 2 }
                    }
                }))
                .expect("the integer schema should compile");
                let handle = FormHandle::new(
                    definition
                        .create_form(json!({ "quantity": 2 }))
                        .expect("the integer form should build"),
                );
                handle
                    .set_finding_visibility(
                        FindingVisibilityPolicy::default()
                            .with_validation(FindingVisibility::Immediate),
                    )
                    .expect("visibility should update");
                let quantity = handle
                    .node(identity_with_binding(&handle, "/quantity"))
                    .unwrap()
                    .unwrap();
                let changes = Arc::new(AtomicUsize::new(0));
                let count = changes.clone();
                let subscriber = ReactiveContext::new_with_callback(
                    move || {
                        count.fetch_add(1, Ordering::Relaxed);
                    },
                    ScopeId::ROOT,
                    std::panic::Location::caller(),
                );
                assert!(
                    subscriber
                        .run_in(|| handle.summary_projection())
                        .findings
                        .is_empty()
                );

                quantity
                    .actions()
                    .input_text("1")
                    .expect("the invalid edit should commit");
                assert_eq!(changes.load(Ordering::Relaxed), 1);
                assert_eq!(handle.summary_projection().findings.len(), 1);

                quantity
                    .actions()
                    .input_text("0")
                    .expect("the second invalid edit should commit");
                assert_eq!(changes.load(Ordering::Relaxed), 1);

                quantity
                    .actions()
                    .input_text("2")
                    .expect("the valid edit should commit");
                assert_eq!(changes.load(Ordering::Relaxed), 2);
                assert!(handle.summary_projection().findings.is_empty());
            });
        }

        #[test]
        fn collection_widget_rejection_does_not_short_circuit_item_preflight() {
            let dom = VirtualDom::new(empty_app);
            dom.in_scope(ScopeId::ROOT, || {
                let array_widget = WidgetSymbol::parse("company:rows").unwrap();
                let item_widget = WidgetSymbol::parse("company:row").unwrap();
                let item_extension = ExtensionNamespace::parse("https://example.com/item").unwrap();
                let definition = FormDefinition::compiler(json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "rows": { "type": "array", "items": { "type": "string" } }
                    }
                }))
                .ui_schema(
                    UiSchema::new(UiElement::Control(
                        Control::new(Binding::root(JsonPointer::parse("/rows").unwrap()))
                            .widget(array_widget.clone())
                            .item_template(UiElement::Control(
                                Control::new(Binding::item(JsonPointer::parse("").unwrap()))
                                    .widget(item_widget.clone())
                                    .meta(ElementMeta::default().extension(
                                        item_extension.clone(),
                                        json!({ "enabled": true }),
                                    )),
                            )),
                    ))
                    .require_extension(item_extension.clone()),
                )
                .compile()
                .unwrap();
                let handle = FormHandle::new(
                    definition
                        .create_form(json!({ "rows": ["first"] }))
                        .unwrap(),
                );
                let error = RenderConfiguration::builder()
                    .controls(
                        ControlRegistry::with_builtins()
                            .widget(array_widget.clone(), Arc::new(TestRenderer)),
                    )
                    .build()
                    .bind(&handle)
                    .err()
                    .expect("a collection widget must fail binding");

                assert_eq!(
                    error.findings().cloned().collect::<Vec<_>>(),
                    [
                        BindFinding::MissingRequiredExtension(item_extension),
                        BindFinding::UnsupportedCollectionWidget(array_widget),
                        BindFinding::MissingWidget(item_widget),
                    ]
                );
            });
        }

        #[test]
        fn array_matchers_are_skipped_while_exact_item_renderers_are_retained() {
            let dom = VirtualDom::new(empty_app);
            dom.in_scope(ScopeId::ROOT, || {
                let item_widget = WidgetSymbol::parse("company:row").unwrap();
                let definition = FormDefinition::compiler(json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "rows": { "type": "array", "items": { "type": "string" } }
                    }
                }))
                .ui_schema(UiSchema::new(UiElement::Control(
                    Control::new(Binding::root(JsonPointer::parse("/rows").unwrap()))
                        .item_template(UiElement::Control(
                            Control::new(Binding::item(JsonPointer::parse("").unwrap()))
                                .widget(item_widget.clone()),
                        )),
                )))
                .compile()
                .unwrap();
                let handle = FormHandle::new(
                    definition
                        .create_form(json!({ "rows": ["first"] }))
                        .unwrap(),
                );
                let matcher_calls = Arc::new(AtomicUsize::new(0));
                let exact_renderer: Arc<dyn ControlRenderer> = Arc::new(TestRenderer);
                let bound = RenderConfiguration::builder()
                    .controls(
                        ControlRegistry::with_builtins()
                            .widget(item_widget, exact_renderer.clone())
                            .matcher(
                                10,
                                Arc::new(CountingMatcher(matcher_calls.clone())),
                                Arc::new(TestRenderer),
                            ),
                    )
                    .build()
                    .bind(&handle)
                    .expect("the adapter-owned array should bind");

                assert_eq!(matcher_calls.load(Ordering::Relaxed), 0);
                let BoundNode::Array(array) = &bound.inner.nodes[0] else {
                    panic!("the root child should remain an adapter-owned array");
                };
                let BoundTemplateNode::Control(control) = &array.template else {
                    panic!("the scalar item template should remain a control");
                };
                assert!(Arc::ptr_eq(&control.renderer, &exact_renderer));
            });
        }
    }
}

/// Render preflight, customization traits, and authority-limited render contexts.
pub mod render {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        error::Error,
        fmt,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use dioxus::prelude::{Callback, Element, ReadableExt, Signal, WritableExt};
    use schemaform::{
        DefinitionNodeId, ExtensionNamespace, Form, FormDefinition, InstanceIdentity, WidgetSymbol,
        definition::{DefinitionNodeKind, DefinitionNodeView, GridSpans, SemanticKind},
    };
    use serde_json::Value;

    use crate::handle::{ControlActions, FormHandle, HandleError, NodeReader};
    #[cfg(schemaform_test_validation_faults)]
    pub(crate) use crate::render_observation::{
        RenderEvent, RenderNodeKind, RenderObservation, RenderObserver,
    };

    static NEXT_BOUND_FORM_ID: AtomicU64 = AtomicU64::new(1);

    /// Matcher priority at which [`ControlRegistry::with_builtins`] registers
    /// [`BuiltinControlRenderer`].
    pub const BUILTIN_CONTROL_PRIORITY: i32 = 0;

    /// Renders one preflight-selected control with target-scoped authority.
    ///
    /// This callback runs synchronously during Dioxus rendering. Implementations should only read
    /// the supplied node while rendering and install [`ControlActions`] on event handlers; they
    /// must not mutate form state during the render call. The context intentionally omits the full
    /// [`FormHandle`].
    pub trait ControlRenderer: 'static {
        /// Returns the Dioxus element for the preflight-selected control.
        fn render(&self, context: ControlRenderContext) -> Element;
    }

    /// Selects a renderer using immutable definition-time information only.
    ///
    /// This synchronous preflight callback must be deterministic and must not depend on current
    /// form data, locale, findings, or interaction state. Renderer selection remains fixed for the
    /// resulting [`BoundForm`].
    pub trait ControlMatcher: 'static {
        /// Returns whether the registration accepts this definition node.
        fn matches(&self, definition: DefinitionNodeView<'_>) -> bool;
    }

    /// Renders one prepared local or summary finding collection.
    ///
    /// The callback runs synchronously during Dioxus rendering. Implementations receive owned
    /// presentation data and focus actions, but no form reader or mutation authority; any captured
    /// authority remains the host application's responsibility.
    pub trait FindingCollectionPresenter: 'static {
        /// Returns the Dioxus element for the supplied finding collection.
        fn render(&self, context: FindingCollectionContext) -> Element;
    }

    /// Resolves authored and adapter-owned messages intended for plain-text presentation.
    ///
    /// The callback is synchronous and may run during rendering. Built-ins render its return value
    /// as escaped text; custom renderers and presenters must not interpret it as markup.
    /// Implementations should avoid form mutation and return promptly; asynchronous loading is a
    /// host concern.
    pub trait Localizer: 'static {
        /// Resolves `message`, normally falling back to [`MessageDescriptor::fallback`].
        fn localize(&self, message: &MessageDescriptor) -> String;
    }

    /// Prepares one namespaced extension occurrence during atomic render preflight.
    ///
    /// The callback runs synchronously while [`RenderConfiguration::bind`] holds a read borrow of
    /// the form. Captured access must not mutate or re-enter that form. Rejection of a required
    /// extension fails binding; rejection of an optional extension is ignored.
    pub trait ExtensionHandler: 'static {
        /// Validates and prepares definition-stable extension data for later rendering.
        fn prepare(
            &self,
            occurrence: ExtensionOccurrence<'_>,
        ) -> Result<Arc<dyn PreparedExtension>, ExtensionPrepareError>;
    }

    /// Wraps adapter-owned presentation without receiving any core reader, action, or handle.
    ///
    /// This confines authority granted by the adapter; host implementations remain ordinary Rust
    /// code and are responsible for any authority they capture independently. The callback runs
    /// synchronously during rendering and should preserve the supplied child in its returned tree.
    pub trait PreparedExtension: 'static {
        /// Decorates `child` for one prepared definition and runtime instance occurrence.
        fn decorate(&self, context: ExtensionRenderContext, child: Element) -> Element;
    }

    /// Read and action context granted to a custom control renderer.
    ///
    /// Authority is node-scoped: the renderer can observe one [`NodeReader`] and invoke approved
    /// [`ControlActions`] for that node. It cannot obtain unrestricted form mutation through this
    /// context. Homogeneous array composition remains adapter-owned.
    ///
    /// The renderer owns the whole control region. The adapter renders exactly what
    /// [`ControlRenderer::render`] returns: it does not add a label, help text, local findings,
    /// presence affordances, or an `aria-describedby` relationship on the renderer's behalf.
    /// Everything needed to render those is pre-localized on
    /// [`ControlRenderContext::presentation`] and [`ControlRenderContext::control`], and
    /// operation failures reach the host through [`ControlRenderContext::report`].
    ///
    /// The context compares by value for presentation and facets, by identity for the reader,
    /// actions, and error route, and by pointer for prepared extensions, so it can be passed as a
    /// prop to a child component without Dioxus memoization showing stale state.
    #[derive(Clone, PartialEq)]
    pub struct ControlRenderContext {
        node: NodeReader,
        actions: ControlActions,
        presentation: NodePresentation,
        control: ControlFacets,
        extensions: PreparedExtensions,
        error_route: Option<crate::OperationErrorHandler>,
    }

    impl fmt::Debug for ControlRenderContext {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("ControlRenderContext")
                .field("node", &self.node)
                .field("presentation", &self.presentation)
                .field("control", &self.control)
                .finish_non_exhaustive()
        }
    }

    impl ControlRenderContext {
        pub(crate) fn new(
            node: NodeReader,
            actions: ControlActions,
            presentation: NodePresentation,
            control: ControlFacets,
            extensions: PreparedExtensions,
            error_route: Option<crate::OperationErrorHandler>,
        ) -> Self {
            Self {
                node,
                actions,
                presentation,
                control,
                extensions,
                error_route,
            }
        }

        /// Returns the target-scoped reactive reader for this control.
        pub fn node(&self) -> &NodeReader {
            &self.node
        }

        /// Returns approved actions scoped to this control's node.
        ///
        /// Install these actions on event callbacks rather than invoking them during rendering,
        /// and pass their results through [`ControlRenderContext::report`] so failures reach the
        /// host instead of being dropped.
        pub fn actions(&self) -> &ControlActions {
            &self.actions
        }

        /// Returns the adapter-computed, localized presentation for this node.
        pub fn presentation(&self) -> &NodePresentation {
            &self.presentation
        }

        /// Returns the control-specific facets derived from the definition and current state.
        pub fn control(&self) -> &ControlFacets {
            &self.control
        }

        /// Returns extension decorators prepared for this definition node during binding.
        ///
        /// The adapter applies these decorators automatically around the selected renderer.
        /// They are not an open-ended renderer-options or normalized schema-facet API.
        pub fn extensions(&self) -> &PreparedExtensions {
            &self.extensions
        }

        /// Routes a failed operation to the host's `SchemaForm::on_error` and returns the
        /// success value.
        ///
        /// `Ok(value)` returns `Some(value)` without side effects; `Err(error)` calls `on_error`
        /// with the error (or drops it when the host set no handler, as the built-ins do) and
        /// returns `None`. Use it around [`ControlActions`] calls installed on event callbacks,
        /// for example `context.report(actions.input_text(event.value()))` in an `oninput`
        /// handler. Affordances in [`NodePresentation::presence`] already report internally.
        pub fn report<T>(&self, result: Result<T, HandleError>) -> Option<T> {
            crate::route_operation(&self.error_route, result)
        }

        pub(crate) fn error_route(&self) -> &Option<crate::OperationErrorHandler> {
            &self.error_route
        }
    }

    /// Adapter-computed, localized presentation data for one form-tree node.
    ///
    /// The adapter computes one value per node render for controls, fixed-object groups,
    /// homogeneous arrays, and unsupported regions. Custom control renderers receive it through
    /// [`ControlRenderContext::presentation`] and are responsible for emitting the elements whose
    /// ids it references: the primary element carrying [`NodePresentation::element_id`], the help
    /// element carrying [`Help::id`], the finding elements carrying
    /// [`FindingDescriptor::stable_id`] (rendered directly or through
    /// [`NodePresentation::present_findings`]), and one element per [`Affordance`] in
    /// [`NodePresentation::presence`] carrying [`Affordance::id`].
    #[derive(Clone, PartialEq)]
    #[non_exhaustive]
    pub struct NodePresentation {
        /// DOM `id` the node's primary element must carry.
        ///
        /// Finding-summary focus, label association, and array focus management target this id.
        pub element_id: String,
        /// Localized plain-text label.
        pub label: String,
        /// Whether the label should be visibly rendered.
        ///
        /// The label is still required for an accessible name when this is `false`.
        pub label_visible: bool,
        /// Localized help text and the DOM id its element must carry, when help exists.
        pub help: Option<Help>,
        /// Local findings in presentation order: validation, capability, external, then parse.
        pub findings: Vec<FindingDescriptor>,
        /// Whether the node's current local state should be exposed as invalid.
        ///
        /// A node is invalid exactly when any of its local findings is blocking. Validation
        /// findings and parse blockers always block; capability and external findings block when
        /// the core marks them blocking. Advisory findings never make a node invalid.
        pub invalid: bool,
        /// Presence affordances the core allows for this node right now, in the built-in's
        /// order: set, set null, remove value, replace.
        ///
        /// Set is offered only while the value is missing or null and a creation seed exists;
        /// replace only while the core allows replacement and a seed exists; set null and remove
        /// value exactly when the core allows them. Renderers place these affordances; they do not
        /// reconstruct the rules. The list is empty for every node kind other than scalar
        /// controls.
        pub presence: Vec<Affordance>,
        form: BoundForm,
    }

    impl fmt::Debug for NodePresentation {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("NodePresentation")
                .field("element_id", &self.element_id)
                .field("label", &self.label)
                .field("label_visible", &self.label_visible)
                .field("help", &self.help)
                .field("findings", &self.findings)
                .field("invalid", &self.invalid)
                .field("presence", &self.presence)
                .finish_non_exhaustive()
        }
    }

    impl NodePresentation {
        pub(crate) fn new(
            form: BoundForm,
            element_id: String,
            label: String,
            label_visible: bool,
            help: Option<Help>,
            findings: Vec<FindingDescriptor>,
            presence: Vec<Affordance>,
        ) -> Self {
            let invalid = findings.iter().any(|finding| finding.blocking);
            Self {
                element_id,
                label,
                label_visible,
                help,
                findings,
                invalid,
                presence,
                form,
            }
        }

        /// Returns the space-joined `aria-describedby` value for the primary element.
        ///
        /// The value references the help id followed by every finding id, or is `None` when the
        /// node has neither help nor findings. Every referenced id names an element the renderer
        /// is responsible for emitting.
        pub fn described_by(&self) -> Option<String> {
            let ids = self
                .help
                .iter()
                .map(|help| help.id.as_str())
                .chain(
                    self.findings
                        .iter()
                        .map(|finding| finding.stable_id.as_str()),
                )
                .collect::<Vec<_>>();
            (!ids.is_empty()).then(|| ids.join(" "))
        }

        /// Renders the node's local findings through the configured local finding presenter.
        ///
        /// The returned element reads the presenter in a child scope, so swapping the presenter
        /// through [`RenderConfiguration::rebind_presentation`] updates the findings without
        /// calling the control renderer again. Each finding element carries its
        /// [`FindingDescriptor::stable_id`], and its focus action targets
        /// [`NodePresentation::element_id`].
        pub fn present_findings(&self) -> Element {
            crate::render_local_findings(&self.form, self.findings.clone(), self.element_id.clone())
        }

        /// Renders the help text as the built-in does: a `div.schemaform-help` carrying
        /// [`Help::id`], or nothing when the node has no help.
        ///
        /// Renderers that want different help markup render [`NodePresentation::help`] themselves
        /// and keep the id.
        pub fn present_help(&self) -> Element {
            let help = self.help.clone();
            dioxus::prelude::rsx! {
                if let Some(help) = help {
                    div { id: help.id, class: "schemaform-help", "{help.text}" }
                }
            }
        }

        /// The bound form this presentation localizes and renders findings through.
        pub(crate) fn form(&self) -> &BoundForm {
            &self.form
        }
    }

    /// Localized help text and the DOM id of the element that must present it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub struct Help {
        /// DOM `id` the help element must carry; [`NodePresentation::described_by`] references it.
        pub id: String,
        /// Localized plain-text help.
        pub text: String,
    }

    /// The operation an [`Affordance`] performs when invoked.
    ///
    /// Only presence operations on scalar controls are produced today. The enum grows as further
    /// renderer seams hand out affordances, so matches need a wildcard arm.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum AffordanceKind {
        /// Sets a missing or null value to its creation seed.
        Set,
        /// Sets a nullable value to JSON null.
        SetNull,
        /// Removes an optional value from its parent.
        RemoveValue,
        /// Replaces a value the control cannot edit with its creation seed.
        Replace,
    }

    /// A localized, pre-authorized user action handed to a renderer.
    ///
    /// Invoking an affordance performs the core operation on the node it was computed for and
    /// reports any failure to the host's `SchemaForm::on_error`; the renderer only places it. The
    /// adapter recomputes the list of affordances on every node render, so an affordance is
    /// present exactly while the core allows its operation.
    ///
    /// Two affordances compare equal when their `kind`, `label`, and `id` are equal. `invoke` is
    /// excluded because an affordance's behaviour is fixed by its node and kind, so a component
    /// that memoizes on an affordance and keeps an earlier `invoke` performs the same operation.
    #[derive(Clone)]
    #[non_exhaustive]
    pub struct Affordance {
        /// Operation performed by [`Affordance::invoke`].
        pub kind: AffordanceKind,
        /// Localized plain-text label for the element that triggers the affordance.
        pub label: String,
        /// DOM `id` the triggering element must carry.
        ///
        /// For presence affordances this is the node's [`NodePresentation::element_id`] followed
        /// by `-set-value`, `-set-null`, `-remove-value`, or `-replace-value`.
        pub id: String,
        /// Performs the operation and reports failures to the host's `SchemaForm::on_error`.
        ///
        /// Install this on an event callback rather than calling it during rendering.
        pub invoke: Callback<()>,
    }

    impl PartialEq for Affordance {
        fn eq(&self, other: &Self) -> bool {
            self.kind == other.kind && self.label == other.label && self.id == other.id
        }
    }

    impl fmt::Debug for Affordance {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("Affordance")
                .field("kind", &self.kind)
                .field("label", &self.label)
                .field("id", &self.id)
                .finish_non_exhaustive()
        }
    }

    /// Control-specific facets derived from the definition node and the node's current state.
    ///
    /// Every string is pre-localized through the configured [`Localizer`], so a custom renderer
    /// can reproduce the built-in write-only and boolean behaviour without the message catalog.
    #[derive(Debug, Clone, PartialEq)]
    #[non_exhaustive]
    pub struct ControlFacets {
        /// Widget family the adapter derived from the definition node.
        pub kind: ControlKind,
        /// Root-origin control binding as a JSON Pointer string, intended as the rendered `name`.
        pub name: String,
        /// Whether the control should present required semantics right now.
        pub required: bool,
        /// Whether the control is unavailable for interaction.
        pub disabled: bool,
        /// Whether the control permits observation but not ordinary editing.
        pub read_only: bool,
        /// Whether the bound value is write-only and must not be echoed back.
        pub write_only: bool,
        /// Whether the user has blurred this control.
        pub touched: bool,
        /// Whether canonical data at this control differs from its baseline.
        pub dirty: bool,
        /// Whether the bound scalar accepts JSON null.
        pub nullable: bool,
        /// Localized replacement chrome for a write-only control that can accept a new value.
        ///
        /// Present exactly when the built-in renders a replacement widget: the control is
        /// write-only, not read-only, and not a constant.
        pub write_only_replacement: Option<WriteOnlyReplacement>,
        /// Localized status text describing a write-only value without revealing it.
        ///
        /// Present for every write-only control.
        pub write_only_status: Option<String>,
        /// Localized labels for the two boolean values.
        ///
        /// Present for every boolean control.
        pub boolean_labels: Option<BooleanLabels>,
    }

    /// Localized label and placeholder for a write-only replacement widget.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub struct WriteOnlyReplacement {
        /// Localized label naming the replacement action, such as `Replace Password`.
        pub label: String,
        /// Localized placeholder shown before a replacement value is chosen.
        pub placeholder: String,
    }

    /// Localized labels for boolean values.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub struct BooleanLabels {
        /// Localized label for `false`.
        pub false_label: String,
        /// Localized label for `true`.
        pub true_label: String,
    }

    /// Immutable prepared extensions attached to one render context.
    ///
    /// Decorators are produced during binding and ordered canonically by extension namespace.
    #[derive(Clone, Default)]
    pub struct PreparedExtensions {
        values: Vec<(ExtensionNamespace, Arc<dyn PreparedExtension>)>,
    }

    impl PreparedExtensions {
        /// Iterates over namespace/prepared-value pairs in canonical invocation order.
        pub fn iter(
            &self,
        ) -> impl ExactSizeIterator<Item = (&ExtensionNamespace, &Arc<dyn PreparedExtension>)>
        {
            self.values
                .iter()
                .map(|(namespace, prepared)| (namespace, prepared))
        }
    }

    impl PartialEq for PreparedExtensions {
        fn eq(&self, other: &Self) -> bool {
            self.values.len() == other.values.len()
                && self.values.iter().zip(&other.values).all(
                    |((left_namespace, left), (right_namespace, right))| {
                        left_namespace == right_namespace && Arc::ptr_eq(left, right)
                    },
                )
        }
    }

    /// Owned presentation context for either local findings or the form summary.
    #[derive(Clone, PartialEq)]
    pub struct FindingCollectionContext {
        entries: Vec<FindingPresentation>,
        summary: bool,
    }

    impl FindingCollectionContext {
        pub(crate) fn local(
            findings: Vec<FindingDescriptor>,
            target_focus: TargetFocusAction,
        ) -> Self {
            Self {
                entries: findings
                    .into_iter()
                    .map(|finding| FindingPresentation {
                        finding,
                        target_focus: target_focus.clone(),
                    })
                    .collect(),
                summary: false,
            }
        }

        pub(crate) fn summary(findings: Vec<(FindingDescriptor, TargetFocusAction)>) -> Self {
            Self {
                entries: findings
                    .into_iter()
                    .map(|(finding, target_focus)| FindingPresentation {
                        finding,
                        target_focus,
                    })
                    .collect(),
                summary: true,
            }
        }

        /// Iterates findings paired with their corresponding focus actions.
        pub fn entries(&self) -> impl ExactSizeIterator<Item = &FindingPresentation> {
            self.entries.iter()
        }

        /// Iterates finding descriptors in presentation order.
        pub fn findings(&self) -> impl ExactSizeIterator<Item = &FindingDescriptor> {
            self.entries.iter().map(FindingPresentation::finding)
        }

        /// Iterates focus actions in the same order as [`FindingCollectionContext::findings`].
        pub fn target_focus(&self) -> impl ExactSizeIterator<Item = &TargetFocusAction> {
            self.entries.iter().map(FindingPresentation::target_focus)
        }

        /// Returns `true` for the form-wide summary and `false` for a node-local collection.
        pub fn is_summary(&self) -> bool {
            self.summary
        }
    }

    /// One prepared finding and the browser focus action for its target.
    #[derive(Clone, PartialEq)]
    pub struct FindingPresentation {
        finding: FindingDescriptor,
        target_focus: TargetFocusAction,
    }

    impl FindingPresentation {
        /// Returns the prepared finding descriptor.
        pub fn finding(&self) -> &FindingDescriptor {
            &self.finding
        }

        /// Returns the action that reveals and focuses the finding's target.
        pub fn target_focus(&self) -> &TargetFocusAction {
            &self.target_focus
        }
    }

    /// Localized, renderer-independent finding presentation data.
    #[derive(Debug, Clone, PartialEq)]
    #[non_exhaustive]
    pub struct FindingDescriptor {
        /// DOM-stable ID for the rendered finding.
        pub stable_id: String,
        /// Finding family used for semantic presentation.
        pub kind: FindingKind,
        /// Stable machine-readable finding code.
        pub code: String,
        /// Localized description intended for plain-text presentation.
        pub text: String,
        /// Whether this finding prevents a ready submission.
        pub blocking: bool,
        /// Structured finding-specific parameters.
        pub parameters: Value,
    }

    /// Presentation-level family of a form finding.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum FindingKind {
        /// Data-schema validation failure.
        Validation,
        /// Unsupported form capability.
        Capability,
        /// Revision-scoped host finding.
        External,
        /// Unparseable edit buffer.
        Parse,
        /// Validation could not determine validity.
        Indeterminate,
    }

    /// Browser action that reveals containing tabs and focuses a finding target.
    ///
    /// Focus is best-effort and meaningful only in the supported browser-CSR target. On other
    /// targets this operation is a no-op. Call it from an interaction callback, not while rendering.
    #[derive(Clone, PartialEq, Eq)]
    pub struct TargetFocusAction {
        target_id: Rc<str>,
        tab_ids: Vec<Rc<str>>,
    }

    impl TargetFocusAction {
        pub(crate) fn new(target_id: String) -> Self {
            Self {
                target_id: target_id.into(),
                tab_ids: Vec::new(),
            }
        }

        pub(crate) fn activate_tabs(mut self, tab_ids: Vec<String>) -> Self {
            self.tab_ids = tab_ids.into_iter().map(Into::into).collect();
            self
        }

        /// Reveals any containing tab panels and focuses the target DOM element.
        pub fn focus(&self) {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            let _ = (&self.target_id, &self.tab_ids);

            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            {
                use wasm_bindgen::JsCast;

                let Some(window) = web_sys::window() else {
                    return;
                };
                let Some(document) = window.document() else {
                    return;
                };
                for tab_id in &self.tab_ids {
                    if let Some(tab) = document
                        .get_element_by_id(tab_id)
                        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
                    {
                        tab.click();
                    }
                }
                if self.tab_ids.is_empty() {
                    crate::focus_element(&self.target_id);
                } else {
                    focus_element_after_render(self.target_id.clone(), 4);
                }
            }
        }
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn focus_element_after_render(target_id: Rc<str>, attempts: u8) {
        use wasm_bindgen::{JsCast, closure::Closure};

        if crate::focus_element(&target_id) || attempts == 0 {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let callback = Closure::once_into_js(move || {
            focus_element_after_render(target_id, attempts - 1);
        });
        window.queue_microtask(callback.unchecked_ref());
    }

    #[derive(Debug, Clone, PartialEq)]
    #[non_exhaustive]
    /// A localizable message with a ready-to-render fallback and structured parameters.
    pub struct MessageDescriptor {
        /// Stable localization key, or `None` for caller-owned literal text.
        pub key: Option<String>,
        /// Required plain-text fallback suitable for immediate presentation.
        pub fallback: String,
        /// Structured values available to interpolation-aware localizers.
        pub parameters: Value,
    }

    /// Borrowed definition-time occurrence passed to an [`ExtensionHandler`].
    #[non_exhaustive]
    pub struct ExtensionOccurrence<'a> {
        /// Exact namespace URI used to select the handler.
        pub namespace: &'a ExtensionNamespace,
        /// Definition node carrying this occurrence.
        pub definition_node: DefinitionNodeId,
        /// Bounded opaque authored extension value.
        pub value: &'a Value,
    }

    /// Identity-only context supplied while a prepared extension decorates one instance.
    ///
    /// It deliberately carries no form reader, actions, or unrestricted handle.
    #[derive(Clone)]
    pub struct ExtensionRenderContext {
        definition_node: DefinitionNodeId,
        instance: InstanceIdentity,
        namespace: ExtensionNamespace,
    }

    impl ExtensionRenderContext {
        pub(crate) fn new(
            definition_node: DefinitionNodeId,
            instance: InstanceIdentity,
            namespace: ExtensionNamespace,
        ) -> Self {
            Self {
                definition_node,
                instance,
                namespace,
            }
        }

        /// Returns the definition node where the extension was authored.
        pub fn definition_node(&self) -> DefinitionNodeId {
            self.definition_node
        }

        /// Returns the runtime form-tree instance being decorated.
        pub fn instance(&self) -> InstanceIdentity {
            self.instance
        }

        /// Returns the exact extension namespace selected during preflight.
        pub fn namespace(&self) -> &ExtensionNamespace {
            &self.namespace
        }
    }

    /// Builder-style registry of exact widget and semantic matcher renderers.
    ///
    /// Exact widget requests never fall back to semantic matching. Duplicate exact registrations
    /// and equal winning matcher priorities are reported during [`RenderConfiguration::bind`]
    /// rather than resolved by registration order. Homogeneous arrays are not renderer candidates:
    /// exact requests on them produce [`BindFinding::UnsupportedCollectionWidget`], and matchers
    /// are not called for them.
    ///
    /// The built-in renderer is an ordinary registration: [`ControlRegistry::with_builtins`]
    /// registers [`BuiltinControlRenderer`] for every supported semantic kind at
    /// [`BUILTIN_CONTROL_PRIORITY`], and [`ControlRegistry::empty`] starts without it. Resolution
    /// has no built-in special case; a control no registration accepts produces
    /// [`BindFinding::NoMatchingRenderer`].
    #[derive(Clone)]
    pub struct ControlRegistry {
        exact: Vec<(WidgetSymbol, Arc<dyn ControlRenderer>)>,
        matchers: Vec<MatcherRegistration>,
    }

    #[derive(Clone)]
    struct MatcherRegistration {
        priority: i32,
        matcher: Arc<dyn ControlMatcher>,
        renderer: Arc<dyn ControlRenderer>,
    }

    /// Accepts every definition node the built-in renderer can present: those with a
    /// [`ControlKind`].
    struct BuiltinControlMatcher;

    impl ControlMatcher for BuiltinControlMatcher {
        fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
            ControlKind::from_definition(definition).is_some()
        }
    }

    impl ControlRegistry {
        /// Creates a registry with no renderers.
        ///
        /// Every control must then be accepted by a registered exact widget or matcher, or
        /// binding reports [`BindFinding::NoMatchingRenderer`] for it. Use this when no unstyled
        /// built-in may appear in a themed form.
        pub fn empty() -> Self {
            Self {
                exact: Vec::new(),
                matchers: Vec::new(),
            }
        }

        /// Creates a registry backed by the adapter's built-in semantic controls.
        ///
        /// Equivalent to [`ControlRegistry::empty`] followed by registering
        /// [`BuiltinControlRenderer`] at [`BUILTIN_CONTROL_PRIORITY`] with a matcher that accepts
        /// every supported semantic kind.
        pub fn with_builtins() -> Self {
            Self::empty().matcher(
                BUILTIN_CONTROL_PRIORITY,
                Arc::new(BuiltinControlMatcher),
                Arc::new(BuiltinControlRenderer),
            )
        }

        /// Registers a renderer for one exact authored widget symbol.
        ///
        /// Registering the same symbol more than once is allowed here but produces
        /// [`BindFinding::AmbiguousWidget`] if that symbol is requested by an eligible control
        /// during preflight. Registration does not enable array-level widget replacement.
        pub fn widget(mut self, symbol: WidgetSymbol, renderer: Arc<dyn ControlRenderer>) -> Self {
            self.exact.push((symbol, renderer));
            self
        }

        /// Registers a definition matcher and its renderer at an explicit priority.
        ///
        /// Matchers are evaluated for eligible controls, including controls instantiated from an
        /// array item template, but never for the homogeneous array node itself.
        ///
        /// The highest priority among matching registrations wins. In a registry created by
        /// [`ControlRegistry::with_builtins`] the built-in occupies [`BUILTIN_CONTROL_PRIORITY`],
        /// so a matching custom priority below that cannot replace it, and equality with it is
        /// ambiguous, as are multiple matching renderers tied at any winning priority.
        pub fn matcher(
            mut self,
            priority: i32,
            matcher: Arc<dyn ControlMatcher>,
            renderer: Arc<dyn ControlRenderer>,
        ) -> Self {
            self.matchers.push(MatcherRegistration {
                priority,
                matcher,
                renderer,
            });
            self
        }
    }

    /// The adapter's built-in scalar control renderer.
    ///
    /// It renders string, number, integer, boolean, choice, and constant controls as unstyled
    /// semantic HTML with `schemaform-*` class hooks, built on the public
    /// [`ControlRenderContext`] and the headless edit hooks exactly as a custom renderer would
    /// be. [`ControlRegistry::with_builtins`] registers it at [`BUILTIN_CONTROL_PRIORITY`]; a
    /// host can also register it for an exact widget symbol or at another priority.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct BuiltinControlRenderer;

    impl ControlRenderer for BuiltinControlRenderer {
        fn render(&self, context: ControlRenderContext) -> Element {
            // The kind is definition-stable, so each node always renders the same child
            // component and every hook inside it is called unconditionally.
            match context.control().kind {
                ControlKind::String | ControlKind::Number | ControlKind::Integer => {
                    dioxus::prelude::rsx! { crate::BuiltinTextControl { context } }
                }
                ControlKind::Boolean => {
                    dioxus::prelude::rsx! { crate::BuiltinBooleanControl { context } }
                }
                ControlKind::Choice => {
                    dioxus::prelude::rsx! { crate::BuiltinChoiceControl { context } }
                }
                ControlKind::Constant => {
                    dioxus::prelude::rsx! { crate::BuiltinConstantControl { context } }
                }
            }
        }
    }

    /// Shareable renderer, presentation, localization, and extension configuration.
    ///
    /// Configuration does not mutate form state. [`RenderConfiguration::bind`] resolves all
    /// definition-stable choices before mounting so renderer type cannot change with runtime data.
    #[derive(Clone)]
    pub struct RenderConfiguration {
        controls: ControlRegistry,
        local_presenter: Arc<dyn FindingCollectionPresenter>,
        summary_presenter: Arc<dyn FindingCollectionPresenter>,
        localizer: Arc<dyn Localizer>,
        extensions: Vec<(ExtensionNamespace, Arc<dyn ExtensionHandler>)>,
        grid_wide_breakpoint_css_px: u32,
        #[cfg(schemaform_test_validation_faults)]
        observer: Option<Arc<dyn RenderObserver>>,
    }

    impl Default for RenderConfiguration {
        fn default() -> Self {
            Self {
                controls: ControlRegistry::with_builtins(),
                local_presenter: Arc::new(DefaultPresenter),
                summary_presenter: Arc::new(DefaultPresenter),
                localizer: Arc::new(FallbackLocalizer),
                extensions: Vec::new(),
                grid_wide_breakpoint_css_px: 640,
                #[cfg(schemaform_test_validation_faults)]
                observer: None,
            }
        }
    }

    impl RenderConfiguration {
        /// Starts a builder with the default built-ins, presenters, localizer, and breakpoint.
        pub fn builder() -> RenderConfigurationBuilder {
            RenderConfigurationBuilder::default()
        }

        /// Atomically preflights renderers and extensions and creates a single-mount render plan.
        ///
        /// Matching and extension preparation run synchronously while the form is immutably
        /// borrowed. The method reports all collected [`BindFinding`] values and returns no partial
        /// plan on failure. Re-entering the same handle from a preflight callback is unsupported;
        /// mutation attempts observe a borrow conflict.
        pub fn bind(&self, form: &FormHandle) -> Result<BoundForm, BindError> {
            form.ensure_live()
                .map_err(|_| BindError::new(BindFinding::Disposed))?;
            let core_form = form
                .inner
                .form
                .try_borrow()
                .map_err(|_| BindError::new(BindFinding::BorrowConflict))?;
            let root = core_form.view().root();
            let root = core_form
                .node(root)
                .expect("a form always instantiates its definition root");
            let mut findings = Vec::new();
            let prepared_extensions =
                self.prepare_extensions(core_form.definition(), &mut findings);
            let bound_form_id = NEXT_BOUND_FORM_ID.fetch_add(1, Ordering::Relaxed);
            let mut node_index = 0;
            let nodes = root
                .children()
                .filter_map(|identity| {
                    assemble_bound_node(
                        &core_form,
                        self,
                        identity,
                        bound_form_id,
                        &mut node_index,
                        &prepared_extensions,
                        &mut findings,
                    )
                })
                .collect();
            drop(core_form);

            if !findings.is_empty() {
                return Err(BindError { findings });
            }
            Ok(BoundForm {
                inner: Rc::new(BoundFormInner {
                    handle: form.clone(),
                    form_id: format!("schemaform-{bound_form_id}"),
                    nodes,
                    local_presenter: Signal::new(self.local_presenter.clone()),
                    summary_presenter: Signal::new(self.summary_presenter.clone()),
                    localizer: Signal::new(self.localizer.clone()),
                    grid_wide_breakpoint_css_px: self.grid_wide_breakpoint_css_px,
                    #[cfg(schemaform_test_validation_faults)]
                    observer: self.observer.clone(),
                }),
            })
        }

        /// Reactively replaces presenters and localization without rebuilding the render plan.
        ///
        /// Controls, extensions, generated DOM identity, and the grid breakpoint remain those
        /// selected by the original bind; call [`RenderConfiguration::bind`] for those changes.
        /// Core form state is not changed.
        pub fn rebind_presentation(&self, form: &BoundForm) {
            let mut local_presenter = form.inner.local_presenter;
            if !Arc::ptr_eq(&local_presenter.peek(), &self.local_presenter) {
                local_presenter.set(self.local_presenter.clone());
            }
            let mut summary_presenter = form.inner.summary_presenter;
            if !Arc::ptr_eq(&summary_presenter.peek(), &self.summary_presenter) {
                summary_presenter.set(self.summary_presenter.clone());
            }
            let mut localizer = form.inner.localizer;
            if !Arc::ptr_eq(&localizer.peek(), &self.localizer) {
                localizer.set(self.localizer.clone());
            }
        }

        fn resolve_control(
            &self,
            definition: DefinitionNodeView<'_>,
            findings: &mut Vec<BindFinding>,
        ) -> Option<(ControlKind, Arc<dyn ControlRenderer>)> {
            let Some(semantic_kind) = definition.semantic_kind() else {
                findings.push(BindFinding::UnsupportedDefinitionNode);
                return None;
            };
            let Some(kind) = ControlKind::from_definition(definition) else {
                findings.push(BindFinding::UnsupportedSemanticKind(semantic_kind));
                return None;
            };
            let renderer = self.resolve_renderer(definition, findings)?;
            Some((kind, renderer))
        }

        fn resolve_renderer(
            &self,
            definition: DefinitionNodeView<'_>,
            findings: &mut Vec<BindFinding>,
        ) -> Option<Arc<dyn ControlRenderer>> {
            if let Some(widget) = definition.widget() {
                let mut renderers = self
                    .controls
                    .exact
                    .iter()
                    .filter(|(symbol, _)| symbol == widget)
                    .map(|(_, renderer)| renderer.clone());
                let Some(renderer) = renderers.next() else {
                    findings.push(BindFinding::MissingWidget(widget.clone()));
                    return None;
                };
                if renderers.next().is_some() {
                    findings.push(BindFinding::AmbiguousWidget(widget.clone()));
                    return None;
                }
                return Some(renderer);
            }

            // Every registration, the built-in included, competes on priority alone.
            let mut highest_priority = None;
            let mut winners = Vec::new();
            for registration in &self.controls.matchers {
                if !registration.matcher.matches(definition) {
                    continue;
                }
                match highest_priority {
                    Some(highest) if registration.priority < highest => {}
                    Some(highest) if registration.priority == highest => {
                        winners.push(registration.renderer.clone());
                    }
                    _ => {
                        highest_priority = Some(registration.priority);
                        winners.clear();
                        winners.push(registration.renderer.clone());
                    }
                }
            }
            match winners.len() {
                0 => {
                    findings.push(BindFinding::NoMatchingRenderer {
                        definition_node: definition.id(),
                    });
                    None
                }
                1 => winners.pop(),
                _ => {
                    findings.push(BindFinding::AmbiguousMatcher);
                    None
                }
            }
        }

        fn prepare_extensions(
            &self,
            definition: &FormDefinition,
            findings: &mut Vec<BindFinding>,
        ) -> HashMap<DefinitionNodeId, PreparedExtensions> {
            let required = definition
                .required_extensions()
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut occurrences =
                BTreeMap::<ExtensionNamespace, Vec<(DefinitionNodeId, &Value)>>::new();
            let mut pending = vec![definition.root()];
            while let Some(id) = pending.pop() {
                let node = definition
                    .node(id)
                    .expect("definition traversal only contains valid node IDs");
                for (namespace, value) in node.extensions() {
                    occurrences
                        .entry(namespace.clone())
                        .or_default()
                        .push((id, value));
                }
                let mut children = node.children().collect::<Vec<_>>();
                children.reverse();
                pending.extend(children);
            }

            let mut prepared = HashMap::<DefinitionNodeId, PreparedExtensions>::new();
            for (namespace, namespace_occurrences) in occurrences {
                let handlers = self
                    .extensions
                    .iter()
                    .filter(|(registered, _)| registered == &namespace)
                    .map(|(_, handler)| handler)
                    .collect::<Vec<_>>();
                let is_required = required.contains(&namespace);
                let Some(handler) = handlers.first() else {
                    if is_required {
                        findings.push(BindFinding::MissingRequiredExtension(namespace));
                    }
                    continue;
                };
                if handlers.len() > 1 {
                    findings.push(BindFinding::AmbiguousExtension(namespace));
                    continue;
                }
                for (definition_node, value) in namespace_occurrences {
                    match handler.prepare(ExtensionOccurrence {
                        namespace: &namespace,
                        definition_node,
                        value,
                    }) {
                        Ok(value) => prepared
                            .entry(definition_node)
                            .or_default()
                            .values
                            .push((namespace.clone(), value)),
                        Err(error) if is_required => {
                            findings.push(BindFinding::InvalidRequiredExtension {
                                namespace: namespace.clone(),
                                definition_node,
                                error,
                            });
                        }
                        Err(_) => {}
                    }
                }
            }
            prepared
        }
    }

    /// Consuming builder for [`RenderConfiguration`].
    #[derive(Clone, Default)]
    pub struct RenderConfigurationBuilder {
        configuration: RenderConfiguration,
    }

    impl RenderConfigurationBuilder {
        /// Replaces the control registry used during preflight.
        pub fn controls(mut self, controls: ControlRegistry) -> Self {
            self.configuration.controls = controls;
            self
        }

        /// Sets the presenter for node-local finding collections.
        pub fn local_presenter(mut self, presenter: Arc<dyn FindingCollectionPresenter>) -> Self {
            self.configuration.local_presenter = presenter;
            self
        }

        /// Sets the presenter for the form-wide finding summary.
        pub fn summary_presenter(mut self, presenter: Arc<dyn FindingCollectionPresenter>) -> Self {
            self.configuration.summary_presenter = presenter;
            self
        }

        /// Sets the synchronous localizer for text that built-ins render escaped.
        pub fn localizer(mut self, localizer: Arc<dyn Localizer>) -> Self {
            self.configuration.localizer = localizer;
            self
        }

        /// Registers an extension handler for an exact namespace URI.
        ///
        /// Duplicate namespaces are retained and reported as [`BindFinding::AmbiguousExtension`]
        /// when the namespace occurs during preflight.
        pub fn extension(
            mut self,
            namespace: ExtensionNamespace,
            handler: Arc<dyn ExtensionHandler>,
        ) -> Self {
            self.configuration.extensions.push((namespace, handler));
            self
        }

        /// Sets the CSS-pixel breakpoint at which authored grid cells use their wide spans.
        pub fn grid_wide_breakpoint_css_px(mut self, breakpoint: u32) -> Self {
            self.configuration.grid_wide_breakpoint_css_px = breakpoint;
            self
        }

        /// Installs the repository's renderer-lifecycle qualification observer.
        #[cfg(schemaform_test_validation_faults)]
        pub fn observer(mut self, observer: Arc<dyn RenderObserver>) -> Self {
            self.configuration.observer = Some(observer);
            self
        }

        /// Finishes the immutable render configuration.
        pub fn build(self) -> RenderConfiguration {
            self.configuration
        }
    }

    struct DefaultPresenter;

    impl FindingCollectionPresenter for DefaultPresenter {
        fn render(&self, context: FindingCollectionContext) -> Element {
            let local = !context.summary;
            let findings = context
                .entries
                .into_iter()
                .map(|entry| {
                    let target = context.summary.then_some(entry.target_focus);
                    (entry.finding, target)
                })
                .collect::<Vec<_>>();
            dioxus::prelude::rsx! {
                for (finding, target_focus) in findings {
                    div {
                        id: finding.stable_id,
                        class: "schemaform-finding",
                        "data-finding": finding.code.clone(),
                        "data-validation-finding": (local && finding.kind == FindingKind::Validation)
                            .then_some(finding.code.clone()),
                        "data-capability-finding": (finding.kind == FindingKind::Capability)
                            .then_some(finding.code.clone()),
                        "data-external-finding": (finding.kind == FindingKind::External)
                            .then_some(finding.code.clone()),
                        "data-parse-blocker": (finding.kind == FindingKind::Parse)
                            .then_some(finding.code),
                        "data-blocking": finding.blocking.to_string(),
                        if let Some(target_focus) = target_focus {
                            button {
                                r#type: "button",
                                onclick: move |_| target_focus.focus(),
                                "{finding.text}"
                            }
                        } else {
                            "{finding.text}"
                        }
                    }
                }
            }
        }
    }

    struct FallbackLocalizer;

    impl Localizer for FallbackLocalizer {
        fn localize(&self, message: &MessageDescriptor) -> String {
            message.fallback.clone()
        }
    }

    /// Preflighted render plan for one [`FormHandle`].
    ///
    /// A bound form owns generated DOM identities. Its clones refer to the same plan and must not
    /// be mounted concurrently; bind the handle separately for each concurrent view. The plan is
    /// intended only for this crate's browser-CSR [`crate::SchemaForm`] component.
    #[derive(Clone)]
    pub struct BoundForm {
        pub(crate) inner: Rc<BoundFormInner>,
    }

    pub(crate) struct BoundFormInner {
        pub(crate) handle: FormHandle,
        pub(crate) form_id: String,
        pub(crate) nodes: Vec<BoundNode>,
        pub(crate) local_presenter: Signal<Arc<dyn FindingCollectionPresenter>>,
        pub(crate) summary_presenter: Signal<Arc<dyn FindingCollectionPresenter>>,
        pub(crate) localizer: Signal<Arc<dyn Localizer>>,
        pub(crate) grid_wide_breakpoint_css_px: u32,
        #[cfg(schemaform_test_validation_faults)]
        pub(crate) observer: Option<Arc<dyn RenderObserver>>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) enum BoundNode {
        Decorated(BoundDecorated),
        Control(BoundControl),
        Group(BoundGroup),
        Stack(BoundStack),
        Grid(BoundGrid),
        GridCell(BoundGridCell),
        AuthoredGroup(BoundAuthoredGroup),
        Tabs(BoundTabs),
        TabPanel(BoundTabPanel),
        Text(BoundText),
        Array(BoundArray),
        Unsupported(BoundUnsupported),
    }

    impl BoundNode {
        pub(crate) fn key(&self) -> String {
            let identity = match self {
                Self::Decorated(decorated) => decorated.identity,
                Self::Control(control) => control.identity,
                Self::Group(group) => group.identity,
                Self::Stack(stack) => stack.identity,
                Self::Grid(grid) => grid.identity,
                Self::GridCell(cell) => cell.identity,
                Self::AuthoredGroup(group) => group.identity,
                Self::Tabs(tabs) => tabs.identity,
                Self::TabPanel(panel) => panel.identity,
                Self::Text(text) => text.identity,
                Self::Array(array) => array.identity,
                Self::Unsupported(region) => region.identity,
            };
            format!("instance-{identity:?}")
        }

        #[cfg(schemaform_test_validation_faults)]
        pub(crate) fn observation(&self) -> Option<(InstanceIdentity, RenderNodeKind, String)> {
            match self {
                Self::Decorated(_) => None,
                Self::Control(control) => Some((
                    control.identity,
                    RenderNodeKind::Control,
                    control.input_id.clone(),
                )),
                Self::Group(group) => Some((
                    group.identity,
                    RenderNodeKind::StaticLayout,
                    group.element_id.clone(),
                )),
                Self::Stack(stack) => Some((
                    stack.identity,
                    RenderNodeKind::StaticLayout,
                    stack.element_id.clone(),
                )),
                Self::Grid(grid) => Some((
                    grid.identity,
                    RenderNodeKind::StaticLayout,
                    grid.element_id.clone(),
                )),
                Self::GridCell(cell) => Some((
                    cell.identity,
                    RenderNodeKind::StaticLayout,
                    cell.element_id.clone(),
                )),
                Self::AuthoredGroup(group) => Some((
                    group.identity,
                    RenderNodeKind::StaticLayout,
                    group.element_id.clone(),
                )),
                Self::Tabs(tabs) => Some((
                    tabs.identity,
                    RenderNodeKind::StaticLayout,
                    tabs.element_id.clone(),
                )),
                Self::TabPanel(_) => None,
                Self::Text(text) => Some((
                    text.identity,
                    RenderNodeKind::StaticLayout,
                    text.element_id.clone(),
                )),
                Self::Array(array) => Some((
                    array.identity,
                    RenderNodeKind::Collection,
                    array.element_id.clone(),
                )),
                Self::Unsupported(region) => Some((
                    region.identity,
                    RenderNodeKind::Unsupported,
                    region.element_id.clone(),
                )),
            }
        }
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundDecorated {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) identity: InstanceIdentity,
        pub(crate) child: Box<BoundNode>,
        pub(crate) extensions: PreparedExtensions,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundGroup {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) label: String,
        pub(crate) help: Option<String>,
        pub(crate) children: Vec<BoundNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundStack {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) transparent: bool,
        pub(crate) children: Vec<BoundNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundGrid {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) cells: Vec<BoundNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundGridCell {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) spans: GridSpans,
        pub(crate) children: Vec<BoundNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundAuthoredGroup {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) label: schemaform::ui::v1::TextReference,
        pub(crate) children: Vec<BoundNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTabs {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) panels: Vec<BoundNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTabPanel {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) label: schemaform::ui::v1::TextReference,
        pub(crate) children: Vec<BoundNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundText {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) content: schemaform::ui::v1::TextReference,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundArray {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
        pub(crate) item_label: Option<schemaform::ui::v1::TextReference>,
        pub(crate) template: BoundTemplateNode,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) enum BoundTemplateNode {
        Decorated(BoundTemplateDecorated),
        Control(BoundTemplateControl),
        Group(BoundTemplateGroup),
        Stack(BoundTemplateStack),
        Grid(BoundTemplateGrid),
        GridCell(BoundTemplateGridCell),
        AuthoredGroup(BoundTemplateAuthoredGroup),
        Tabs(BoundTemplateTabs),
        TabPanel(BoundTemplateTabPanel),
        Text(BoundTemplateText),
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateDecorated {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) child: Box<BoundTemplateNode>,
        pub(crate) extensions: PreparedExtensions,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateGroup {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) label: String,
        pub(crate) help: Option<String>,
        pub(crate) children: Vec<BoundTemplateNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateStack {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) transparent: bool,
        pub(crate) children: Vec<BoundTemplateNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateGrid {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) cells: Vec<BoundTemplateNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateGridCell {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) spans: GridSpans,
        pub(crate) children: Vec<BoundTemplateNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateAuthoredGroup {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) label: schemaform::ui::v1::TextReference,
        pub(crate) children: Vec<BoundTemplateNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateTabs {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) panels: Vec<BoundTemplateNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateTabPanel {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) label: schemaform::ui::v1::TextReference,
        pub(crate) children: Vec<BoundTemplateNode>,
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundTemplateText {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) content: schemaform::ui::v1::TextReference,
    }

    #[derive(Clone)]
    pub(crate) struct BoundTemplateControl {
        pub(crate) definition: DefinitionNodeId,
        pub(crate) kind: ControlKind,
        pub(crate) renderer: Arc<dyn ControlRenderer>,
        pub(crate) extensions: PreparedExtensions,
    }

    impl PartialEq for BoundTemplateControl {
        fn eq(&self, other: &Self) -> bool {
            self.definition == other.definition
                && self.kind == other.kind
                && Arc::ptr_eq(&self.renderer, &other.renderer)
                && self.extensions == other.extensions
        }
    }

    #[derive(Clone, PartialEq)]
    pub(crate) struct BoundUnsupported {
        pub(crate) identity: InstanceIdentity,
        pub(crate) element_id: String,
    }

    fn assemble_bound_node(
        form: &Form,
        configuration: &RenderConfiguration,
        identity: InstanceIdentity,
        bound_form_id: u64,
        node_index: &mut usize,
        prepared_extensions: &HashMap<DefinitionNodeId, PreparedExtensions>,
        findings: &mut Vec<BindFinding>,
    ) -> Option<BoundNode> {
        let node = form
            .node(identity)
            .expect("definition nodes always have form instances");
        let definition = node.definition();
        let index = *node_index;
        *node_index += 1;
        if definition.kind() == DefinitionNodeKind::Unsupported {
            let bound = BoundNode::Unsupported(BoundUnsupported {
                identity,
                element_id: format!("schemaform-{bound_form_id}-unsupported-{index}"),
            });
            return Some(decorate_bound_node(
                bound,
                definition.id(),
                identity,
                prepared_extensions,
            ));
        }
        if node.definition().semantic_kind() == Some(SemanticKind::HomogeneousArray) {
            if let Some(widget) = definition.widget() {
                findings.push(BindFinding::UnsupportedCollectionWidget(widget.clone()));
            }
            let template = node
                .definition()
                .children()
                .next()
                .expect("compiled arrays own one item template");
            let template = assemble_bound_template(
                form.definition(),
                configuration,
                template,
                prepared_extensions,
                findings,
            );
            let template = template?;
            let bound = BoundNode::Array(BoundArray {
                identity,
                element_id: format!("schemaform-{bound_form_id}-array-{index}"),
                item_label: definition.item_label_reference().cloned(),
                template,
            });
            return Some(decorate_bound_node(
                bound,
                definition.id(),
                identity,
                prepared_extensions,
            ));
        }
        if definition.kind() == DefinitionNodeKind::Control {
            let (kind, renderer) = configuration.resolve_control(definition, findings)?;
            let bound = BoundNode::Control(BoundControl {
                identity,
                input_id: format!("schemaform-{bound_form_id}-control-{index}"),
                name: definition
                    .binding()
                    .expect("control definitions have bindings")
                    .as_str()
                    .to_owned(),
                kind,
                renderer,
                extensions: prepared_extensions
                    .get(&definition.id())
                    .cloned()
                    .unwrap_or_default(),
            });
            return Some(decorate_bound_node(
                bound,
                definition.id(),
                identity,
                prepared_extensions,
            ));
        }
        let children = node
            .children()
            .filter_map(|child| {
                assemble_bound_node(
                    form,
                    configuration,
                    child,
                    bound_form_id,
                    node_index,
                    prepared_extensions,
                    findings,
                )
            })
            .collect::<Vec<_>>();
        let bound = match definition.kind() {
            DefinitionNodeKind::AutoGeneratedLayout
                if definition.semantic_kind() == Some(SemanticKind::FixedObject) =>
            {
                Some(BoundNode::Group(BoundGroup {
                    identity,
                    element_id: format!("schemaform-{bound_form_id}-group-{index}"),
                    label: definition.label().to_owned(),
                    help: definition.help().map(str::to_owned),
                    children,
                }))
            }
            DefinitionNodeKind::Stack => Some(BoundNode::Stack(BoundStack {
                identity,
                element_id: format!("schemaform-{bound_form_id}-stack-{index}"),
                transparent: false,
                children,
            })),
            DefinitionNodeKind::Grid => Some(BoundNode::Grid(BoundGrid {
                identity,
                element_id: format!("schemaform-{bound_form_id}-grid-{index}"),
                cells: children,
            })),
            DefinitionNodeKind::GridCell => {
                let spans = definition
                    .grid_spans()
                    .expect("compiled grid cells contain responsive spans");
                Some(BoundNode::GridCell(BoundGridCell {
                    identity,
                    element_id: format!("schemaform-{bound_form_id}-grid-cell-{index}"),
                    spans,
                    children,
                }))
            }
            DefinitionNodeKind::Group => Some(BoundNode::AuthoredGroup(BoundAuthoredGroup {
                identity,
                element_id: format!("schemaform-{bound_form_id}-authored-group-{index}"),
                label: definition
                    .label_reference()
                    .expect("authored groups contain title references")
                    .clone(),
                children,
            })),
            DefinitionNodeKind::Tabs => Some(BoundNode::Tabs(BoundTabs {
                identity,
                element_id: format!("schemaform-{bound_form_id}-tabs-{index}"),
                panels: children,
            })),
            DefinitionNodeKind::TabPanel => Some(BoundNode::TabPanel(BoundTabPanel {
                identity,
                element_id: format!("schemaform-{bound_form_id}-tab-panel-{index}"),
                label: definition
                    .label_reference()
                    .expect("authored tab panels contain title references")
                    .clone(),
                children,
            })),
            DefinitionNodeKind::Text => Some(BoundNode::Text(BoundText {
                identity,
                element_id: format!("schemaform-{bound_form_id}-text-{index}"),
                content: definition
                    .text()
                    .expect("authored text definitions contain text")
                    .clone(),
            })),
            DefinitionNodeKind::AutoGeneratedLayout => Some(BoundNode::Stack(BoundStack {
                identity,
                element_id: format!("schemaform-{bound_form_id}-generated-region-{index}"),
                transparent: true,
                children,
            })),
            _ => {
                findings.push(BindFinding::UnsupportedDefinitionNode);
                None
            }
        }?;
        Some(decorate_bound_node(
            bound,
            definition.id(),
            identity,
            prepared_extensions,
        ))
    }

    fn decorate_bound_node(
        child: BoundNode,
        definition: DefinitionNodeId,
        identity: InstanceIdentity,
        prepared_extensions: &HashMap<DefinitionNodeId, PreparedExtensions>,
    ) -> BoundNode {
        let extensions = prepared_extensions
            .get(&definition)
            .cloned()
            .unwrap_or_default();
        if extensions.values.is_empty() {
            child
        } else {
            BoundNode::Decorated(BoundDecorated {
                definition,
                identity,
                child: Box::new(child),
                extensions,
            })
        }
    }

    fn assemble_bound_template(
        definition: &FormDefinition,
        configuration: &RenderConfiguration,
        id: DefinitionNodeId,
        prepared_extensions: &HashMap<DefinitionNodeId, PreparedExtensions>,
        findings: &mut Vec<BindFinding>,
    ) -> Option<BoundTemplateNode> {
        let node = definition
            .node(id)
            .expect("item-template nodes belong to their definition");
        if node.kind() == DefinitionNodeKind::Control {
            let (kind, renderer) = configuration.resolve_control(node, findings)?;
            let bound = BoundTemplateNode::Control(BoundTemplateControl {
                definition: id,
                kind,
                renderer,
                extensions: prepared_extensions.get(&id).cloned().unwrap_or_default(),
            });
            return Some(decorate_bound_template(bound, id, prepared_extensions));
        }
        let mut children = || {
            node.children()
                .filter_map(|child| {
                    assemble_bound_template(
                        definition,
                        configuration,
                        child,
                        prepared_extensions,
                        findings,
                    )
                })
                .collect()
        };
        let bound = match node.kind() {
            DefinitionNodeKind::AutoGeneratedLayout
                if node.semantic_kind() == Some(SemanticKind::FixedObject) =>
            {
                Some(BoundTemplateNode::Group(BoundTemplateGroup {
                    definition: id,
                    label: node.label().to_owned(),
                    help: node.help().map(str::to_owned),
                    children: children(),
                }))
            }
            DefinitionNodeKind::Stack => Some(BoundTemplateNode::Stack(BoundTemplateStack {
                definition: id,
                transparent: false,
                children: children(),
            })),
            DefinitionNodeKind::Grid => Some(BoundTemplateNode::Grid(BoundTemplateGrid {
                definition: id,
                cells: children(),
            })),
            DefinitionNodeKind::GridCell => {
                let spans = node
                    .grid_spans()
                    .expect("compiled grid cells contain responsive spans");
                Some(BoundTemplateNode::GridCell(BoundTemplateGridCell {
                    definition: id,
                    spans,
                    children: children(),
                }))
            }
            DefinitionNodeKind::Group => Some(BoundTemplateNode::AuthoredGroup(
                BoundTemplateAuthoredGroup {
                    definition: id,
                    label: node
                        .label_reference()
                        .expect("authored groups contain title references")
                        .clone(),
                    children: children(),
                },
            )),
            DefinitionNodeKind::Tabs => Some(BoundTemplateNode::Tabs(BoundTemplateTabs {
                definition: id,
                panels: children(),
            })),
            DefinitionNodeKind::TabPanel => {
                Some(BoundTemplateNode::TabPanel(BoundTemplateTabPanel {
                    definition: id,
                    label: node
                        .label_reference()
                        .expect("authored tab panels contain title references")
                        .clone(),
                    children: children(),
                }))
            }
            DefinitionNodeKind::Text => Some(BoundTemplateNode::Text(BoundTemplateText {
                definition: id,
                content: node
                    .text()
                    .expect("authored text definitions contain text")
                    .clone(),
            })),
            DefinitionNodeKind::AutoGeneratedLayout => {
                Some(BoundTemplateNode::Stack(BoundTemplateStack {
                    definition: id,
                    transparent: true,
                    children: children(),
                }))
            }
            _ => {
                findings.push(BindFinding::UnsupportedDefinitionNode);
                None
            }
        }?;
        Some(decorate_bound_template(bound, id, prepared_extensions))
    }

    fn decorate_bound_template(
        child: BoundTemplateNode,
        definition: DefinitionNodeId,
        prepared_extensions: &HashMap<DefinitionNodeId, PreparedExtensions>,
    ) -> BoundTemplateNode {
        let extensions = prepared_extensions
            .get(&definition)
            .cloned()
            .unwrap_or_default();
        if extensions.values.is_empty() {
            child
        } else {
            BoundTemplateNode::Decorated(BoundTemplateDecorated {
                definition,
                child: Box::new(child),
                extensions,
            })
        }
    }

    #[derive(Clone)]
    pub(crate) struct BoundControl {
        pub(crate) identity: InstanceIdentity,
        pub(crate) input_id: String,
        pub(crate) name: String,
        pub(crate) kind: ControlKind,
        /// The preflight-selected renderer; the built-in is one possible selection.
        pub(crate) renderer: Arc<dyn ControlRenderer>,
        pub(crate) extensions: PreparedExtensions,
    }

    impl PartialEq for BoundControl {
        fn eq(&self, other: &Self) -> bool {
            self.identity == other.identity
                && self.input_id == other.input_id
                && self.name == other.name
                && self.kind == other.kind
                && Arc::ptr_eq(&self.renderer, &other.renderer)
                && self.extensions == other.extensions
        }
    }

    /// Widget family the adapter derives from a control's definition node.
    ///
    /// The derivation uses only definition-time information, so the kind is fixed for the
    /// lifetime of a [`BoundForm`]. `Constant` covers nodes that present a fixed value without
    /// offering a selection: non-selectable choices and null-typed nodes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum ControlKind {
        /// Free-text string input.
        String,
        /// Decimal number input.
        Number,
        /// Integer input.
        Integer,
        /// Two-state or nullable boolean.
        Boolean,
        /// Selection among compiled choice options.
        Choice,
        /// Fixed value presented without a selection.
        Constant,
    }

    impl ControlKind {
        fn from_definition(definition: DefinitionNodeView<'_>) -> Option<Self> {
            match definition.semantic_kind()? {
                SemanticKind::String => Some(Self::String),
                SemanticKind::Number => Some(Self::Number),
                SemanticKind::Integer => Some(Self::Integer),
                SemanticKind::Boolean => Some(Self::Boolean),
                SemanticKind::Choice if definition.is_choice_selectable() => Some(Self::Choice),
                SemanticKind::Choice | SemanticKind::Null => Some(Self::Constant),
                _ => None,
            }
        }

        pub(crate) fn data_attribute(self) -> &'static str {
            match self {
                Self::String => "string",
                Self::Number => "number",
                Self::Integer => "integer",
                Self::Boolean => "boolean",
                Self::Choice => "choice",
                Self::Constant => "constant",
            }
        }

        pub(crate) fn input_mode(self) -> &'static str {
            match self {
                Self::String => "text",
                Self::Number => "decimal",
                Self::Integer => "numeric",
                Self::Boolean | Self::Choice | Self::Constant => "text",
            }
        }
    }

    impl PartialEq for BoundForm {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.inner, &other.inner)
        }
    }

    impl BoundForm {
        /// Returns a clone of the form handle rendered by this plan.
        pub fn handle(&self) -> FormHandle {
            self.inner.handle.clone()
        }

        pub(crate) fn render(&self) -> Element {
            dioxus::prelude::rsx! { crate::BoundControls { form: self.clone() } }
        }
    }

    /// Atomic render-preflight failure containing one or more structured findings.
    #[derive(Debug)]
    pub struct BindError {
        findings: Vec<BindFinding>,
    }

    impl BindError {
        fn new(finding: BindFinding) -> Self {
            Self {
                findings: vec![finding],
            }
        }

        /// Iterates all findings collected during the failed preflight.
        pub fn findings(&self) -> impl Iterator<Item = &BindFinding> {
            self.findings.iter()
        }
    }

    impl fmt::Display for BindError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "render configuration has {} finding(s)",
                self.findings.len()
            )
        }
    }

    impl Error for BindError {}

    /// One reason a render plan could not be bound safely and deterministically.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum BindFinding {
        /// An exact widget was requested without a registered renderer.
        MissingWidget(WidgetSymbol),
        /// More than one renderer was registered for an exact widget.
        AmbiguousWidget(WidgetSymbol),
        /// A homogeneous array requested a widget, but collection composition is adapter-owned.
        UnsupportedCollectionWidget(WidgetSymbol),
        /// More than one renderer won semantic matching at the highest priority.
        AmbiguousMatcher,
        /// No registered exact widget applied and no matcher accepted a control definition.
        ///
        /// Reachable only through a registry that does not include the built-in renderer, such
        /// as one created by [`ControlRegistry::empty`].
        NoMatchingRenderer {
            /// Definition node of the control no renderer accepted.
            definition_node: DefinitionNodeId,
        },
        /// No handler was registered for an authored required extension namespace.
        MissingRequiredExtension(ExtensionNamespace),
        /// More than one handler was registered for an occurring namespace.
        AmbiguousExtension(ExtensionNamespace),
        /// A required extension occurrence could not be prepared.
        InvalidRequiredExtension {
            /// Exact namespace URI of the required extension.
            namespace: ExtensionNamespace,
            /// Definition node containing the rejected occurrence.
            definition_node: DefinitionNodeId,
            /// Handler-supplied rejection category.
            error: ExtensionPrepareError,
        },
        /// No built-in control contract supports this semantic kind.
        UnsupportedSemanticKind(SemanticKind),
        /// The definition contains a node kind the adapter cannot render.
        UnsupportedDefinitionNode,
        /// The form handle's creating Dioxus scope has been disposed.
        Disposed,
        /// Preflight could not borrow the form because the handle was re-entered.
        BorrowConflict,
    }

    /// Stable rejection category returned by [`ExtensionHandler::prepare`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ExtensionPrepareError {
        /// The authored value is malformed for the extension contract.
        InvalidValue,
        /// The value is valid but unsupported by this handler.
        UnsupportedValue,
    }

    impl fmt::Display for ExtensionPrepareError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "extension preparation failed: {self:?}")
        }
    }

    impl Error for ExtensionPrepareError {}
}

/// Headless edit hooks that give a custom control renderer the built-in editing behaviour.
///
/// Each hook is called inside the renderer's own child component with the
/// [`render::ControlRenderContext`] it received, and returns hook-stable callbacks plus a derived
/// read signal the component wires to its widget. The hooks own the correctness-critical parts of
/// editing so renderers place widgets rather than reimplementing IME composition, lifecycle
/// discard, or DOM resynchronisation after the core rejects input.
pub mod edit {
    use std::{fmt, rc::Rc};

    use dioxus::prelude::{
        Callback, Memo, ReadSignal, ReadableExt, Signal, WritableExt, use_callback, use_effect,
        use_hook, use_memo, use_signal,
    };
    use schemaform::form::AllowedOperations;
    use serde_json::Value;

    use crate::{
        handle::{
            ChoiceIdentity, ControlActions, FormHandle, HandleError, NodeProjection, NodeReader,
        },
        render::ControlRenderContext,
    };

    /// Headless text-editing behaviour for one string, number, or integer control.
    ///
    /// Obtained from [`use_text_edit`]. The callbacks keep their identity across renders and
    /// `value` is a read signal, so a widget that receives this value as a prop does not
    /// re-render per keystroke and stays wired to the live control.
    ///
    /// Two values compare equal when they come from the same hook call site, that is when their
    /// `value` signal and callbacks are the same handles, and `read_only` agrees; `value`'s
    /// current text is not compared. The struct is non-exhaustive so later releases can add
    /// fields without breaking renderers; it is only ever constructed by the hook.
    #[derive(Clone, Copy, PartialEq)]
    #[non_exhaustive]
    pub struct TextEdit {
        /// Text the widget should display right now.
        ///
        /// While an IME composition is in progress this is the composition buffer; otherwise it
        /// is the node's edit buffer, then its canonical text, and empty for a write-only control
        /// without an edit buffer. It is derived through a memo that subscribes to the node, so
        /// the first render after a transition already sees the new text.
        pub value: ReadSignal<String>,
        /// Applies the widget's current text.
        ///
        /// While composing, the text is buffered locally and no core operation runs. Otherwise
        /// it is applied through [`ControlActions::input_text`]; a failure is reported to
        /// `SchemaForm::on_error` and the widget's DOM value is resynchronised to the canonical
        /// text.
        pub input: Callback<String>,
        /// Starts an IME composition: subsequent `input` calls buffer until `composition_end`.
        pub composition_start: Callback<()>,
        /// Ends an IME composition and applies the buffered text.
        ///
        /// A composition started before the form was reset or reinitialized is discarded.
        pub composition_end: Callback<()>,
        /// Finishes any composition, then marks the control touched through
        /// [`ControlActions::blur`].
        pub blur: Callback<()>,
        /// Whether the widget should reject text input right now.
        ///
        /// True while the node is read-only or the core does not currently accept text input,
        /// matching [`render::ControlFacets::read_only`](crate::render::ControlFacets::read_only)
        /// for text controls.
        pub read_only: bool,
    }

    /// One in-flight IME composition: the lifecycle it started under and its current text.
    #[derive(Clone, PartialEq)]
    struct Composition {
        lifecycle: u64,
        text: String,
    }

    impl fmt::Debug for TextEdit {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The value signal is owned by the component that called the hook; a handle that
            // outlives it must still be printable.
            let value = self.value.try_peek().ok().map(|value| value.clone());
            formatter
                .debug_struct("TextEdit")
                .field("value", &value)
                .field("read_only", &self.read_only)
                .finish_non_exhaustive()
        }
    }

    /// Owns the built-in text-editing behaviour for the control behind `context`.
    ///
    /// This is a Dioxus hook: call it unconditionally, in a stable order, inside the renderer's
    /// own child component. [`render::ControlRenderer::render`](crate::render::ControlRenderer)
    /// itself is not a hook-safe call site.
    ///
    /// The returned [`TextEdit`] reproduces the built-in string, number, and integer control
    /// exactly: input while composing is buffered locally and committed when the composition
    /// ends; a form reset or reinitialization discards an in-flight composition and
    /// resynchronises the widget; a rejected write is reported to `SchemaForm::on_error` and the
    /// widget's DOM value is restored to the canonical text.
    ///
    /// ```rust,no_run
    /// use dioxus::prelude::*;
    /// use schemaform_dioxus::{ControlRenderContext, ControlRenderer, use_text_edit};
    ///
    /// struct PlainTextRenderer;
    ///
    /// impl ControlRenderer for PlainTextRenderer {
    ///     fn render(&self, context: ControlRenderContext) -> Element {
    ///         // Hooks belong in the renderer's own component, not in `render` itself.
    ///         rsx! { PlainTextControl { context } }
    ///     }
    /// }
    ///
    /// #[component]
    /// fn PlainTextControl(context: ControlRenderContext) -> Element {
    ///     let edit = use_text_edit(&context);
    ///     let presentation = context.presentation();
    ///     let control = context.control();
    ///     rsx! {
    ///         label { r#for: presentation.element_id.clone(), "{presentation.label}" }
    ///         input {
    ///             id: presentation.element_id.clone(),
    ///             name: control.name.clone(),
    ///             value: edit.value,
    ///             readonly: edit.read_only,
    ///             required: control.required,
    ///             "aria-invalid": presentation.invalid,
    ///             "aria-describedby": presentation.described_by(),
    ///             oninput: move |event| edit.input.call(event.value()),
    ///             oncompositionstart: move |_| edit.composition_start.call(()),
    ///             oncompositionend: move |_| edit.composition_end.call(()),
    ///             onblur: move |_| edit.blur.call(()),
    ///         }
    ///         {presentation.present_help()}
    ///         {presentation.present_findings()}
    ///     }
    /// }
    /// ```
    pub fn use_text_edit(context: &ControlRenderContext) -> TextEdit {
        let reader = context.node().clone();
        let composition = use_signal(|| None::<Composition>);
        // The canonical text tracks the node through this memo, so the memo rather than the
        // calling component subscribes to it. `None` means the node cannot be read right now.
        let canonical = {
            let reader = reader.clone();
            use_memo(move || canonical_text_of(reader.read()))
        };
        let target = Rc::new(TextEditTarget {
            node: EditTarget::new(context),
            composition,
            canonical,
        });

        let value_memo = {
            let target = target.clone();
            use_memo(move || {
                let lifecycle = target.handle().observe_lifecycle();
                let canonical = target.canonical.read().clone().unwrap_or_default();
                match &*target.composition.read() {
                    Some(current) if current.lifecycle == lifecycle => current.text.clone(),
                    _ => canonical,
                }
            })
        };
        let value = use_hook(|| ReadSignal::new(value_memo));

        // Discard a composition that began under an earlier lifecycle once the new lifecycle has
        // rendered, and put the canonical text back into the widget the composition had filled.
        {
            let target = target.clone();
            use_effect(move || {
                let lifecycle = target.handle().observe_lifecycle();
                if target.composition_is_stale(lifecycle) {
                    target.discard_composition();
                    target.resynchronize();
                }
            });
        }

        let input = {
            let target = target.clone();
            use_callback(move |text: String| target.input(text))
        };
        let composition_start = {
            let target = target.clone();
            use_callback(move |()| target.start_composition())
        };
        let composition_end = {
            let target = target.clone();
            use_callback(move |()| target.finish_composition())
        };
        let blur = use_callback(move |()| {
            target.finish_composition();
            target.node.blur();
        });

        TextEdit {
            value,
            input,
            composition_start,
            composition_end,
            blur,
            read_only: context.control().read_only,
        }
    }

    /// What every edit hook shares about the control it edits: the node, its approved actions,
    /// the route to the host's `on_error`, and the widget the hook resynchronises.
    struct EditTarget {
        reader: NodeReader,
        actions: ControlActions,
        error_route: Option<crate::OperationErrorHandler>,
        element_id: String,
    }

    impl EditTarget {
        fn new(context: &ControlRenderContext) -> Self {
            Self {
                reader: context.node().clone(),
                actions: context.actions().clone(),
                error_route: context.error_route().clone(),
                element_id: context.presentation().element_id.clone(),
            }
        }

        /// Reads the node without subscribing, for event handlers.
        fn read_untracked(&self) -> Result<Option<NodeProjection>, HandleError> {
            self.reader.read_untracked()
        }

        /// Routes a failed operation to the host and reports whether it succeeded.
        fn report<T>(&self, result: Result<T, HandleError>) -> bool {
            crate::report_operation(&self.error_route, result)
        }

        /// Marks the control touched.
        fn blur(&self) {
            self.report(self.actions.blur());
        }

        /// Writes a concrete value the way the built-ins do: replacing when the core allows
        /// replacement right now (incompatible data, or a write-only control), otherwise setting.
        fn set_or_replace(
            &self,
            value: Value,
            operations: Option<AllowedOperations>,
        ) -> Result<schemaform::Transition, HandleError> {
            if operations.is_some_and(|operations| operations.can_replace_value()) {
                self.actions.replace_value(value)
            } else {
                self.actions.set_value(value)
            }
        }
    }

    /// The node one [`use_text_edit`] call edits, with the state its callbacks share.
    struct TextEditTarget {
        node: EditTarget,
        composition: Signal<Option<Composition>>,
        /// Canonical display text tracked through the node; `None` while the node is unreadable.
        canonical: Memo<Option<String>>,
    }

    impl TextEditTarget {
        fn handle(&self) -> &FormHandle {
            self.node.reader.handle()
        }

        /// The current lifecycle, read without subscribing, for event handlers.
        fn lifecycle(&self) -> u64 {
            self.handle().peek_lifecycle()
        }

        fn composition_is_stale(&self, lifecycle: u64) -> bool {
            self.composition
                .peek()
                .as_ref()
                .is_some_and(|current| current.lifecycle != lifecycle)
        }

        fn discard_composition(&self) {
            let mut composition = self.composition;
            composition.set(None);
        }

        /// The canonical text to put back into the widget: a fresh read, or the last rendered
        /// text when the form cannot be read right now, for example while a host transaction
        /// holds the borrow that also rejected the write.
        fn canonical_text(&self) -> String {
            canonical_text_of(self.node.read_untracked())
                .or_else(|| self.canonical.peek().clone())
                .unwrap_or_default()
        }

        fn resynchronize(&self) {
            crate::resynchronize_control_value(&self.node.element_id, &self.canonical_text());
        }

        /// Buffers `text` while composing; otherwise applies it through the core and restores
        /// the widget when the core rejects it.
        fn input(&self, text: String) {
            let lifecycle = self.lifecycle();
            let composing = self
                .composition
                .peek()
                .as_ref()
                .is_some_and(|current| current.lifecycle == lifecycle);
            if composing {
                let mut composition = self.composition;
                composition.set(Some(Composition { lifecycle, text }));
            } else {
                self.apply_text(&text);
            }
        }

        fn apply_text(&self, text: &str) {
            if !self.node.report(self.node.actions.input_text(text)) {
                self.resynchronize();
            }
        }

        /// Starts a composition seeded with the canonical text, so `value` is unchanged until
        /// the first composed input arrives.
        fn start_composition(&self) {
            let mut composition = self.composition;
            composition.set(Some(Composition {
                lifecycle: self.lifecycle(),
                text: self.canonical_text(),
            }));
        }

        /// Takes the in-flight composition and applies its text if it belongs to the current
        /// lifecycle.
        fn finish_composition(&self) {
            let Some(current) = self.composition.peek().clone() else {
                return;
            };
            self.discard_composition();
            if current.lifecycle == self.lifecycle() {
                self.apply_text(&current.text);
            }
        }
    }

    /// The display text of a node read, or `None` when the node could not be read.
    fn canonical_text_of(read: Result<Option<NodeProjection>, HandleError>) -> Option<String> {
        read.ok()
            .flatten()
            .map(|projection| display_text(&projection))
    }

    /// Headless editing behaviour for one boolean control.
    ///
    /// Obtained from [`use_boolean_edit`]. The callbacks keep their identity across renders and
    /// `checked` is a read signal, so a widget that receives this value as a prop stays wired to
    /// the live control without re-rendering per edit.
    ///
    /// Two values compare equal when they come from the same hook call site; `checked`'s current
    /// value is not compared. The struct is non-exhaustive so later releases can add fields
    /// without breaking renderers; it is only ever constructed by the hook.
    #[derive(Clone, Copy, PartialEq)]
    #[non_exhaustive]
    pub struct BooleanEdit {
        /// The tri-state the widget should display right now.
        ///
        /// `Some(true)` or `Some(false)` while the node's current data is a JSON boolean and
        /// `None` while it is null. A missing or incompatible value reads as `Some(false)`: there
        /// is no boolean to show, and the built-in checkbox renders it unchecked beside its
        /// value state and repair affordances. A write-only control always reads as `None`; its
        /// value is never echoed. The signal is derived through a memo that subscribes to the
        /// node, so the first render after a transition already sees the new state.
        pub checked: ReadSignal<Option<bool>>,
        /// Applies the widget's new state.
        ///
        /// `None` sets the value to JSON null through [`ControlActions::set_null`]. `Some` reads
        /// the operations the core allows at event time and replaces the value through
        /// [`ControlActions::replace_value`] when replacement is allowed (incompatible data, or a
        /// write-only control), otherwise sets it through [`ControlActions::set_value`]. A
        /// failure is reported to `SchemaForm::on_error` and the widget carrying the node's
        /// element id is resynchronised to `checked`; a write-only control is resynchronised
        /// after every call so the widget never shows the value it just wrote. Resynchronisation
        /// sets a checkbox's `checked` property, or a `select`'s `value` to `"true"`, `"false"`,
        /// or `""` for `None`.
        pub set: Callback<Option<bool>>,
        /// Marks the control touched through [`ControlActions::blur`].
        pub blur: Callback<()>,
    }

    impl fmt::Debug for BooleanEdit {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The signal is owned by the component that called the hook; a handle that outlives
            // it must still be printable.
            let checked = self.checked.try_peek().ok().map(|checked| *checked);
            formatter
                .debug_struct("BooleanEdit")
                .field("checked", &checked)
                .finish_non_exhaustive()
        }
    }

    /// What a boolean widget needs from one node read: the tri-state to show and the operations
    /// the core allows.
    #[derive(Clone, Copy, PartialEq)]
    struct BooleanState {
        checked: Option<bool>,
        operations: AllowedOperations,
        write_only: bool,
    }

    /// Owns the built-in boolean-editing behaviour for the control behind `context`.
    ///
    /// This is a Dioxus hook: call it unconditionally, in a stable order, inside the renderer's
    /// own child component. [`render::ControlRenderer::render`](crate::render::ControlRenderer)
    /// itself is not a hook-safe call site.
    ///
    /// The returned [`BooleanEdit`] reproduces the built-in checkbox and write-only replacement
    /// select exactly: `set` chooses between set null, set value, and replace value from the
    /// operations the core allows when the event fires, reports a rejected write to
    /// `SchemaForm::on_error`, and restores the widget to the node's state.
    ///
    /// ```rust,no_run
    /// use dioxus::prelude::*;
    /// use schemaform_dioxus::{ControlRenderContext, ControlRenderer, use_boolean_edit};
    ///
    /// struct CheckboxRenderer;
    ///
    /// impl ControlRenderer for CheckboxRenderer {
    ///     fn render(&self, context: ControlRenderContext) -> Element {
    ///         rsx! { Checkbox { context } }
    ///     }
    /// }
    ///
    /// #[component]
    /// fn Checkbox(context: ControlRenderContext) -> Element {
    ///     let edit = use_boolean_edit(&context);
    ///     let presentation = context.presentation();
    ///     let control = context.control();
    ///     rsx! {
    ///         input {
    ///             id: presentation.element_id.clone(),
    ///             name: control.name.clone(),
    ///             r#type: "checkbox",
    ///             checked: edit.checked.read().unwrap_or(false),
    ///             disabled: control.disabled,
    ///             "aria-required": control.required,
    ///             "aria-invalid": presentation.invalid,
    ///             "aria-describedby": presentation.described_by(),
    ///             oninput: move |event| edit.set.call(Some(event.checked())),
    ///             onblur: move |_| edit.blur.call(()),
    ///         }
    ///         label { r#for: presentation.element_id.clone(), "{presentation.label}" }
    ///         {presentation.present_help()}
    ///         {presentation.present_findings()}
    ///     }
    /// }
    /// ```
    pub fn use_boolean_edit(context: &ControlRenderContext) -> BooleanEdit {
        // The state tracks the node through this memo, so the memo rather than the calling
        // component subscribes to it. `None` means the node cannot be read right now.
        let state = {
            let reader = context.node().clone();
            use_memo(move || boolean_state_of(reader.read()))
        };
        let checked_memo = use_memo(move || state.read().as_ref().and_then(|state| state.checked));
        let checked = use_hook(|| ReadSignal::new(checked_memo));
        let target = Rc::new(BooleanEditTarget {
            node: EditTarget::new(context),
            state,
        });

        let set = {
            let target = target.clone();
            use_callback(move |value: Option<bool>| target.set(value))
        };
        let blur = use_callback(move |()| target.node.blur());

        BooleanEdit { checked, set, blur }
    }

    /// The node one [`use_boolean_edit`] call edits, with the state its callbacks share.
    struct BooleanEditTarget {
        node: EditTarget,
        /// State tracked through the node; `None` while the node is unreadable.
        state: Memo<Option<BooleanState>>,
    }

    impl BooleanEditTarget {
        /// The state to decide and resynchronise against: a fresh read, or the last rendered
        /// state when the form cannot be read right now, for example while a host transaction
        /// holds the borrow that also rejects the write.
        fn current_state(&self) -> Option<BooleanState> {
            boolean_state_of(self.node.read_untracked()).or_else(|| *self.state.peek())
        }

        fn set(&self, value: Option<bool>) {
            let state = self.current_state();
            let result = match value {
                None => self.node.actions.set_null(),
                Some(value) => self
                    .node
                    .set_or_replace(Value::Bool(value), state.map(|state| state.operations)),
            };
            let succeeded = self.node.report(result);
            if !succeeded || state.is_some_and(|state| state.write_only) {
                crate::resynchronize_boolean(
                    &self.node.element_id,
                    state.and_then(|state| state.checked),
                );
            }
        }
    }

    /// The boolean state of a node read, or `None` when the node could not be read.
    fn boolean_state_of(read: Result<Option<NodeProjection>, HandleError>) -> Option<BooleanState> {
        read.ok().flatten().map(|projection| BooleanState {
            checked: if projection.write_only {
                None
            } else {
                displayed_boolean(projection.current_data.as_ref())
            },
            operations: projection.allowed_operations,
            write_only: projection.write_only,
        })
    }

    /// The tri-state a boolean widget displays for `data`: the boolean itself, `None` for null,
    /// and unchecked for a missing or incompatible value, as the built-in checkbox shows it.
    fn displayed_boolean(data: Option<&Value>) -> Option<bool> {
        match data {
            Some(Value::Bool(value)) => Some(*value),
            Some(Value::Null) => None,
            _ => Some(false),
        }
    }

    /// Headless editing behaviour for one choice control.
    ///
    /// Obtained from [`use_choice_edit`]. The callbacks keep their identity across renders and
    /// `selected` is a read signal, so a widget that receives them as props stays wired to the
    /// live control; `options` is a plain list the widget renders.
    ///
    /// Two values compare equal when they come from the same hook call site and their `options`
    /// are equal; `selected`'s current value is not compared. The struct is non-exhaustive so
    /// later releases can add fields without breaking renderers; it is only ever constructed by
    /// the hook.
    #[derive(Clone, PartialEq)]
    #[non_exhaustive]
    pub struct ChoiceEdit {
        /// The option the widget should show as selected right now.
        ///
        /// `None` while the node's current data matches no option (missing or incompatible
        /// data) and always for a write-only control, whose value is never echoed. It is derived
        /// through a memo that subscribes to the node, so the first render after a transition
        /// already sees the new selection.
        pub selected: ReadSignal<Option<ChoiceIdentity>>,
        /// The selectable options in the core's compiled order (the null option first), with
        /// localized labels.
        pub options: Vec<ChoiceOption>,
        /// Applies the widget's selection.
        ///
        /// Selecting the null option sets the value to JSON null through
        /// [`ControlActions::set_null`]. Selecting another option reads the operations the core
        /// allows at event time and replaces the value through [`ControlActions::replace_value`]
        /// when replacement is allowed (incompatible data, or a write-only control), otherwise
        /// sets it through [`ControlActions::set_value`]. Reselecting the current option, `None`,
        /// and an identity that is not among `options` run no core operation. A failure is
        /// reported to `SchemaForm::on_error`. Whenever no core operation changed the value, and
        /// after every call for a write-only control, the widget carrying the node's element id
        /// has its `value` property restored to the selected identity (or `""`), so a native
        /// `select` stays in step with the node.
        pub select: Callback<Option<ChoiceIdentity>>,
        /// Marks the control touched through [`ControlActions::blur`].
        pub blur: Callback<()>,
    }

    impl fmt::Debug for ChoiceEdit {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The signal is owned by the component that called the hook; a handle that outlives
            // it must still be printable.
            let selected = self
                .selected
                .try_peek()
                .ok()
                .map(|selected| selected.clone());
            formatter
                .debug_struct("ChoiceEdit")
                .field("selected", &selected)
                .field("options", &self.options)
                .finish_non_exhaustive()
        }
    }

    /// One selectable option of a choice control, as a widget should present it.
    ///
    /// Options are compiled from the definition, so their identities and order are fixed for the
    /// lifetime of the bound form; `label` follows the configured localizer and `disabled`
    /// follows the operations the core allows right now. The struct is non-exhaustive and only
    /// ever constructed by [`use_choice_edit`].
    #[derive(Debug, Clone, PartialEq)]
    #[non_exhaustive]
    pub struct ChoiceOption {
        /// Opaque identity to hand back to [`ChoiceEdit::select`]; its
        /// [`ChoiceIdentity::as_str`] form is a safe DOM value.
        pub identity: ChoiceIdentity,
        /// Localized plain-text label.
        pub label: String,
        /// Whether this option selects JSON null.
        pub is_null: bool,
        /// Whether selecting this option right now would be rejected by the core: the null
        /// option while the core does not allow set null, another option while it allows neither
        /// set value nor replace value. The currently selected option is never disabled.
        pub disabled: bool,
    }

    /// What a choice widget needs from one node read.
    #[derive(Clone, PartialEq)]
    struct ChoiceState {
        selected: Option<ChoiceIdentity>,
        entries: Vec<ChoiceEntry>,
        operations: AllowedOperations,
        write_only: bool,
    }

    /// One option as the widget presents it, with the canonical value selecting it writes.
    #[derive(Clone, PartialEq)]
    struct ChoiceEntry {
        option: ChoiceOption,
        value: Value,
    }

    impl ChoiceState {
        fn options(&self) -> Vec<ChoiceOption> {
            self.entries
                .iter()
                .map(|entry| entry.option.clone())
                .collect()
        }
    }

    /// Owns the built-in choice-editing behaviour for the control behind `context`.
    ///
    /// This is a Dioxus hook: call it unconditionally, in a stable order, inside the renderer's
    /// own child component. [`render::ControlRenderer::render`](crate::render::ControlRenderer)
    /// itself is not a hook-safe call site.
    ///
    /// The returned [`ChoiceEdit`] reproduces the built-in select and its write-only replacement
    /// variant exactly: `select` maps an opaque option identity to set null, set value, or replace
    /// value from the operations the core allows when the event fires, treats reselection as a
    /// no-op, reports a rejected write to `SchemaForm::on_error`, and restores the widget to the
    /// node's selection. The same handles drive a radio group or a combobox.
    ///
    /// ```rust,no_run
    /// use dioxus::prelude::*;
    /// use schemaform_dioxus::{ControlRenderContext, ControlRenderer, use_choice_edit};
    ///
    /// struct SelectRenderer;
    ///
    /// impl ControlRenderer for SelectRenderer {
    ///     fn render(&self, context: ControlRenderContext) -> Element {
    ///         rsx! { Select { context } }
    ///     }
    /// }
    ///
    /// #[component]
    /// fn Select(context: ControlRenderContext) -> Element {
    ///     let edit = use_choice_edit(&context);
    ///     let presentation = context.presentation();
    ///     let control = context.control();
    ///     let selected = edit.selected.read().clone();
    ///     let options = edit.options.clone();
    ///     let lookup = edit.options.clone();
    ///     rsx! {
    ///         label { r#for: presentation.element_id.clone(), "{presentation.label}" }
    ///         select {
    ///             id: presentation.element_id.clone(),
    ///             name: control.name.clone(),
    ///             value: selected.as_ref().map(|identity| identity.as_str().to_owned()),
    ///             disabled: control.disabled,
    ///             required: control.required,
    ///             "aria-invalid": presentation.invalid,
    ///             "aria-describedby": presentation.described_by(),
    ///             onchange: move |event| {
    ///                 let identity = lookup
    ///                     .iter()
    ///                     .find(|option| option.identity.as_str() == event.value())
    ///                     .map(|option| option.identity.clone());
    ///                 edit.select.call(identity);
    ///             },
    ///             onblur: move |_| edit.blur.call(()),
    ///             for option in options {
    ///                 option {
    ///                     value: option.identity.as_str().to_owned(),
    ///                     selected: Some(&option.identity) == selected.as_ref(),
    ///                     disabled: option.disabled,
    ///                     "{option.label}"
    ///                 }
    ///             }
    ///         }
    ///         {presentation.present_help()}
    ///         {presentation.present_findings()}
    ///     }
    /// }
    /// ```
    pub fn use_choice_edit(context: &ControlRenderContext) -> ChoiceEdit {
        // The state tracks the node and the localizer through this memo, so the memo rather
        // than the calling component subscribes to them. `None` means the node cannot be read
        // right now.
        let state = {
            let reader = context.node().clone();
            let form = context.presentation().form().clone();
            use_memo(move || {
                choice_state_of(reader.read(), |label| {
                    crate::localize_text(&form, None, label)
                })
            })
        };
        let selected_memo = use_memo(move || {
            state
                .read()
                .as_ref()
                .and_then(|state| state.selected.clone())
        });
        let selected = use_hook(|| ReadSignal::new(selected_memo));
        // The calling component subscribes to the options alone, so it re-renders when a label
        // or an option's availability changes, not on every node change.
        let options = use_memo(move || {
            state
                .read()
                .as_ref()
                .map(ChoiceState::options)
                .unwrap_or_default()
        })
        .read()
        .clone();
        let target = Rc::new(ChoiceEditTarget {
            node: EditTarget::new(context),
            state,
        });

        let select = {
            let target = target.clone();
            use_callback(move |identity: Option<ChoiceIdentity>| target.select(identity))
        };
        let blur = use_callback(move |()| target.node.blur());

        ChoiceEdit {
            selected,
            options,
            select,
            blur,
        }
    }

    /// The node one [`use_choice_edit`] call edits, with the state its callbacks share.
    struct ChoiceEditTarget {
        node: EditTarget,
        /// State tracked through the node; `None` while the node is unreadable.
        state: Memo<Option<ChoiceState>>,
    }

    impl ChoiceEditTarget {
        /// The state to decide and resynchronise against: a fresh read, or the last rendered
        /// state when the form cannot be read right now, for example while a host transaction
        /// holds the borrow that also rejects the write. Decisions never read labels, so a fresh
        /// read leaves them unlocalized.
        fn current_state(&self) -> Option<ChoiceState> {
            choice_state_of(self.node.read_untracked(), str::to_owned)
                .or_else(|| self.state.peek().clone())
        }

        fn select(&self, identity: Option<ChoiceIdentity>) {
            let Some(state) = self.current_state() else {
                return;
            };
            let Some(entry) = identity.and_then(|identity| {
                state
                    .entries
                    .iter()
                    .find(|entry| entry.option.identity == identity)
            }) else {
                self.resynchronize(&state);
                return;
            };
            if !state.write_only && state.selected.as_ref() == Some(&entry.option.identity) {
                self.resynchronize(&state);
                return;
            }
            let result = if entry.option.is_null {
                self.node.actions.set_null()
            } else {
                self.node
                    .set_or_replace(entry.value.clone(), Some(state.operations))
            };
            let succeeded = self.node.report(result);
            if state.write_only || !succeeded {
                self.resynchronize(&state);
            }
        }

        fn resynchronize(&self, state: &ChoiceState) {
            let selected = state.selected.as_ref().map_or("", ChoiceIdentity::as_str);
            crate::resynchronize_control_value(&self.node.element_id, selected);
        }
    }

    /// The choice state of a node read, with labels passed through `localize`, or `None` when the
    /// node could not be read.
    fn choice_state_of(
        read: Result<Option<NodeProjection>, HandleError>,
        mut localize: impl FnMut(&str) -> String,
    ) -> Option<ChoiceState> {
        let projection = read.ok().flatten()?;
        let operations = projection.allowed_operations;
        let write_only = projection.write_only;
        let selected = (!write_only)
            .then(|| {
                projection
                    .choice_options
                    .iter()
                    .find(|option| option.selected)
                    .map(|option| option.identity.clone())
            })
            .flatten();
        let entries = projection
            .choice_options
            .iter()
            .map(|option| {
                let is_null = option.value.is_null();
                let current = !write_only && option.selected;
                let allowed = if is_null {
                    operations.can_set_null()
                } else {
                    operations.can_set_value() || operations.can_replace_value()
                };
                ChoiceEntry {
                    option: ChoiceOption {
                        identity: option.identity.clone(),
                        label: localize(&option.label),
                        is_null,
                        disabled: !current && !allowed,
                    },
                    value: option.value.clone(),
                }
            })
            .collect();
        Some(ChoiceState {
            selected,
            entries,
            operations,
            write_only,
        })
    }

    /// Display text for a projected text control, as the built-in control shows it.
    pub(crate) fn display_text(projection: &NodeProjection) -> String {
        if projection.write_only && projection.edit_buffer.is_none() {
            return String::new();
        }
        projection.value.clone().unwrap_or_else(|| {
            projection
                .current_data
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_default()
        })
    }
}

/// Route from adapter operations to the host's `SchemaForm::on_error`.
///
/// One handler is provided as context per mounted [`SchemaForm`] and shared by the built-ins,
/// presence affordances, and [`render::ControlRenderContext::report`]. It compares by identity.
#[derive(Clone, Default)]
struct OperationErrorHandler(Rc<RefCell<Option<EventHandler<handle::HandleError>>>>);

impl PartialEq for OperationErrorHandler {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl OperationErrorHandler {
    fn set(&self, handler: Option<EventHandler<handle::HandleError>>) {
        *self.0.borrow_mut() = handler;
    }

    fn report(&self, error: handle::HandleError) {
        let handler = *self.0.borrow();
        if let Some(handler) = handler {
            handler.call(error);
        }
    }
}

/// Routes a failed operation to `handler` and returns the success value.
///
/// `handler` is `None` when the operation runs outside a mounted [`SchemaForm`]; the error is
/// then dropped, matching the documented behaviour of an unset `on_error`.
fn route_operation<T>(
    handler: &Option<OperationErrorHandler>,
    result: Result<T, handle::HandleError>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            if let Some(handler) = handler {
                handler.report(error);
            }
            None
        }
    }
}

/// [`route_operation`] for built-ins that only branch on success.
fn report_operation<T>(
    handler: &Option<OperationErrorHandler>,
    result: Result<T, handle::HandleError>,
) -> bool {
    route_operation(handler, result).is_some()
}

/// Properties for the browser-CSR [`SchemaForm`] component.
///
/// Callbacks run synchronously after the adapter operation has released its form borrow. They may
/// start host-owned asynchronous work, but transport, retries, and pending/success lifecycle are
/// not managed by this component.
#[derive(Props, Clone, PartialEq)]
pub struct SchemaFormProps {
    /// Preflighted single-mount form plan to render.
    pub form: render::BoundForm,
    /// Receives only immutable snapshots that passed submission preparation.
    ///
    /// Blocked submissions do not call this callback; they update finding presentation and focus.
    pub on_submit: EventHandler<SubmissionSnapshot>,
    /// Receives adapter operation failures, including reentrant handle borrow conflicts.
    ///
    /// This callback is optional; when it is not set, failures are dropped. Failures are never
    /// converted into submission blockers.
    #[props(default)]
    pub on_error: EventHandler<handle::HandleError>,
}

#[allow(non_snake_case)]
/// Renders one bound browser-CSR form.
///
/// The component supports client-side browser rendering only, not SSR, hydration, or desktop and
/// WebView targets. A [`render::BoundForm`] and its clones share generated DOM identity and must
/// have at most one concurrent mount. Submission calls `on_submit` only for a ready
/// [`SubmissionSnapshot`]; blocked outcomes update findings and focus, while adapter failures call
/// `on_error`. Built-ins emit semantic accessibility markup; a custom control renderer owns its
/// whole control region and is responsible for emitting the elements its
/// [`render::NodePresentation`] references.
pub fn SchemaForm(props: SchemaFormProps) -> Element {
    let operation_errors = use_context_provider(OperationErrorHandler::default);
    operation_errors.set(Some(props.on_error));
    let submit_form = props.form.clone();
    let summary_focus =
        render::TargetFocusAction::new(format!("{}-summary", props.form.inner.form_id));
    let on_submit = props.on_submit;
    let on_error = props.on_error;
    let controls = props.form.render();
    let submit_label = localize_builtin(&props.form, BuiltinMessage::Submit);
    let mut grid_styles = format!(
        "#{id} .schemaform-grid{{display:grid;grid-template-columns:repeat(12,minmax(0,1fr))}}",
        id = props.form.inner.form_id,
    );
    for span in 1..=12 {
        grid_styles.push_str(&format!(
            "#{id} .schemaform-grid-cell[data-compact-span='{span}']{{grid-column:span {span} / span {span}}}",
            id = props.form.inner.form_id,
        ));
    }
    grid_styles.push_str(&format!(
        "@media (min-width:{}px){{",
        props.form.inner.grid_wide_breakpoint_css_px
    ));
    for span in 1..=12 {
        grid_styles.push_str(&format!(
            "#{id} .schemaform-grid-cell[data-wide-span='{span}']{{grid-column:span {span} / span {span}}}",
            id = props.form.inner.form_id,
        ));
    }
    grid_styles.push('}');

    rsx! {
        style { dangerous_inner_html: grid_styles }
        form {
            id: props.form.inner.form_id.clone(),
            class: "schemaform",
            "data-schemaform": "",
            novalidate: true,
            tabindex: "-1",
            onsubmit: move |event| {
                event.prevent_default();
                match submit_form.handle().prepare_submission() {
                    Ok(preparation) => match preparation.into_parts().1 {
                        SubmissionOutcome::Ready(snapshot) => on_submit.call(snapshot),
                        SubmissionOutcome::Blocked(_) => summary_focus.focus(),
                    },
                    Err(error) => on_error.call(error),
                }
            },
            {controls}
            button { r#type: "submit", "{submit_label}" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct BoundControlsProps {
    form: render::BoundForm,
}

#[allow(non_snake_case)]
fn BoundControls(props: BoundControlsProps) -> Element {
    if props.form.handle().ensure_live().is_err() {
        return rsx! {};
    }
    let projection = props.form.handle().summary_projection();
    let root = projection.root;
    let mut targets = vec![(
        root,
        FocusTarget {
            element_id: props.form.inner.form_id.clone(),
            tab_ids: Vec::new(),
        },
    )];
    collect_focus_targets(&props.form, &props.form.inner.nodes, &[], &mut targets);
    let target_ids = targets
        .iter()
        .map(|(identity, target)| (*identity, target.clone()))
        .collect::<HashMap<_, _>>();
    let summary_findings = projection
        .findings
        .iter()
        .map(|finding| {
            let target = match finding {
                handle::FindingProjection::Validation { target, .. }
                | handle::FindingProjection::ValidationFindingsTruncated { target, .. }
                | handle::FindingProjection::Indeterminate { target, .. }
                | handle::FindingProjection::Capability { target, .. }
                | handle::FindingProjection::External { target, .. }
                | handle::FindingProjection::Parse { target, .. } => target_ids.get(target),
            }
            .cloned()
            .unwrap_or_else(|| FocusTarget {
                element_id: props.form.inner.form_id.clone(),
                tab_ids: Vec::new(),
            });
            let stable_id = summary_finding_stable_id(&props.form.inner.form_id, finding);
            (
                finding_descriptor(&props.form, finding, stable_id),
                render::TargetFocusAction::new(target.element_id).activate_tabs(target.tab_ids),
            )
        })
        .collect();
    let summary_context = render::FindingCollectionContext::summary(summary_findings);
    let summary_label = localize_builtin(&props.form, BuiltinMessage::FindingSummary);

    rsx! {
        div {
            id: format!("{}-summary", props.form.inner.form_id),
            "data-finding-summary": "",
            role: "region",
            "aria-label": summary_label,
            tabindex: "-1",
            FindingCollectionPresentation {
                form: props.form.clone(),
                context: summary_context,
            }
        }
        for node in props.form.inner.nodes.iter().cloned() {
            BoundNode {
                key: "{node.key()}",
                form: props.form.clone(),
                node,
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct FindingCollectionPresentationProps {
    form: render::BoundForm,
    context: render::FindingCollectionContext,
}

#[allow(non_snake_case)]
fn FindingCollectionPresentation(props: FindingCollectionPresentationProps) -> Element {
    let presenter = if props.context.is_summary() {
        props.form.inner.summary_presenter
    } else {
        props.form.inner.local_presenter
    };
    let presenter = presenter.read();
    presenter.render(props.context)
}

#[derive(Clone)]
struct FocusTarget {
    element_id: String,
    tab_ids: Vec<String>,
}

fn collect_focus_targets(
    form: &render::BoundForm,
    nodes: &[render::BoundNode],
    containing_tabs: &[String],
    targets: &mut Vec<(schemaform::InstanceIdentity, FocusTarget)>,
) {
    let push_target = |targets: &mut Vec<_>, identity, element_id: &str| {
        targets.push((
            identity,
            FocusTarget {
                element_id: element_id.to_owned(),
                tab_ids: containing_tabs.to_vec(),
            },
        ));
    };
    for node in nodes {
        match node {
            render::BoundNode::Decorated(decorated) => {
                collect_focus_targets(
                    form,
                    std::slice::from_ref(decorated.child.as_ref()),
                    containing_tabs,
                    targets,
                );
            }
            render::BoundNode::Control(control) => {
                push_target(targets, control.identity, &control.input_id);
            }
            render::BoundNode::Group(group) => {
                push_target(targets, group.identity, &group.element_id);
                collect_focus_targets(form, &group.children, containing_tabs, targets);
            }
            render::BoundNode::Stack(stack) => {
                collect_focus_targets(form, &stack.children, containing_tabs, targets);
            }
            render::BoundNode::Grid(grid) => {
                collect_focus_targets(form, &grid.cells, containing_tabs, targets);
            }
            render::BoundNode::GridCell(cell) => {
                collect_focus_targets(form, &cell.children, containing_tabs, targets);
            }
            render::BoundNode::AuthoredGroup(group) => {
                collect_focus_targets(form, &group.children, containing_tabs, targets);
            }
            render::BoundNode::Tabs(tabs) => {
                for (index, panel) in tabs.panels.iter().enumerate() {
                    let mut panel_tabs = containing_tabs.to_vec();
                    panel_tabs.push(format!("{}-tab-{index}", tabs.element_id));
                    collect_focus_targets(form, std::slice::from_ref(panel), &panel_tabs, targets);
                }
            }
            render::BoundNode::TabPanel(panel) => {
                push_target(targets, panel.identity, &panel.element_id);
                collect_focus_targets(form, &panel.children, containing_tabs, targets);
            }
            render::BoundNode::Text(_) => {}
            render::BoundNode::Array(array) => {
                push_target(targets, array.identity, &array.element_id);
                if let Some(rows) = form
                    .handle()
                    .node(array.identity)
                    .ok()
                    .flatten()
                    .and_then(|reader| reader.read_untracked().ok().flatten())
                    .map(|projection| projection.children)
                {
                    for identity in rows {
                        if let Some(node) = instantiate_array_template(
                            form,
                            &array.template,
                            identity,
                            &array.element_id,
                        ) {
                            collect_focus_targets(form, &[node], containing_tabs, targets);
                        }
                    }
                }
            }
            render::BoundNode::Unsupported(region) => {
                push_target(targets, region.identity, &region.element_id);
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct BoundNodeProps {
    form: render::BoundForm,
    node: render::BoundNode,
}

#[cfg(schemaform_test_validation_faults)]
fn observe_renderer_entry(
    form: &render::BoundForm,
    identity: schemaform::InstanceIdentity,
    node_kind: render::RenderNodeKind,
    dom_id: &str,
) {
    if let Some(observer) = &form.inner.observer {
        observer.observe(render::RenderObservation {
            event: render::RenderEvent::RendererEntered,
            identity,
            node_kind,
            dom_id: dom_id.to_owned(),
        });
    }
}

#[cfg(schemaform_test_validation_faults)]
fn use_observed_lifecycle(
    form: &render::BoundForm,
    observation: Option<(schemaform::InstanceIdentity, render::RenderNodeKind, String)>,
) {
    let mount_observer = form.inner.observer.clone();
    let mount_observation = observation.clone();
    use_hook(move || {
        if let (Some(observer), Some((identity, node_kind, dom_id))) =
            (mount_observer, mount_observation)
        {
            observer.observe(render::RenderObservation {
                event: render::RenderEvent::Mounted,
                identity,
                node_kind,
                dom_id,
            });
        }
    });
    let drop_observer = form.inner.observer.clone();
    use_drop(move || {
        if let (Some(observer), Some((identity, node_kind, dom_id))) = (drop_observer, observation)
        {
            observer.observe(render::RenderObservation {
                event: render::RenderEvent::Dropped,
                identity,
                node_kind,
                dom_id,
            });
        }
    });
}

#[allow(non_snake_case)]
fn BoundNode(props: BoundNodeProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    use_observed_lifecycle(&props.form, props.node.observation());
    #[cfg(schemaform_test_validation_faults)]
    if let render::BoundNode::Text(text) = &props.node {
        observe_renderer_entry(
            &props.form,
            text.identity,
            render::RenderNodeKind::StaticLayout,
            &text.element_id,
        );
    }
    match props.node {
        render::BoundNode::Decorated(decorated) => {
            let mut child = rsx! {
                BoundNode {
                    form: props.form,
                    node: *decorated.child,
                }
            };
            // Canonical ascending URI invocation makes the smallest URI innermost.
            for (namespace, extension) in decorated.extensions.iter() {
                child = extension.decorate(
                    render::ExtensionRenderContext::new(
                        decorated.definition,
                        decorated.identity,
                        namespace.clone(),
                    ),
                    child,
                );
            }
            child
        }
        render::BoundNode::Control(control) => rsx! {
            ControlHost {
                form: props.form,
                control,
            }
        },
        render::BoundNode::Group(group) => rsx! {
            FixedObjectGroup {
                form: props.form,
                group,
            }
        },
        render::BoundNode::Stack(stack) => rsx! {
            SemanticStack {
                form: props.form,
                stack,
            }
        },
        render::BoundNode::Grid(grid) => rsx! {
            SemanticGrid {
                form: props.form,
                grid,
            }
        },
        render::BoundNode::GridCell(cell) => rsx! {
            SemanticGridCell {
                form: props.form,
                cell,
            }
        },
        render::BoundNode::AuthoredGroup(group) => rsx! {
            AuthoredGroup {
                form: props.form,
                group,
            }
        },
        render::BoundNode::Tabs(tabs) => rsx! {
            SemanticTabs {
                form: props.form,
                tabs,
            }
        },
        render::BoundNode::TabPanel(_) => {
            unreachable!("tab panels are rendered only through their tabs component")
        }
        render::BoundNode::Text(text) => {
            let content = localize_ui_text(&props.form, &text.content);
            rsx! {
                p {
                    id: text.element_id,
                    class: "schemaform-text",
                    "data-schemaform-text": "",
                    "{content}"
                }
            }
        }
        render::BoundNode::Array(array) => rsx! {
            HomogeneousArray {
                form: props.form,
                array,
            }
        },
        render::BoundNode::Unsupported(region) => rsx! {
            UnsupportedRegion {
                form: props.form,
                region,
            }
        },
    }
}

#[derive(Props, Clone, PartialEq)]
struct SemanticStackProps {
    form: render::BoundForm,
    stack: render::BoundStack,
}

#[allow(non_snake_case)]
fn SemanticStack(props: SemanticStackProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.stack.identity,
        render::RenderNodeKind::StaticLayout,
        &props.stack.element_id,
    );
    if props.stack.transparent {
        return rsx! {
            div {
                id: props.stack.element_id,
                style: "display: contents",
                "data-schemaform-transparent-stack": "",
                for node in props.stack.children {
                    BoundNode {
                        key: "{node.key()}",
                        form: props.form.clone(),
                        node,
                    }
                }
            }
        };
    }
    rsx! {
        div {
            id: props.stack.element_id,
            class: "schemaform-stack",
            "data-schemaform-stack": "",
            for node in props.stack.children {
                BoundNode {
                    key: "{node.key()}",
                    form: props.form.clone(),
                    node,
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SemanticGridProps {
    form: render::BoundForm,
    grid: render::BoundGrid,
}

#[allow(non_snake_case)]
fn SemanticGrid(props: SemanticGridProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.grid.identity,
        render::RenderNodeKind::StaticLayout,
        &props.grid.element_id,
    );
    rsx! {
        div {
            id: props.grid.element_id,
            class: "schemaform-grid",
            "data-schemaform-grid": "",
            for cell in props.grid.cells {
                BoundNode {
                    key: "{cell.key()}",
                    form: props.form.clone(),
                    node: cell,
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SemanticGridCellProps {
    form: render::BoundForm,
    cell: render::BoundGridCell,
}

#[allow(non_snake_case)]
fn SemanticGridCell(props: SemanticGridCellProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.cell.identity,
        render::RenderNodeKind::StaticLayout,
        &props.cell.element_id,
    );
    rsx! {
        div {
            id: props.cell.element_id,
            class: "schemaform-grid-cell",
            "data-schemaform-grid-cell": "",
            "data-compact-span": props.cell.spans.compact(),
            "data-wide-span": props.cell.spans.wide(),
            for node in props.cell.children {
                BoundNode {
                    key: "{node.key()}",
                    form: props.form.clone(),
                    node,
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct AuthoredGroupProps {
    form: render::BoundForm,
    group: render::BoundAuthoredGroup,
}

#[allow(non_snake_case)]
fn AuthoredGroup(props: AuthoredGroupProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.group.identity,
        render::RenderNodeKind::StaticLayout,
        &props.group.element_id,
    );
    let label = localize_ui_text(&props.form, &props.group.label);
    rsx! {
        fieldset {
            id: props.group.element_id,
            class: "schemaform-group schemaform-authored-group",
            "data-schemaform-group": "",
            legend { "{label}" }
            for node in props.group.children {
                BoundNode {
                    key: "{node.key()}",
                    form: props.form.clone(),
                    node,
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SemanticTabsProps {
    form: render::BoundForm,
    tabs: render::BoundTabs,
}

#[derive(Props, Clone, PartialEq)]
struct SemanticTabPanelProps {
    form: render::BoundForm,
    panel: render::BoundTabPanel,
    index: usize,
    selected_index: usize,
    tab_id: String,
}

#[allow(non_snake_case)]
fn SemanticTabPanel(props: SemanticTabPanelProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.panel.identity,
        render::RenderNodeKind::StaticLayout,
        &props.panel.element_id,
    );
    #[cfg(schemaform_test_validation_faults)]
    use_observed_lifecycle(
        &props.form,
        Some((
            props.panel.identity,
            render::RenderNodeKind::StaticLayout,
            props.panel.element_id.clone(),
        )),
    );
    rsx! {
        div {
            id: props.panel.element_id,
            role: "tabpanel",
            class: "schemaform-tab-panel",
            "data-schemaform-tab-panel": "",
            "aria-labelledby": props.tab_id,
            hidden: props.selected_index != props.index,
            tabindex: "-1",
            for node in props.panel.children {
                BoundNode {
                    key: "{node.key()}",
                    form: props.form.clone(),
                    node,
                }
            }
        }
    }
}

#[allow(non_snake_case)]
fn SemanticTabs(props: SemanticTabsProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.tabs.identity,
        render::RenderNodeKind::StaticLayout,
        &props.tabs.element_id,
    );
    let mut selected = use_signal(|| 0_usize);
    let selected_index = *selected.read();
    let panel_count = props.tabs.panels.len();
    let tablist_label = props
        .form
        .inner
        .localizer
        .read()
        .localize(&render::MessageDescriptor {
            key: Some("schemaform.tabs.label".to_owned()),
            fallback: "Tabs".to_owned(),
            parameters: Value::Object(Default::default()),
        });
    let tabs_element_id = props.tabs.element_id.clone();
    let root_element_id = tabs_element_id.clone();
    let panels = props
        .tabs
        .panels
        .into_iter()
        .enumerate()
        .map(|(index, panel)| {
            let render::BoundNode::TabPanel(panel) = panel else {
                unreachable!("compiled tabs contain only tab panels")
            };
            let label = localize_ui_text(&props.form, &panel.label);
            let tab_id = format!("{}-tab-{index}", props.tabs.element_id);
            (index, label, tab_id, panel)
        })
        .collect::<Vec<_>>();
    let tab_buttons = panels
        .iter()
        .map(|(index, label, tab_id, panel)| {
            (
                *index,
                label.clone(),
                tab_id.clone(),
                panel.element_id.clone(),
                tabs_element_id.clone(),
            )
        })
        .collect::<Vec<_>>();

    rsx! {
        div {
            id: root_element_id,
            class: "schemaform-tabs",
            "data-schemaform-tabs": "",
            div {
                role: "tablist",
                "aria-label": tablist_label,
                "aria-orientation": "horizontal",
                for (index, label, tab_id, panel_id, tabs_element_id) in tab_buttons {
                    button {
                        id: tab_id.clone(),
                        r#type: "button",
                        role: "tab",
                        class: "schemaform-tab",
                        "aria-controls": panel_id,
                        "aria-selected": (selected_index == index).to_string(),
                        tabindex: if selected_index == index { "0" } else { "-1" },
                        onclick: move |_| selected.set(index),
                        onfocus: move |_| selected.set(index),
                        onkeydown: move |event| {
                            use dioxus_elements::Key;

                            let next = match event.key() {
                                Key::ArrowRight if panel_count > 0 => Some((index + 1) % panel_count),
                                Key::ArrowLeft if panel_count > 0 => {
                                    Some((index + panel_count - 1) % panel_count)
                                }
                                Key::Home if panel_count > 0 => Some(0),
                                Key::End if panel_count > 0 => Some(panel_count - 1),
                                Key::Enter => Some(index),
                                Key::Character(character) if character == " " => Some(index),
                                _ => None,
                            };
                            if let Some(next) = next {
                                event.prevent_default();
                                selected.set(next);
                                focus_element(&format!("{tabs_element_id}-tab-{next}"));
                            }
                        },
                        "{label}"
                    }
                }
            }
            for (index, _, tab_id, panel) in panels {
                SemanticTabPanel {
                    key: "{render::BoundNode::TabPanel(panel.clone()).key()}",
                    form: props.form.clone(),
                    panel,
                    index,
                    selected_index,
                    tab_id,
                }
            }
        }
    }
}

fn focus_element(id: &str) -> bool {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = id;
        false
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;

        if let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(id))
            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = element.focus();
        }
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_some_and(|element| element.id() == id)
    }
}

fn localize_ui_text(
    form: &render::BoundForm,
    reference: &schemaform::ui::v1::TextReference,
) -> String {
    localize_text(form, reference.key(), reference.fallback())
}

fn localize_text(form: &render::BoundForm, key: Option<&str>, fallback: &str) -> String {
    localize_message(
        form,
        &render::MessageDescriptor {
            key: key.map(str::to_owned),
            fallback: fallback.to_owned(),
            parameters: Value::Object(Default::default()),
        },
    )
}

enum BuiltinMessage {
    Submit,
    FindingSummary,
    ArrayItem { array_label: String },
    ArrayInsertBefore { item_label: String },
    ArrayMoveUp { item_label: String },
    ArrayMoveDown { item_label: String },
    ArrayRemove { item_label: String },
    ArrayAdd { item_label: String },
    ArrayInsertBeforeAt { item_label: String, position: usize },
    ArrayMoveUpAt { item_label: String, position: usize },
    ArrayMoveDownAt { item_label: String, position: usize },
    ArrayRemoveAt { item_label: String, position: usize },
    ArrayInserted { item_label: String, position: usize },
    ArrayMovedUp { item_label: String, position: usize },
    ArrayMovedDown { item_label: String, position: usize },
    ArrayRemoved { item_label: String, position: usize },
    ArrayAdded { item_label: String, position: usize },
    ArrayMaterialized { array_label: String },
    ArrayReplaced { array_label: String },
    ArrayCleared { array_label: String },
    PresenceAdd { label: String },
    PresenceSet { label: String },
    PresenceSetNull { label: String },
    PresenceRemove { label: String },
    PresenceReplace { label: String },
    WriteOnlyReplace { label: String },
    WriteOnlyReplacementPlaceholder { label: String },
    BooleanFalse,
    BooleanTrue,
    WriteOnlyNotSet { label: String },
    WriteOnlyNeedsReplacement { label: String },
    WriteOnlySet { label: String },
}

impl BuiltinMessage {
    fn descriptor(self) -> render::MessageDescriptor {
        let (key, fallback, parameters) = match self {
            Self::Submit => (
                "schemaform.submit.label",
                "Submit".to_owned(),
                serde_json::json!({}),
            ),
            Self::FindingSummary => (
                "schemaform.finding-summary.label",
                "Finding summary".to_owned(),
                serde_json::json!({}),
            ),
            Self::ArrayItem { array_label } => (
                "schemaform.array.item.label",
                format!("{array_label} item"),
                serde_json::json!({ "array_label": array_label }),
            ),
            Self::ArrayInsertBefore { item_label } => (
                "schemaform.array.insert-before.label",
                format!("Insert {item_label} before"),
                serde_json::json!({ "item_label": item_label }),
            ),
            Self::ArrayMoveUp { item_label } => (
                "schemaform.array.move-up.label",
                format!("Move {item_label} up"),
                serde_json::json!({ "item_label": item_label }),
            ),
            Self::ArrayMoveDown { item_label } => (
                "schemaform.array.move-down.label",
                format!("Move {item_label} down"),
                serde_json::json!({ "item_label": item_label }),
            ),
            Self::ArrayRemove { item_label } => (
                "schemaform.array.remove.label",
                format!("Remove {item_label}"),
                serde_json::json!({ "item_label": item_label }),
            ),
            Self::ArrayAdd { item_label } => (
                "schemaform.array.add.label",
                format!("Add {item_label}"),
                serde_json::json!({ "item_label": item_label }),
            ),
            Self::ArrayInsertBeforeAt {
                item_label,
                position,
            } => (
                "schemaform.array.insert-before-position.label",
                format!("Insert {item_label} before position {position}"),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayMoveUpAt {
                item_label,
                position,
            } => (
                "schemaform.array.move-up-position.label",
                format!("Move {item_label} at position {position} up"),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayMoveDownAt {
                item_label,
                position,
            } => (
                "schemaform.array.move-down-position.label",
                format!("Move {item_label} at position {position} down"),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayRemoveAt {
                item_label,
                position,
            } => (
                "schemaform.array.remove-position.label",
                format!("Remove {item_label} at position {position}"),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayInserted {
                item_label,
                position,
            } => (
                "schemaform.array.inserted.announcement",
                format!("{item_label} inserted at position {position}."),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayMovedUp {
                item_label,
                position,
            } => (
                "schemaform.array.moved-up.announcement",
                format!("{item_label} moved up to position {position}."),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayMovedDown {
                item_label,
                position,
            } => (
                "schemaform.array.moved-down.announcement",
                format!("{item_label} moved down to position {position}."),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayRemoved {
                item_label,
                position,
            } => (
                "schemaform.array.removed.announcement",
                format!("{item_label} removed from position {position}."),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayAdded {
                item_label,
                position,
            } => (
                "schemaform.array.added.announcement",
                format!("{item_label} added at position {position}."),
                serde_json::json!({ "item_label": item_label, "position": position }),
            ),
            Self::ArrayMaterialized { array_label } => (
                "schemaform.array.materialized.announcement",
                format!("{array_label} added."),
                serde_json::json!({ "array_label": array_label }),
            ),
            Self::ArrayReplaced { array_label } => (
                "schemaform.array.replaced.announcement",
                format!("{array_label} replaced."),
                serde_json::json!({ "array_label": array_label }),
            ),
            Self::ArrayCleared { array_label } => (
                "schemaform.array.cleared.announcement",
                format!("{array_label} removed."),
                serde_json::json!({ "array_label": array_label }),
            ),
            Self::PresenceAdd { label } => (
                "schemaform.presence.add.label",
                format!("Add {label}"),
                serde_json::json!({ "label": label }),
            ),
            Self::PresenceSet { label } => (
                "schemaform.presence.set.label",
                format!("Set {label}"),
                serde_json::json!({ "label": label }),
            ),
            Self::PresenceSetNull { label } => (
                "schemaform.presence.set-null.label",
                format!("Set {label} to null"),
                serde_json::json!({ "label": label }),
            ),
            Self::PresenceRemove { label } => (
                "schemaform.presence.remove.label",
                format!("Remove {label}"),
                serde_json::json!({ "label": label }),
            ),
            Self::PresenceReplace { label } => (
                "schemaform.presence.replace.label",
                format!("Replace {label}"),
                serde_json::json!({ "label": label }),
            ),
            Self::WriteOnlyReplace { label } => (
                "schemaform.write-only.replace.label",
                format!("Replace {label}"),
                serde_json::json!({ "label": label }),
            ),
            Self::WriteOnlyReplacementPlaceholder { label } => (
                "schemaform.write-only.replacement-placeholder",
                "Choose replacement".to_owned(),
                serde_json::json!({ "label": label }),
            ),
            Self::BooleanFalse => (
                "schemaform.boolean.false",
                "False".to_owned(),
                serde_json::json!({}),
            ),
            Self::BooleanTrue => (
                "schemaform.boolean.true",
                "True".to_owned(),
                serde_json::json!({}),
            ),
            Self::WriteOnlyNotSet { label } => (
                "schemaform.write-only.not-set.status",
                "Value is not set".to_owned(),
                serde_json::json!({ "label": label }),
            ),
            Self::WriteOnlyNeedsReplacement { label } => (
                "schemaform.write-only.needs-replacement.status",
                "Value needs replacement".to_owned(),
                serde_json::json!({ "label": label }),
            ),
            Self::WriteOnlySet { label } => (
                "schemaform.write-only.set.status",
                "Value is set".to_owned(),
                serde_json::json!({ "label": label }),
            ),
        };
        render::MessageDescriptor {
            key: Some(key.to_owned()),
            fallback,
            parameters,
        }
    }
}

fn localize_builtin(form: &render::BoundForm, message: BuiltinMessage) -> String {
    localize_message(form, &message.descriptor())
}

fn localize_message(form: &render::BoundForm, message: &render::MessageDescriptor) -> String {
    form.inner.localizer.read().localize(message)
}

fn localize_projection_text(
    form: &render::BoundForm,
    reference: Option<&schemaform::ui::v1::TextReference>,
    fallback: &str,
) -> String {
    localize_text(
        form,
        reference.and_then(schemaform::ui::v1::TextReference::key),
        fallback,
    )
}

fn localize_node_text(form: &render::BoundForm, projection: &mut handle::NodeProjection) {
    let label_fallback = projection.label.clone();
    projection.label =
        localize_projection_text(form, projection.label_reference.as_ref(), &label_fallback);
    if let Some(help) = projection.help.clone() {
        projection.help = Some(localize_projection_text(
            form,
            projection.help_reference.as_ref(),
            &help,
        ));
    }
}

fn instantiate_array_template(
    form: &render::BoundForm,
    template: &render::BoundTemplateNode,
    identity: schemaform::InstanceIdentity,
    array_element_id: &str,
) -> Option<render::BoundNode> {
    let projection = form.handle().node(identity).ok()??.read().ok()??;
    match template {
        render::BoundTemplateNode::Decorated(decorated) => {
            let child =
                instantiate_array_template(form, &decorated.child, identity, array_element_id)?;
            Some(render::BoundNode::Decorated(render::BoundDecorated {
                definition: decorated.definition,
                identity,
                child: Box::new(child),
                extensions: decorated.extensions.clone(),
            }))
        }
        render::BoundTemplateNode::Control(control) => {
            Some(render::BoundNode::Control(render::BoundControl {
                identity,
                input_id: array_item_input_id(array_element_id, identity),
                name: projection.binding?.as_str().to_owned(),
                kind: control.kind,
                renderer: control.renderer.clone(),
                extensions: control.extensions.clone(),
            }))
        }
        render::BoundTemplateNode::Group(group) => {
            Some(render::BoundNode::Group(render::BoundGroup {
                identity,
                element_id: array_item_input_id(array_element_id, identity),
                label: group.label.clone(),
                help: group.help.clone(),
                children: instantiate_array_template_children(
                    form,
                    &group.children,
                    projection.children,
                    array_element_id,
                )?,
            }))
        }
        render::BoundTemplateNode::Stack(stack) => {
            Some(render::BoundNode::Stack(render::BoundStack {
                identity,
                element_id: array_item_input_id(array_element_id, identity),
                transparent: stack.transparent,
                children: instantiate_array_template_children(
                    form,
                    &stack.children,
                    projection.children,
                    array_element_id,
                )?,
            }))
        }
        render::BoundTemplateNode::Grid(grid) => Some(render::BoundNode::Grid(render::BoundGrid {
            identity,
            element_id: array_item_input_id(array_element_id, identity),
            cells: instantiate_array_template_children(
                form,
                &grid.cells,
                projection.children,
                array_element_id,
            )?,
        })),
        render::BoundTemplateNode::GridCell(cell) => {
            Some(render::BoundNode::GridCell(render::BoundGridCell {
                identity,
                element_id: array_item_input_id(array_element_id, identity),
                spans: cell.spans,
                children: instantiate_array_template_children(
                    form,
                    &cell.children,
                    projection.children,
                    array_element_id,
                )?,
            }))
        }
        render::BoundTemplateNode::AuthoredGroup(group) => Some(render::BoundNode::AuthoredGroup(
            render::BoundAuthoredGroup {
                identity,
                element_id: array_item_input_id(array_element_id, identity),
                label: group.label.clone(),
                children: instantiate_array_template_children(
                    form,
                    &group.children,
                    projection.children,
                    array_element_id,
                )?,
            },
        )),
        render::BoundTemplateNode::Tabs(tabs) => Some(render::BoundNode::Tabs(render::BoundTabs {
            identity,
            element_id: array_item_input_id(array_element_id, identity),
            panels: instantiate_array_template_children(
                form,
                &tabs.panels,
                projection.children,
                array_element_id,
            )?,
        })),
        render::BoundTemplateNode::TabPanel(panel) => {
            Some(render::BoundNode::TabPanel(render::BoundTabPanel {
                identity,
                element_id: array_item_input_id(array_element_id, identity),
                label: panel.label.clone(),
                children: instantiate_array_template_children(
                    form,
                    &panel.children,
                    projection.children,
                    array_element_id,
                )?,
            }))
        }
        render::BoundTemplateNode::Text(text) => Some(render::BoundNode::Text(render::BoundText {
            identity,
            element_id: array_item_input_id(array_element_id, identity),
            content: text.content.clone(),
        })),
    }
}

fn instantiate_array_template_children(
    form: &render::BoundForm,
    templates: &[render::BoundTemplateNode],
    identities: Vec<schemaform::InstanceIdentity>,
    array_element_id: &str,
) -> Option<Vec<render::BoundNode>> {
    if identities.len() != templates.len() {
        return None;
    }
    templates
        .iter()
        .zip(identities)
        .map(|(template, identity)| {
            instantiate_array_template(form, template, identity, array_element_id)
        })
        .collect()
}

/// Computes the localized presentation shared by every node kind that renders chrome.
///
/// `projection` must already be localized through [`localize_node_text`]. Stable finding ids are
/// prefixed by `element_id`, so the same node rendered under a different element id yields
/// distinct ids. `presence` is the node's current presence affordances; only scalar controls
/// compute them today (see [`scalar_presence_affordances`]), containers pass an empty list.
fn node_presentation(
    form: &render::BoundForm,
    projection: &handle::NodeProjection,
    element_id: &str,
    presence: Vec<render::Affordance>,
) -> render::NodePresentation {
    let mut findings =
        validation_descriptors(form, projection, &format!("{element_id}-local-validation"));
    findings.extend(capability_descriptors(
        form,
        projection,
        &format!("{element_id}-local-capability"),
    ));
    findings.extend(external_descriptors(
        form,
        projection,
        &format!("{element_id}-local-external"),
    ));
    if let Some(kind) = projection.parse_blocker {
        findings.push(parse_descriptor(
            form,
            kind,
            format!("{element_id}-local-parse"),
        ));
    }
    let help = projection.help.clone().map(|text| render::Help {
        id: format!("{element_id}-help"),
        text,
    });
    render::NodePresentation::new(
        form.clone(),
        element_id.to_owned(),
        projection.label.clone(),
        projection.label_visible,
        help,
        findings,
        presence,
    )
}

#[derive(Props, Clone, PartialEq)]
struct HomogeneousArrayProps {
    form: render::BoundForm,
    array: render::BoundArray,
}

#[derive(Clone)]
enum ArrayFocusRequest {
    Element(Vec<String>),
    Row(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArrayAnnouncement {
    Inserted { position: usize },
    MovedUp { position: usize },
    MovedDown { position: usize },
    Removed { position: usize },
    Added { position: usize },
    Materialized,
    Replaced,
    Cleared,
}

impl ArrayAnnouncement {
    fn message(self, item_label: String, array_label: String) -> BuiltinMessage {
        match self {
            Self::Inserted { position } => BuiltinMessage::ArrayInserted {
                item_label,
                position,
            },
            Self::MovedUp { position } => BuiltinMessage::ArrayMovedUp {
                item_label,
                position,
            },
            Self::MovedDown { position } => BuiltinMessage::ArrayMovedDown {
                item_label,
                position,
            },
            Self::Removed { position } => BuiltinMessage::ArrayRemoved {
                item_label,
                position,
            },
            Self::Added { position } => BuiltinMessage::ArrayAdded {
                item_label,
                position,
            },
            Self::Materialized => BuiltinMessage::ArrayMaterialized { array_label },
            Self::Replaced => BuiltinMessage::ArrayReplaced { array_label },
            Self::Cleared => BuiltinMessage::ArrayCleared { array_label },
        }
    }
}

#[allow(non_snake_case)]
fn HomogeneousArray(props: HomogeneousArrayProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.array.identity,
        render::RenderNodeKind::Collection,
        &props.array.element_id,
    );
    let Ok(Some(reader)) = props.form.handle().node(props.array.identity) else {
        return rsx! {};
    };
    let Ok(Some(mut projection)) = reader.read() else {
        return rsx! {};
    };
    localize_node_text(&props.form, &mut projection);
    let item_label = props
        .array
        .item_label
        .as_ref()
        .map(|reference| localize_ui_text(&props.form, reference))
        .unwrap_or_else(|| {
            localize_builtin(
                &props.form,
                BuiltinMessage::ArrayItem {
                    array_label: projection.label.clone(),
                },
            )
        });
    let insert_before_label = localize_builtin(
        &props.form,
        BuiltinMessage::ArrayInsertBefore {
            item_label: item_label.clone(),
        },
    );
    let move_up_label = localize_builtin(
        &props.form,
        BuiltinMessage::ArrayMoveUp {
            item_label: item_label.clone(),
        },
    );
    let move_down_label = localize_builtin(
        &props.form,
        BuiltinMessage::ArrayMoveDown {
            item_label: item_label.clone(),
        },
    );
    let remove_label = localize_builtin(
        &props.form,
        BuiltinMessage::ArrayRemove {
            item_label: item_label.clone(),
        },
    );
    let add_label = localize_builtin(
        &props.form,
        BuiltinMessage::ArrayAdd {
            item_label: item_label.clone(),
        },
    );
    let collection = reader.collection_actions();
    let operation_errors = dioxus_core::try_consume_context::<OperationErrorHandler>();
    let can_remove = projection.allowed_operations.can_remove_item();
    let can_insert = projection.allowed_operations.can_append_item();
    let can_move = projection.allowed_operations.can_move_item();
    let mut announcement = use_signal(|| (0_u64, None::<ArrayAnnouncement>));
    let mut pending_announcement = use_signal(|| None::<(u64, ArrayAnnouncement)>);
    use_effect(move || {
        let pending = *pending_announcement.read();
        if let Some(pending) = pending {
            pending_announcement.write().take();
            announcement.set((pending.0, Some(pending.1)));
        }
    });
    let mut focus_target = use_signal(|| None::<ArrayFocusRequest>);
    let mut pending_focus_target = use_signal(|| None::<ArrayFocusRequest>);
    use_effect(move || {
        let pending = pending_focus_target.read().clone();
        if let Some(pending) = pending {
            pending_focus_target.write().take();
            focus_target.set(Some(pending));
        }
    });
    use_effect(move || {
        let target = focus_target.read().clone();
        if let Some(target) = target {
            focus_array_target(&target);
            focus_target.write().take();
        }
    });
    let presence_element_id = props.array.element_id.clone();
    let presence_success: ContainerPresenceSuccess = Rc::new(move |change| {
        let mut pending_focus = pending_focus_target;
        pending_focus.set(Some(ArrayFocusRequest::Element(vec![
            presence_element_id.clone(),
        ])));
        let event = match change {
            ContainerPresenceChange::Materialized => ArrayAnnouncement::Materialized,
            ContainerPresenceChange::Replaced => ArrayAnnouncement::Replaced,
            ContainerPresenceChange::Removed => ArrayAnnouncement::Cleared,
        };
        set_array_announcement(announcement, pending_announcement, event);
    });
    let presence_actions = container_presence_actions(
        &props.form,
        reader.actions(),
        &projection,
        Some(presence_success),
    );
    let presentation = node_presentation(
        &props.form,
        &projection,
        &props.array.element_id,
        Vec::new(),
    );
    let described_by = presentation.described_by();
    let invalid = presentation.invalid;
    let help = presentation.present_help();
    let presented_findings = presentation.present_findings();
    let mut items = Vec::new();
    for identity in projection.children.iter().copied() {
        let Ok(Some(item_reader)) = props.form.handle().node(identity) else {
            continue;
        };
        let Ok(Some(item)) = item_reader.read() else {
            continue;
        };
        let Some(item_identity) = item.item else {
            continue;
        };
        let Some(node) = instantiate_array_template(
            &props.form,
            &props.array.template,
            identity,
            &props.array.element_id,
        ) else {
            continue;
        };
        let row_id = array_item_input_id(&props.array.element_id, identity);
        items.push((item_identity, row_id, node));
    }
    let append_id = format!("{}-append", props.array.element_id);
    let rendered_items = items
        .into_iter()
        .enumerate()
        .map(|(index, (item, row_id, node))| (item, row_id, node, index))
        .collect::<Vec<_>>();
    let item_count = rendered_items.len();
    let append = collection.clone();
    let append_errors = operation_errors.clone();
    let append_reader = reader.clone();
    let append_element_id = props.array.element_id.clone();
    let (announcement_sequence, announcement_event) = *announcement.read();
    let announcement_text = announcement_event
        .map(|event| {
            localize_builtin(
                &props.form,
                event.message(item_label.clone(), projection.label.clone()),
            )
        })
        .unwrap_or_default();

    rsx! {
        fieldset {
            id: props.array.element_id,
            class: "schemaform-group schemaform-array",
            "data-schemaform-array": "",
            "aria-invalid": invalid,
            "aria-describedby": described_by,
            tabindex: "-1",
            legend { "{projection.label}" }
            {help}
            {presence_actions}
            for (item, row_id, node, index) in rendered_items {
                div {
                    key: "{row_id}",
                    id: "{row_id}-row",
                    class: "schemaform-array-item",
                    "data-array-item": "",
                    BoundNode {
                        form: props.form.clone(),
                        node,
                    }
                    if can_insert {
                        button {
                            id: "{row_id}-insert-before",
                            r#type: "button",
                            "data-insert-item-before": "",
                            "aria-label": localize_builtin(
                                &props.form,
                                BuiltinMessage::ArrayInsertBeforeAt {
                                    item_label: item_label.clone(),
                                    position: index + 1,
                                },
                            ),
                            onclick: {
                                let collection = collection.clone();
                                let reader = reader.clone();
                                let element_id = props.array.element_id.clone();
                                let operation_errors = operation_errors.clone();
                                move |_| {
                                    let before = reader
                                        .read()
                                        .ok()
                                        .flatten()
                                        .map(|view| view.children)
                                        .unwrap_or_default();
                                    if report_operation(
                                        &operation_errors,
                                        collection.insert_before(item),
                                    ) {
                                        if let Some(children) = reader
                                            .read()
                                            .ok()
                                            .flatten()
                                            .map(|view| view.children)
                                            && let Some(inserted) = children
                                                .iter()
                                                .find(|identity| !before.contains(identity))
                                        {
                                            pending_focus_target.set(Some(ArrayFocusRequest::Row(format!(
                                                "{}-row",
                                                array_item_input_id(&element_id, *inserted)
                                            ))));
                                        }
                                        set_array_announcement(
                                            announcement,
                                            pending_announcement,
                                            ArrayAnnouncement::Inserted {
                                                position: index + 1,
                                            },
                                        );
                                    }
                                }
                            },
                            "{insert_before_label}"
                        }
                    }
                    if can_move && index > 0 {
                        button {
                            id: "{row_id}-move-up",
                            r#type: "button",
                            "data-move-item-up": "",
                            "aria-label": localize_builtin(
                                &props.form,
                                BuiltinMessage::ArrayMoveUpAt {
                                    item_label: item_label.clone(),
                                    position: index + 1,
                                },
                            ),
                            onclick: {
                                let collection = collection.clone();
                                let row_id = row_id.clone();
                                let operation_errors = operation_errors.clone();
                                move |_| {
                                    if report_operation(
                                        &operation_errors,
                                        collection.move_up(item),
                                    ) {
                                        pending_focus_target.set(Some(ArrayFocusRequest::Element(vec![
                                            format!("{row_id}-move-up"),
                                            format!("{row_id}-move-down"),
                                            format!("{row_id}-row"),
                                        ])));
                                        set_array_announcement(
                                            announcement,
                                            pending_announcement,
                                            ArrayAnnouncement::MovedUp { position: index },
                                        );
                                    }
                                }
                            },
                            "{move_up_label}"
                        }
                    }
                    if can_move && index + 1 < item_count {
                        button {
                            id: "{row_id}-move-down",
                            r#type: "button",
                            "data-move-item-down": "",
                            "aria-label": localize_builtin(
                                &props.form,
                                BuiltinMessage::ArrayMoveDownAt {
                                    item_label: item_label.clone(),
                                    position: index + 1,
                                },
                            ),
                            onclick: {
                                let collection = collection.clone();
                                let row_id = row_id.clone();
                                let operation_errors = operation_errors.clone();
                                move |_| {
                                    if report_operation(
                                        &operation_errors,
                                        collection.move_down(item),
                                    ) {
                                        pending_focus_target.set(Some(ArrayFocusRequest::Element(vec![
                                            format!("{row_id}-move-down"),
                                            format!("{row_id}-move-up"),
                                            format!("{row_id}-row"),
                                        ])));
                                        set_array_announcement(
                                            announcement,
                                            pending_announcement,
                                            ArrayAnnouncement::MovedDown {
                                                position: index + 2,
                                            },
                                        );
                                    }
                                }
                            },
                            "{move_down_label}"
                        }
                    }
                    if can_remove {
                        button {
                            id: "{row_id}-remove",
                            r#type: "button",
                            "data-remove-item": "",
                            "aria-label": localize_builtin(
                                &props.form,
                                BuiltinMessage::ArrayRemoveAt {
                                    item_label: item_label.clone(),
                                    position: index + 1,
                                },
                            ),
                            onclick: {
                                let collection = collection.clone();
                                let reader = reader.clone();
                                let handle = props.form.handle().clone();
                                let element_id = props.array.element_id.clone();
                                let append_id = append_id.clone();
                                let operation_errors = operation_errors.clone();
                                move |_| {
                                    let children = reader
                                        .read()
                                        .ok()
                                        .flatten()
                                        .map(|view| view.children)
                                        .unwrap_or_default();
                                    let target_index = children.iter().position(|identity| {
                                        handle
                                            .node(*identity)
                                            .ok()
                                            .flatten()
                                            .and_then(|node| node.read().ok().flatten())
                                            .is_some_and(|view| view.item == Some(item))
                                    });
                                    let next_focus = target_index
                                        .and_then(|index| {
                                            children
                                                .get(index + 1)
                                                .or_else(|| {
                                                    index.checked_sub(1).and_then(|index| {
                                                        children.get(index)
                                                    })
                                                })
                                        })
                                        .map(|identity| {
                                            ArrayFocusRequest::Row(format!(
                                                "{}-row",
                                                array_item_input_id(&element_id, *identity)
                                            ))
                                        })
                                        .unwrap_or_else(|| {
                                            ArrayFocusRequest::Element(vec![append_id.clone()])
                                        });
                                    if report_operation(
                                        &operation_errors,
                                        collection.remove(item),
                                    ) {
                                        pending_focus_target.set(Some(next_focus));
                                        set_array_announcement(
                                            announcement,
                                            pending_announcement,
                                            ArrayAnnouncement::Removed {
                                                position: target_index
                                                    .map_or(0, |index| index + 1),
                                            },
                                        );
                                    }
                                }
                            },
                            "{remove_label}"
                        }
                    }
                }
            }
            if projection.allowed_operations.can_append_item() {
                button {
                    id: append_id,
                    r#type: "button",
                    "data-append-item": "",
                    onclick: move |_| {
                        let before = append_reader
                            .read()
                            .ok()
                            .flatten()
                            .map(|view| view.children)
                            .unwrap_or_default();
                        if report_operation(&append_errors, append.append()) {
                            if let Some(identity) = append_reader
                                .read()
                                .ok()
                                .flatten()
                                .and_then(|view| {
                                    view.children
                                        .into_iter()
                                        .find(|identity| !before.contains(identity))
                                })
                            {
                                pending_focus_target.set(Some(ArrayFocusRequest::Row(format!(
                                    "{}-row",
                                    array_item_input_id(&append_element_id, identity)
                                ))));
                            }
                            let position = append_reader
                                .read()
                                .ok()
                                .flatten()
                                .map_or(0, |view| view.children.len());
                            set_array_announcement(
                                announcement,
                                pending_announcement,
                                ArrayAnnouncement::Added { position },
                            );
                        }
                    },
                    "{add_label}"
                }
            }
            div {
                "data-array-status": "",
                "data-announcement-sequence": "{announcement_sequence}",
                role: "status",
                "aria-live": "polite",
                "aria-atomic": "true",
                "{announcement_text}"
            }
            {presented_findings}
        }
    }
}

fn array_item_input_id(array_element_id: &str, identity: schemaform::InstanceIdentity) -> String {
    format!("{array_element_id}-item-{:016x}", identity_hash(identity))
}

fn set_array_announcement(
    mut announcement: Signal<(u64, Option<ArrayAnnouncement>)>,
    mut pending: Signal<Option<(u64, ArrayAnnouncement)>>,
    event: ArrayAnnouncement,
) {
    let sequence = announcement.peek().0.saturating_add(1);
    announcement.set((sequence, None));
    pending.set(Some((sequence, event)));
}

fn focus_array_target(target: &ArrayFocusRequest) {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    match target {
        ArrayFocusRequest::Element(targets) => {
            let _ = targets;
        }
        ArrayFocusRequest::Row(row) => {
            let _ = row;
        }
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;

        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let target = match target {
            ArrayFocusRequest::Element(targets) => targets.iter().find_map(|target| {
                document
                    .get_element_by_id(target)
                    .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
            }),
            ArrayFocusRequest::Row(row) => document
                .get_element_by_id(row)
                .and_then(|row| {
                    row.query_selector(
                        "input:not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])",
                    )
                    .ok()
                    .flatten()
                })
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok()),
        };
        if let Some(target) = target {
            let _ = target.focus();
        }
    }
}

fn identity_hash(identity: schemaform::InstanceIdentity) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity.hash(&mut hasher);
    hasher.finish()
}

#[derive(Props, Clone, PartialEq)]
struct FixedObjectGroupProps {
    form: render::BoundForm,
    group: render::BoundGroup,
}

#[derive(Clone, Copy)]
enum ContainerPresenceChange {
    Materialized,
    Replaced,
    Removed,
}

type ContainerPresenceSuccess = Rc<dyn Fn(ContainerPresenceChange)>;

fn container_presence_actions(
    form: &render::BoundForm,
    actions: handle::ControlActions,
    projection: &handle::NodeProjection,
    on_success: Option<ContainerPresenceSuccess>,
) -> Element {
    let materialize_actions = actions.clone();
    let replace_actions = actions.clone();
    let remove_actions = actions;
    let replacement = projection.creation_seed.clone();
    let operation_errors = dioxus_core::try_consume_context::<OperationErrorHandler>();
    let materialize_errors = operation_errors.clone();
    let replace_errors = operation_errors.clone();
    let remove_errors = operation_errors;
    let materialize_success = on_success.clone();
    let replace_success = on_success.clone();
    let remove_success = on_success;
    let incompatible_value = (projection.allowed_operations.can_replace_value()
        && !projection.write_only)
        .then(|| projection.current_data.as_ref().map(Value::to_string))
        .flatten();
    let add_label = localize_builtin(
        form,
        BuiltinMessage::PresenceAdd {
            label: projection.label.clone(),
        },
    );
    let replace_label = localize_builtin(
        form,
        BuiltinMessage::PresenceReplace {
            label: projection.label.clone(),
        },
    );
    let remove_label = localize_builtin(
        form,
        BuiltinMessage::PresenceRemove {
            label: projection.label.clone(),
        },
    );

    rsx! {
        div { class: "schemaform-presence-actions",
            if let Some(value) = incompatible_value {
                output { "data-incompatible-value": "", "{value}" }
            }
            if projection.allowed_operations.can_materialize() {
                button {
                    r#type: "button",
                    "data-materialize": "",
                    onclick: move |_| {
                        if report_operation(
                            &materialize_errors,
                            materialize_actions.materialize(),
                        ) && let Some(on_success) = &materialize_success {
                            on_success(ContainerPresenceChange::Materialized);
                        }
                    },
                    "{add_label}"
                }
            }
            if projection.allowed_operations.can_replace_value()
                && let Some(replacement) = replacement
            {
                button {
                    r#type: "button",
                    "data-replace-value": "",
                    onclick: move |_| {
                        if report_operation(
                            &replace_errors,
                            replace_actions.replace_value(replacement.clone()),
                        ) && let Some(on_success) = &replace_success {
                            on_success(ContainerPresenceChange::Replaced);
                        }
                    },
                    "{replace_label}"
                }
            }
            if projection.allowed_operations.can_remove_value() {
                button {
                    r#type: "button",
                    "data-remove-value": "",
                    onclick: move |_| {
                        if report_operation(&remove_errors, remove_actions.remove_value())
                            && let Some(on_success) = &remove_success
                        {
                            on_success(ContainerPresenceChange::Removed);
                        }
                    },
                    "{remove_label}"
                }
            }
        }
    }
}

#[allow(non_snake_case)]
fn FixedObjectGroup(props: FixedObjectGroupProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.group.identity,
        render::RenderNodeKind::StaticLayout,
        &props.group.element_id,
    );
    let Ok(Some(reader)) = props.form.handle().node(props.group.identity) else {
        return rsx! {};
    };
    let Ok(Some(mut projection)) = reader.read() else {
        return rsx! {};
    };
    localize_node_text(&props.form, &mut projection);
    let presence_actions =
        container_presence_actions(&props.form, reader.actions(), &projection, None);
    let presentation = node_presentation(
        &props.form,
        &projection,
        &props.group.element_id,
        Vec::new(),
    );
    let described_by = presentation.described_by();
    let invalid = presentation.invalid;
    let help = presentation.present_help();
    let presented_findings = presentation.present_findings();
    let group_label = projection.label.clone();
    rsx! {
        fieldset {
            id: props.group.element_id,
            class: "schemaform-group schemaform-fixed-object",
            "data-schemaform-fixed-object": "",
            "aria-invalid": invalid,
            "aria-describedby": described_by,
            tabindex: "-1",
            legend { "{group_label}" }
            {help}
            {presence_actions}
            for node in props.group.children {
                BoundNode {
                    key: "{node.key()}",
                    form: props.form.clone(),
                    node,
                }
            }
            {presented_findings}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct UnsupportedRegionProps {
    form: render::BoundForm,
    region: render::BoundUnsupported,
}

#[allow(non_snake_case)]
fn UnsupportedRegion(props: UnsupportedRegionProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.region.identity,
        render::RenderNodeKind::Unsupported,
        &props.region.element_id,
    );
    let Some(mut projection) = props
        .form
        .handle()
        .node(props.region.identity)
        .ok()
        .flatten()
        .and_then(|reader| reader.read().ok().flatten())
    else {
        return rsx! {};
    };
    localize_node_text(&props.form, &mut projection);
    let Some(first_finding) = projection.capability_findings.first() else {
        return rsx! {};
    };
    let code = first_finding.code().to_owned();
    let binding = projection
        .binding
        .as_ref()
        .map(|binding| binding.as_str().to_owned())
        .unwrap_or_default();
    let presentation = node_presentation(
        &props.form,
        &projection,
        &props.region.element_id,
        Vec::new(),
    );
    let described_by = presentation.described_by();
    let help = presentation.present_help();
    let presented_findings = presentation.present_findings();

    rsx! {
        section {
            id: props.region.element_id,
            class: "schemaform-unsupported",
            "data-schemaform-unsupported": "",
            "data-capability-finding": code,
            "data-binding": binding,
            "aria-label": projection.label.clone(),
            "aria-describedby": described_by,
            tabindex: "-1",
            strong { "{projection.label}" }
            {help}
            {presented_findings}
        }
    }
}

fn validation_descriptors(
    form: &render::BoundForm,
    projection: &handle::NodeProjection,
    stable_id_prefix: &str,
) -> Vec<render::FindingDescriptor> {
    projection
        .validation_findings
        .iter()
        .map(|finding| {
            let mut descriptor = validation_descriptors_from_finding(form, finding);
            descriptor.stable_id = validation_finding_stable_id(stable_id_prefix, finding);
            descriptor
        })
        .collect()
}

fn capability_descriptors(
    form: &render::BoundForm,
    projection: &handle::NodeProjection,
    stable_id_prefix: &str,
) -> Vec<render::FindingDescriptor> {
    projection
        .capability_findings
        .iter()
        .map(|finding| {
            let mut descriptor = capability_descriptor(form, finding);
            descriptor.stable_id = capability_finding_stable_id(stable_id_prefix, finding);
            descriptor
        })
        .collect()
}

fn capability_descriptor(
    form: &render::BoundForm,
    finding: &schemaform::CapabilityFinding,
) -> render::FindingDescriptor {
    let message = render::MessageDescriptor {
        key: Some(format!("schemaform.capability.{}", finding.code())),
        fallback: match finding.code() {
            "applicator.one-of" => {
                "This form region cannot be edited because oneOf branch selection is unsupported."
                    .to_owned()
            }
            "applicator.all-of.ambiguous" => {
                "This form region cannot be edited because its allOf constraints are incompatible."
                    .to_owned()
            }
            "applicator.additional-properties.open" => {
                "Declared properties can be edited. Undeclared properties are preserved and validated, but arbitrary-key editing is unavailable."
                    .to_owned()
            }
            "applicator.additional-properties.schema-projection" => {
                "Declared properties can be edited. Schema-constrained additional properties are preserved and validated, but arbitrary-key editing is unavailable."
                    .to_owned()
            }
            "applicator.pattern-properties.fixed-projection" => {
                "Declared properties can be edited. Pattern-matched properties are preserved and validated, but arbitrary-key editing is unavailable."
                    .to_owned()
            }
            "annotation.conflict" => {
                let keyword = finding
                    .parameters()
                    .get("keyword")
                    .and_then(Value::as_str)
                    .unwrap_or("presentation annotation");
                format!("Conflicting {keyword} annotations were ignored for this form control.")
            }
            code => format!("This form region cannot be edited because {code} is unsupported."),
        },
        parameters: finding.parameters().clone(),
    };
    render::FindingDescriptor {
        stable_id: String::new(),
        kind: render::FindingKind::Capability,
        code: finding.code().to_owned(),
        text: localize_message(form, &message),
        blocking: finding.is_blocking(),
        parameters: finding.parameters().clone(),
    }
}

fn external_descriptors(
    form: &render::BoundForm,
    projection: &handle::NodeProjection,
    stable_id_prefix: &str,
) -> Vec<render::FindingDescriptor> {
    projection
        .external_findings
        .iter()
        .map(|(source, finding)| {
            external_descriptor(
                form,
                source,
                finding,
                external_finding_stable_id(stable_id_prefix, source, finding),
            )
        })
        .collect()
}

fn external_descriptor(
    form: &render::BoundForm,
    source: &str,
    finding: &schemaform::ExternalFinding,
    stable_id: String,
) -> render::FindingDescriptor {
    let message = render::MessageDescriptor {
        key: Some(format!("schemaform.external.{source}.{}", finding.code())),
        fallback: format!("{} reported {}.", source, finding.code()),
        parameters: finding.parameters().clone(),
    };
    render::FindingDescriptor {
        stable_id,
        kind: render::FindingKind::External,
        code: finding.code().to_owned(),
        text: localize_message(form, &message),
        blocking: finding.is_blocking(),
        parameters: finding.parameters().clone(),
    }
}

fn parse_descriptor(
    form: &render::BoundForm,
    kind: schemaform::form::ParseBlockerKind,
    stable_id: String,
) -> render::FindingDescriptor {
    let (code, fallback) = match kind {
        schemaform::form::ParseBlockerKind::InvalidNumber => {
            ("invalid-number", "Enter a valid number.")
        }
        schemaform::form::ParseBlockerKind::InvalidInteger => {
            ("invalid-integer", "Enter a valid integer.")
        }
        schemaform::form::ParseBlockerKind::ResourceLimitExceeded => (
            "resource-limit-exceeded",
            "The entered value exceeds the supported size.",
        ),
        _ => ("parse-blocked", "The entered value cannot be used."),
    };
    let parameters = serde_json::json!({});
    let message = render::MessageDescriptor {
        key: Some(format!("schemaform.parse.{code}")),
        fallback: fallback.to_owned(),
        parameters: parameters.clone(),
    };
    render::FindingDescriptor {
        stable_id,
        kind: render::FindingKind::Parse,
        code: code.to_owned(),
        text: localize_message(form, &message),
        blocking: true,
        parameters,
    }
}

fn indeterminate_descriptor(
    form: &render::BoundForm,
    reason: &schemaform::form::IndeterminateReason,
    stable_id: String,
) -> render::FindingDescriptor {
    let parameters = serde_json::json!({});
    let message = render::MessageDescriptor {
        key: Some(format!("schemaform.indeterminate.{}", reason.code())),
        fallback: "Validation could not be completed reliably.".to_owned(),
        parameters: parameters.clone(),
    };
    render::FindingDescriptor {
        stable_id,
        kind: render::FindingKind::Indeterminate,
        code: reason.code().to_owned(),
        text: localize_message(form, &message),
        blocking: true,
        parameters,
    }
}

fn summary_finding_stable_id(form_id: &str, finding: &handle::FindingProjection) -> String {
    let mut hasher = StableIdHasher::default();
    match finding {
        handle::FindingProjection::Validation { target, finding } => {
            hasher.write_part(b"validation");
            target.hash(&mut hasher);
            hasher.write_part(finding.code().as_bytes());
            hasher.write_part(finding.instance_location().as_str().as_bytes());
            hasher.write_part(finding.keyword_location().resource().as_str().as_bytes());
            hasher.write_part(finding.keyword_location().pointer().as_str().as_bytes());
            hasher.write_json(finding.parameters());
        }
        handle::FindingProjection::ValidationFindingsTruncated { target, retained } => {
            hasher.write_part(b"validation-findings-truncated");
            target.hash(&mut hasher);
            hasher.write_usize(*retained);
        }
        handle::FindingProjection::Indeterminate { target, reason } => {
            hasher.write_part(b"indeterminate");
            target.hash(&mut hasher);
            hasher.write_part(reason.code().as_bytes());
        }
        handle::FindingProjection::Capability { target, finding } => {
            hasher.write_part(b"capability");
            target.hash(&mut hasher);
            hasher.write_part(finding.code().as_bytes());
            hasher.write_part(finding.instance_location().as_str().as_bytes());
            hasher.write_part(finding.keyword_location().resource().as_str().as_bytes());
            hasher.write_part(finding.keyword_location().pointer().as_str().as_bytes());
            hasher.write_u8(u8::from(finding.is_blocking()));
            hasher.write_json(finding.parameters());
        }
        handle::FindingProjection::External {
            target,
            source,
            finding,
        } => {
            hasher.write_part(b"external");
            target.hash(&mut hasher);
            hasher.write_part(source.as_bytes());
            hasher.write_part(finding.code().as_bytes());
            hasher.write_part(finding.instance_location().as_str().as_bytes());
            hasher.write_u8(u8::from(finding.is_blocking()));
            hasher.write_json(finding.parameters());
        }
        handle::FindingProjection::Parse { target, kind } => {
            hasher.write_part(b"parse");
            target.hash(&mut hasher);
            let kind = match kind {
                schemaform::form::ParseBlockerKind::InvalidNumber => b"invalid-number".as_slice(),
                schemaform::form::ParseBlockerKind::InvalidInteger => b"invalid-integer".as_slice(),
                schemaform::form::ParseBlockerKind::ResourceLimitExceeded => {
                    b"resource-limit-exceeded".as_slice()
                }
                _ => b"parse-blocked".as_slice(),
            };
            hasher.write_part(kind);
        }
    }
    format!("{form_id}-summary-finding-{:016x}", hasher.finish())
}

#[derive(Default)]
struct StableIdHasher(u64);

impl StableIdHasher {
    fn write_part(&mut self, bytes: &[u8]) {
        self.write_usize(bytes.len());
        self.write(bytes);
    }

    fn write_json(&mut self, value: &Value) {
        let encoded = serde_json::to_vec(value).expect("finding parameters are JSON values");
        self.write_part(&encoded);
    }
}

impl Hasher for StableIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

fn validation_finding_stable_id(prefix: &str, finding: &schemaform::ValidationFinding) -> String {
    let mut hasher = StableIdHasher::default();
    hasher.write_part(finding.code().as_bytes());
    hasher.write_part(finding.instance_location().as_str().as_bytes());
    hasher.write_part(finding.keyword_location().resource().as_str().as_bytes());
    hasher.write_part(finding.keyword_location().pointer().as_str().as_bytes());
    hasher.write_json(finding.parameters());
    format!("{prefix}-finding-{:016x}", hasher.finish())
}

fn capability_finding_stable_id(prefix: &str, finding: &schemaform::CapabilityFinding) -> String {
    let mut hasher = StableIdHasher::default();
    hasher.write_part(finding.code().as_bytes());
    hasher.write_part(finding.instance_location().as_str().as_bytes());
    hasher.write_part(finding.keyword_location().resource().as_str().as_bytes());
    hasher.write_part(finding.keyword_location().pointer().as_str().as_bytes());
    hasher.write_u8(u8::from(finding.is_blocking()));
    hasher.write_json(finding.parameters());
    format!("{prefix}-finding-{:016x}", hasher.finish())
}

fn external_finding_stable_id(
    prefix: &str,
    source: &str,
    finding: &schemaform::ExternalFinding,
) -> String {
    let mut hasher = StableIdHasher::default();
    hasher.write_part(source.as_bytes());
    hasher.write_part(finding.code().as_bytes());
    hasher.write_part(finding.instance_location().as_str().as_bytes());
    hasher.write_u8(u8::from(finding.is_blocking()));
    hasher.write_json(finding.parameters());
    format!("{prefix}-finding-{:016x}", hasher.finish())
}

fn finding_descriptor(
    form: &render::BoundForm,
    finding: &handle::FindingProjection,
    stable_id: String,
) -> render::FindingDescriptor {
    match finding {
        handle::FindingProjection::Validation { finding, .. } => {
            let mut descriptor = validation_descriptors_from_finding(form, finding);
            descriptor.stable_id = stable_id;
            descriptor
        }
        handle::FindingProjection::ValidationFindingsTruncated { retained, .. } => {
            let parameters = serde_json::json!({ "retained": retained });
            let message = render::MessageDescriptor {
                key: Some("schemaform.validation.findings-truncated".to_owned()),
                fallback: format!(
                    "Additional validation findings were omitted after the first {retained}."
                ),
                parameters: parameters.clone(),
            };
            render::FindingDescriptor {
                stable_id,
                kind: render::FindingKind::Validation,
                code: "validation-findings-truncated".to_owned(),
                text: localize_message(form, &message),
                blocking: true,
                parameters,
            }
        }
        handle::FindingProjection::Indeterminate { reason, .. } => {
            indeterminate_descriptor(form, reason, stable_id)
        }
        handle::FindingProjection::Capability { finding, .. } => {
            let mut descriptor = capability_descriptor(form, finding);
            descriptor.stable_id = stable_id;
            descriptor
        }
        handle::FindingProjection::External {
            source, finding, ..
        } => external_descriptor(form, source, finding, stable_id),
        handle::FindingProjection::Parse { kind, .. } => parse_descriptor(form, *kind, stable_id),
    }
}

fn validation_descriptors_from_finding(
    form: &render::BoundForm,
    finding: &schemaform::ValidationFinding,
) -> render::FindingDescriptor {
    let message = render::MessageDescriptor {
        key: Some(format!("schemaform.validation.{}", finding.code())),
        fallback: validation_finding_fallback(finding),
        parameters: finding.parameters().clone(),
    };
    render::FindingDescriptor {
        stable_id: String::new(),
        kind: render::FindingKind::Validation,
        code: finding.code().to_owned(),
        text: localize_message(form, &message),
        blocking: true,
        parameters: finding.parameters().clone(),
    }
}

#[derive(Props, Clone, PartialEq)]
struct ControlHostProps {
    form: render::BoundForm,
    control: render::BoundControl,
}

/// Derives the control facets shared by the built-in control and custom renderers.
///
/// `projection` must already be localized through [`localize_node_text`].
fn control_facets(
    form: &render::BoundForm,
    control: &render::BoundControl,
    projection: &handle::NodeProjection,
) -> render::ControlFacets {
    use schemaform::form::{AllowedOperations, ScalarValueState};

    let kind = control.kind;
    let value_state = projection.value_state;
    let operations = projection.allowed_operations;
    let selectable =
        operations.can_set_value() || operations.can_set_null() || operations.can_replace_value();
    let required = projection.required
        && if projection.write_only {
            matches!(
                value_state,
                Some(ScalarValueState::Missing | ScalarValueState::Incompatible)
            ) || matches!(value_state, Some(ScalarValueState::Null))
                && operations.can_replace_value()
        } else {
            projection.current_data.is_some() || operations != AllowedOperations::default()
        };
    let disabled = kind == render::ControlKind::Constant
        || matches!(
            kind,
            render::ControlKind::Boolean | render::ControlKind::Choice
        ) && !selectable;
    let read_only = projection.read_only
        || kind == render::ControlKind::Constant
        || matches!(
            kind,
            render::ControlKind::String
                | render::ControlKind::Number
                | render::ControlKind::Integer
        ) && !operations.can_input_text();
    let write_only_replacement =
        (projection.write_only && !projection.read_only && kind != render::ControlKind::Constant)
            .then(|| render::WriteOnlyReplacement {
                label: localize_builtin(
                    form,
                    BuiltinMessage::WriteOnlyReplace {
                        label: projection.label.clone(),
                    },
                ),
                placeholder: localize_builtin(
                    form,
                    BuiltinMessage::WriteOnlyReplacementPlaceholder {
                        label: projection.label.clone(),
                    },
                ),
            });
    let write_only_status = projection.write_only.then(|| {
        let label = projection.label.clone();
        let message = match value_state {
            Some(ScalarValueState::Missing) => BuiltinMessage::WriteOnlyNotSet { label },
            Some(ScalarValueState::Incompatible) => {
                BuiltinMessage::WriteOnlyNeedsReplacement { label }
            }
            _ => BuiltinMessage::WriteOnlySet { label },
        };
        localize_builtin(form, message)
    });
    let boolean_labels = (kind == render::ControlKind::Boolean).then(|| render::BooleanLabels {
        false_label: localize_builtin(form, BuiltinMessage::BooleanFalse),
        true_label: localize_builtin(form, BuiltinMessage::BooleanTrue),
    });
    render::ControlFacets {
        kind,
        name: control.name.clone(),
        required,
        disabled,
        read_only,
        write_only: projection.write_only,
        touched: projection.touched,
        dirty: projection.dirty,
        nullable: projection.nullable,
        write_only_replacement,
        write_only_status,
        boolean_labels,
    }
}

fn value_state_attribute(state: Option<schemaform::form::ScalarValueState>) -> &'static str {
    use schemaform::form::ScalarValueState;

    match state {
        Some(ScalarValueState::Missing) => "missing",
        Some(ScalarValueState::Null) => "null",
        Some(ScalarValueState::Empty) => "empty",
        Some(ScalarValueState::Compatible) => "compatible",
        Some(ScalarValueState::Incompatible) => "incompatible",
        Some(_) | None => "unsupported",
    }
}

/// Hook-stable callbacks behind one scalar control's presence affordances.
///
/// The callbacks keep their identity across renders, so a renderer that stores an
/// [`render::Affordance`] does not accumulate a new callback per keystroke and a child component
/// that memoizes on the affordance keeps calling a live callback.
#[derive(Clone, Copy)]
struct ScalarPresenceCallbacks {
    set: Callback<()>,
    set_null: Callback<()>,
    remove_value: Callback<()>,
    replace: Callback<()>,
}

/// Creates the presence callbacks for one scalar control.
///
/// Each callback performs its core operation through `actions` at invocation time and reports a
/// failure to the host's `on_error`. `seed` is the definition's creation seed used by set and
/// replace. `actions` is `None` and `seed` absent while the node is unavailable; the callbacks are
/// then no-ops, and the matching affordances are never offered. This is a hook: call it at the
/// same position on every render.
fn use_scalar_presence_callbacks(
    actions: Option<&handle::ControlActions>,
    seed: Option<Value>,
    error_route: Option<OperationErrorHandler>,
) -> ScalarPresenceCallbacks {
    /// One presence operation; `None` when its precondition (a seed) is absent.
    type Operation = fn(
        &handle::ControlActions,
        Option<&Value>,
    ) -> Option<Result<schemaform::Transition, handle::HandleError>>;
    let callback = |operation: Operation| {
        let actions = actions.cloned();
        let seed = seed.clone();
        let error_route = error_route.clone();
        use_callback(move |()| {
            if let Some(actions) = &actions
                && let Some(result) = operation(actions, seed.as_ref())
            {
                report_operation(&error_route, result);
            }
        })
    };
    ScalarPresenceCallbacks {
        set: callback(|actions, seed| seed.map(|value| actions.set_value(value.clone()))),
        set_null: callback(|actions, _| Some(actions.set_null())),
        remove_value: callback(|actions, _| Some(actions.remove_value())),
        replace: callback(|actions, seed| seed.map(|value| actions.replace_value(value.clone()))),
    }
}

/// Computes the presence affordances the core allows for one scalar control right now.
///
/// This is the single statement of the built-in presence rules: set only while the value is
/// missing or null and a creation seed exists; set null and remove value exactly when the core
/// allows them; replace only while the core allows replacement and a seed exists. The built-in
/// control renders its presence buttons from this list, so custom renderers receive exactly the
/// operations the built-in would offer.
fn scalar_presence_affordances(
    form: &render::BoundForm,
    projection: &handle::NodeProjection,
    element_id: &str,
    callbacks: ScalarPresenceCallbacks,
) -> Vec<render::Affordance> {
    use render::{Affordance, AffordanceKind};
    use schemaform::form::ScalarValueState;

    let operations = projection.allowed_operations;
    let has_seed = projection.creation_seed.is_some();
    let label = || projection.label.clone();
    let mut presence = Vec::new();
    if operations.can_set_value()
        && matches!(
            projection.value_state,
            Some(ScalarValueState::Missing | ScalarValueState::Null)
        )
        && has_seed
    {
        presence.push(Affordance {
            kind: AffordanceKind::Set,
            label: localize_builtin(form, BuiltinMessage::PresenceSet { label: label() }),
            id: format!("{element_id}-set-value"),
            invoke: callbacks.set,
        });
    }
    if operations.can_set_null() {
        presence.push(Affordance {
            kind: AffordanceKind::SetNull,
            label: localize_builtin(form, BuiltinMessage::PresenceSetNull { label: label() }),
            id: format!("{element_id}-set-null"),
            invoke: callbacks.set_null,
        });
    }
    if operations.can_remove_value() {
        presence.push(Affordance {
            kind: AffordanceKind::RemoveValue,
            label: localize_builtin(form, BuiltinMessage::PresenceRemove { label: label() }),
            id: format!("{element_id}-remove-value"),
            invoke: callbacks.remove_value,
        });
    }
    if operations.can_replace_value() && has_seed {
        presence.push(Affordance {
            kind: AffordanceKind::Replace,
            label: localize_builtin(form, BuiltinMessage::PresenceReplace { label: label() }),
            id: format!("{element_id}-replace-value"),
            invoke: callbacks.replace,
        });
    }
    presence
}

/// Renders the built-in presence buttons for one scalar control from its affordances.
///
/// Each button carries the affordance's id and one marker attribute for its operation
/// (`data-set-value`, `data-set-null`, `data-remove-value`, `data-replace-value`) and invokes the
/// affordance. rsx attribute names are literal, so each marker is an `Option` attribute.
fn scalar_presence_actions(presence: &[render::Affordance]) -> Element {
    use render::AffordanceKind;

    let buttons = presence.to_vec();
    rsx! {
        div { class: "schemaform-presence-actions",
            for affordance in buttons {
                button {
                    key: "{affordance.id}",
                    id: affordance.id.clone(),
                    r#type: "button",
                    "data-set-value": (affordance.kind == AffordanceKind::Set).then_some(""),
                    "data-set-null": (affordance.kind == AffordanceKind::SetNull).then_some(""),
                    "data-remove-value": (affordance.kind == AffordanceKind::RemoveValue)
                        .then_some(""),
                    "data-replace-value": (affordance.kind == AffordanceKind::Replace)
                        .then_some(""),
                    onclick: move |_| affordance.invoke.call(()),
                    "{affordance.label}"
                }
            }
        }
    }
}

/// Hosts one bound control: computes its render context and hands it to the preflight-selected
/// renderer.
///
/// This is the single control render path. The built-in renderer is one possible selection, so
/// there is no built-in/custom fork here. Renderer-entry observation lives in this host so the
/// reactivity gate sees exactly one entry per edit regardless of how the renderer is composed.
#[allow(non_snake_case)]
fn ControlHost(props: ControlHostProps) -> Element {
    #[cfg(schemaform_test_validation_faults)]
    observe_renderer_entry(
        &props.form,
        props.control.identity,
        render::RenderNodeKind::Control,
        &props.control.input_id,
    );
    let reader = props
        .form
        .handle()
        .node(props.control.identity)
        .ok()
        .flatten();
    let projection = reader
        .as_ref()
        .and_then(|reader| reader.read().ok().flatten());
    // Hooks run before the availability guards below so the hook order is identical on every
    // render, including renders where the node has already been removed or disposed.
    let operation_errors = dioxus_core::try_consume_context::<OperationErrorHandler>();
    let actions = reader.as_ref().map(handle::NodeReader::actions);
    let presence_callbacks = use_scalar_presence_callbacks(
        actions.as_ref(),
        projection
            .as_ref()
            .and_then(|projection| projection.creation_seed.clone()),
        operation_errors.clone(),
    );
    let (Some(reader), Some(mut projection), Some(actions)) = (reader, projection, actions) else {
        return rsx! {};
    };
    localize_node_text(&props.form, &mut projection);
    let presence = scalar_presence_affordances(
        &props.form,
        &projection,
        &props.control.input_id,
        presence_callbacks,
    );
    let presentation =
        node_presentation(&props.form, &projection, &props.control.input_id, presence);
    let facets = control_facets(&props.form, &props.control, &projection);
    let context = render::ControlRenderContext::new(
        reader,
        actions,
        presentation,
        facets,
        props.control.extensions.clone(),
        operation_errors,
    );
    props.control.renderer.render(context)
}

/// The value shown beside a control that cannot edit its current data, as the built-in shows it.
///
/// Present while the value is incompatible, or null where null is not accepted, the core rejects
/// text input but allows replacement, and the control is not write-only.
fn incompatible_value(projection: &handle::NodeProjection) -> Option<String> {
    use schemaform::form::ScalarValueState;

    let operations = projection.allowed_operations;
    (matches!(
        projection.value_state,
        Some(ScalarValueState::Incompatible | ScalarValueState::Null)
            if !operations.can_input_text() && operations.can_replace_value()
    ) && !projection.write_only)
        .then(|| {
            projection
                .current_data
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_default()
        })
}

#[derive(Props, Clone, PartialEq)]
struct BuiltinControlProps {
    context: render::ControlRenderContext,
}

/// The chrome every built-in control derives from its render context.
///
/// Each built-in child component reads the node like any renderer would: whether the node is
/// read-only (rendered as `output`) rather than merely rejecting edits right now, and the
/// incompatible value to show beside an editable widget, are node state the facets fold together.
struct BuiltinChrome {
    element_id: String,
    name: String,
    kind: render::ControlKind,
    label: String,
    label_visible: bool,
    invalid: bool,
    described_by: Option<String>,
    supplements: Element,
    presented_findings: Element,
    presence_actions: Element,
    value_state_attribute: &'static str,
    /// The write-only replacement label, shown in place of the label for editable write-only
    /// widgets.
    replacement_label: Option<String>,
    /// The accessible name for a widget whose label is not rendered.
    accessible_label: Option<String>,
}

impl BuiltinChrome {
    fn new(context: &render::ControlRenderContext, projection: &handle::NodeProjection) -> Self {
        let presentation = context.presentation();
        let facets = context.control();
        let replacement_label = facets
            .write_only_replacement
            .as_ref()
            .map(|replacement| replacement.label.clone());
        let accessible_label = (!presentation.label_visible).then(|| {
            replacement_label
                .clone()
                .unwrap_or_else(|| presentation.label.clone())
        });
        Self {
            element_id: presentation.element_id.clone(),
            name: facets.name.clone(),
            kind: facets.kind,
            label: presentation.label.clone(),
            label_visible: presentation.label_visible,
            invalid: presentation.invalid,
            described_by: presentation.described_by(),
            supplements: presentation.present_help(),
            presented_findings: presentation.present_findings(),
            presence_actions: scalar_presence_actions(&presentation.presence),
            value_state_attribute: value_state_attribute(projection.value_state),
            replacement_label,
            accessible_label,
        }
    }

    /// The visible label text: the replacement label for editable write-only widgets.
    fn widget_label(&self, write_only: bool) -> String {
        if write_only {
            self.replacement_label.clone().unwrap_or_default()
        } else {
            self.label.clone()
        }
    }

    /// Renders the visible label for the primary element, or nothing while the label is hidden.
    fn label(&self, text: String) -> Element {
        let element_id = self.element_id.clone();
        rsx! {
            if self.label_visible {
                label { r#for: element_id, "{text}" }
            }
        }
    }

    /// Renders a read-only node as noninteractive `output`, as every built-in kind does.
    fn read_only_output(self, display_value: String) -> Element {
        let label = self.label(self.label.clone());
        rsx! {
            div {
                class: "schemaform-control",
                "data-schemaform-control": self.kind.data_attribute(),
                {label}
                output {
                    id: self.element_id,
                    name: self.name,
                    tabindex: "-1",
                    "data-read-only": "",
                    "aria-invalid": self.invalid,
                    "aria-label": self.accessible_label,
                    "aria-describedby": self.described_by,
                    "data-value-state": self.value_state_attribute,
                    "{display_value}"
                }
                {self.supplements}
                {self.presented_findings}
            }
        }
    }
}

/// The built-in string, number, and integer control.
///
/// It is rendered from the public [`render::ControlRenderContext`] and [`edit::use_text_edit`]
/// exactly as a custom renderer would be, so the hook is proven complete by the built-in running
/// on it. The host [`ControlHost`] computes the context and records renderer entry; this child
/// owns the widget. It displays `value` and therefore re-renders per keystroke anyway; the hook's
/// stable handles matter to widgets that receive them as props.
#[allow(non_snake_case)]
fn BuiltinTextControl(props: BuiltinControlProps) -> Element {
    let context = &props.context;
    let edit = use_text_edit(context);
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let chrome = BuiltinChrome::new(context, &projection);
    let display_value = edit.value.cloned();
    if projection.read_only {
        return chrome.read_only_output(display_value);
    }
    let facets = context.control();
    let write_only = facets.write_only;
    let required = facets.required;
    let label = chrome.label(chrome.widget_label(write_only));
    let incompatible_value = incompatible_value(&projection);
    rsx! {
        div {
            class: "schemaform-control",
            "data-schemaform-control": chrome.kind.data_attribute(),
            {label}
            input {
                id: chrome.element_id,
                name: chrome.name,
                r#type: if write_only { "password" } else { "text" },
                inputmode: chrome.kind.input_mode(),
                value: display_value,
                "data-write-only-replacement": write_only.then_some(""),
                required,
                "aria-invalid": chrome.invalid,
                "aria-label": chrome.accessible_label,
                "aria-describedby": chrome.described_by,
                readonly: edit.read_only,
                "data-value-state": chrome.value_state_attribute,
                oninput: move |event| edit.input.call(event.value()),
                oncompositionstart: move |_| edit.composition_start.call(()),
                oncompositionend: move |_| edit.composition_end.call(()),
                onblur: move |_| edit.blur.call(()),
            }
            {chrome.supplements}
            if let Some(incompatible_value) = incompatible_value {
                output { "data-incompatible-value": "", "{incompatible_value}" }
            }
            {chrome.presence_actions}
            {chrome.presented_findings}
        }
    }
}

/// The built-in boolean control: a native checkbox, or a replacement select for a write-only
/// boolean whose value must not be echoed.
///
/// Built on [`edit::use_boolean_edit`] and the public context, as a custom renderer would be.
#[allow(non_snake_case)]
fn BuiltinBooleanControl(props: BuiltinControlProps) -> Element {
    let context = &props.context;
    let edit = use_boolean_edit(context);
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let chrome = BuiltinChrome::new(context, &projection);
    if projection.read_only {
        return chrome.read_only_output(edit::display_text(&projection));
    }
    let facets = context.control();
    let required = facets.required;
    let disabled = facets.disabled;
    if let (Some(replacement), Some(labels)) = (
        facets.write_only_replacement.clone(),
        facets.boolean_labels.clone(),
    ) {
        let label = chrome.label(chrome.widget_label(true));
        return rsx! {
            div {
                class: "schemaform-control",
                "data-schemaform-control": chrome.kind.data_attribute(),
                {label}
                select {
                    id: chrome.element_id,
                    name: chrome.name,
                    value: "",
                    "data-write-only-replacement": "",
                    required,
                    "aria-invalid": chrome.invalid,
                    "aria-label": chrome.accessible_label,
                    "aria-describedby": chrome.described_by,
                    "data-value-state": chrome.value_state_attribute,
                    onchange: move |event| {
                        // The placeholder is disabled, so only the two value options reach here;
                        // the hook puts the select back on the placeholder after every write.
                        let value = match event.value().as_str() {
                            "true" => Some(true),
                            "false" => Some(false),
                            _ => None,
                        };
                        if let Some(value) = value {
                            edit.set.call(Some(value));
                        }
                    },
                    onblur: move |_| edit.blur.call(()),
                    option { value: "", disabled: true, selected: true, "{replacement.placeholder}" }
                    option { value: "false", "{labels.false_label}" }
                    option { value: "true", "{labels.true_label}" }
                }
                {chrome.supplements}
                {chrome.presence_actions}
                {chrome.presented_findings}
            }
        };
    }
    let checked = edit.checked.cloned().unwrap_or(false);
    let incompatible_value = incompatible_value(&projection);
    // The checkbox's label follows the widget, unlike every other built-in kind.
    let label = chrome.label(chrome.label.clone());
    rsx! {
        div {
            class: "schemaform-control",
            "data-schemaform-control": chrome.kind.data_attribute(),
            input {
                id: chrome.element_id.clone(),
                name: chrome.name,
                r#type: "checkbox",
                checked,
                disabled,
                "aria-required": required,
                "aria-invalid": chrome.invalid,
                "aria-label": chrome.accessible_label,
                "aria-describedby": chrome.described_by,
                "data-value-state": chrome.value_state_attribute,
                oninput: move |event| edit.set.call(Some(event.checked())),
                onblur: move |_| edit.blur.call(()),
            }
            {label}
            {chrome.supplements}
            if let Some(incompatible_value) = incompatible_value {
                output { "data-incompatible-value": "", "{incompatible_value}" }
            }
            {chrome.presence_actions}
            {chrome.presented_findings}
        }
    }
}

/// The built-in choice control: a native select over opaque option identities.
///
/// Built on [`edit::use_choice_edit`] and the public context, as a custom renderer would be.
#[allow(non_snake_case)]
fn BuiltinChoiceControl(props: BuiltinControlProps) -> Element {
    let context = &props.context;
    let edit = use_choice_edit(context);
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let chrome = BuiltinChrome::new(context, &projection);
    if projection.read_only {
        return chrome.read_only_output(edit::display_text(&projection));
    }
    let facets = context.control();
    let write_only = facets.write_only;
    let required = facets.required;
    let disabled = facets.disabled;
    let label = chrome.label(chrome.widget_label(write_only));
    let selected = edit.selected.cloned();
    let selected_value = selected
        .as_ref()
        .map(|identity| identity.as_str().to_owned())
        .unwrap_or_default();
    let placeholder_selected = selected.is_none();
    let placeholder_hidden = !write_only
        && !matches!(
            projection.value_state,
            Some(schemaform::form::ScalarValueState::Incompatible)
        );
    let placeholder_label = match &facets.write_only_replacement {
        Some(replacement) => replacement.placeholder.clone(),
        None if placeholder_hidden => String::new(),
        None => edit::display_text(&projection),
    };
    // The event handler maps the select's DOM value back to an opaque identity, so it needs its
    // own copy of the options the option list below consumes.
    let options = edit.options.clone();
    let lookup = edit.options;
    rsx! {
        div {
            class: "schemaform-control",
            "data-schemaform-control": chrome.kind.data_attribute(),
            {label}
            select {
                id: chrome.element_id,
                name: chrome.name,
                value: selected_value,
                "data-write-only-replacement": write_only.then_some(""),
                disabled,
                required,
                "aria-invalid": chrome.invalid,
                "aria-label": chrome.accessible_label,
                "aria-describedby": chrome.described_by,
                "data-value-state": chrome.value_state_attribute,
                onchange: move |event| {
                    let identity = lookup
                        .iter()
                        .find(|option| option.identity.as_str() == event.value())
                        .map(|option| option.identity.clone());
                    edit.select.call(identity);
                },
                onblur: move |_| edit.blur.call(()),
                option {
                    value: "",
                    disabled: true,
                    hidden: placeholder_hidden,
                    selected: placeholder_selected,
                    "{placeholder_label}"
                }
                for option in options {
                    option {
                        value: option.identity.as_str().to_owned(),
                        selected: selected.as_ref() == Some(&option.identity),
                        "{option.label}"
                    }
                }
            }
            {chrome.supplements}
            {chrome.presence_actions}
            {chrome.presented_findings}
        }
    }
}

/// The built-in constant control: noninteractive output of a fixed value, with the presence
/// affordances that can still repair it.
///
/// Constants have no edit hook; the output comes from the presentation and facets alone.
#[allow(non_snake_case)]
fn BuiltinConstantControl(props: BuiltinControlProps) -> Element {
    let context = &props.context;
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let chrome = BuiltinChrome::new(context, &projection);
    let display_value = edit::display_text(&projection);
    if projection.read_only {
        return chrome.read_only_output(display_value);
    }
    let text = context
        .control()
        .write_only_status
        .clone()
        .unwrap_or(display_value);
    let label = chrome.label(chrome.label.clone());
    rsx! {
        div {
            class: "schemaform-control",
            "data-schemaform-control": chrome.kind.data_attribute(),
            {label}
            output {
                id: chrome.element_id,
                name: chrome.name,
                tabindex: "-1",
                "aria-invalid": chrome.invalid,
                "aria-label": chrome.accessible_label,
                "aria-describedby": chrome.described_by,
                "data-value-state": chrome.value_state_attribute,
                "{text}"
            }
            {chrome.supplements}
            {chrome.presence_actions}
            {chrome.presented_findings}
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function schemaformResynchronizeControlValue(id, value) {
    const control = document.getElementById(id);
    if (control) control.value = value;
}

export function schemaformResynchronizeBoolean(id, checked) {
    const control = document.getElementById(id);
    if (!control) return;
    if (control instanceof HTMLSelectElement) {
        control.value = checked === undefined ? "" : String(checked);
    } else {
        control.checked = checked === true;
    }
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = schemaformResynchronizeControlValue)]
    fn resynchronize_control_value(control_id: &str, value: &str);

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = schemaformResynchronizeBoolean)]
    fn resynchronize_boolean(control_id: &str, checked: Option<bool>);
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn resynchronize_control_value(_control_id: &str, _value: &str) {}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn resynchronize_boolean(_control_id: &str, _checked: Option<bool>) {}

fn render_local_findings(
    form: &render::BoundForm,
    findings: Vec<render::FindingDescriptor>,
    target_id: String,
) -> Element {
    let context = render::FindingCollectionContext::local(
        findings,
        render::TargetFocusAction::new(target_id),
    );
    rsx! {
        FindingCollectionPresentation {
            form: form.clone(),
            context,
        }
    }
}

fn validation_finding_fallback(finding: &schemaform::ValidationFinding) -> String {
    if finding.code() == "minimum"
        && let Some(limit) = finding.parameters().get("limit")
    {
        return format!("Value must be at least {limit}.");
    }
    format!("Value does not satisfy {}.", finding.code())
}

pub use edit::{
    BooleanEdit, ChoiceEdit, ChoiceOption, TextEdit, use_boolean_edit, use_choice_edit,
    use_text_edit,
};
pub use handle::{
    ChoiceIdentity, ChoiceOptionProjection, CollectionActions, ControlActions, FormHandle,
    FormReader, HandleError, HandleTransactionError, NodeProjection, NodeReader, use_form,
};
pub use render::{
    Affordance, AffordanceKind, BindError, BindFinding, BoundForm, BuiltinControlRenderer,
    ControlFacets, ControlKind, ControlMatcher, ControlRegistry, ControlRenderContext,
    ControlRenderer, ExtensionHandler, ExtensionOccurrence, ExtensionPrepareError,
    ExtensionRenderContext, FindingCollectionPresenter, Help, Localizer, NodePresentation,
    PreparedExtension, PreparedExtensions, RenderConfiguration,
};
#[cfg(schemaform_test_validation_faults)]
pub use render_observation::{RenderEvent, RenderNodeKind, RenderObservation, RenderObserver};

#[cfg(test)]
mod tests {
    use super::BuiltinMessage;
    use serde_json::json;

    #[test]
    fn built_in_message_catalog_has_stable_fallbacks_and_parameters() {
        let cases = [
            (
                BuiltinMessage::Submit,
                "schemaform.submit.label",
                "Submit",
                json!({}),
            ),
            (
                BuiltinMessage::FindingSummary,
                "schemaform.finding-summary.label",
                "Finding summary",
                json!({}),
            ),
            (
                BuiltinMessage::ArrayItem {
                    array_label: "Rows".to_owned(),
                },
                "schemaform.array.item.label",
                "Rows item",
                json!({ "array_label": "Rows" }),
            ),
            (
                BuiltinMessage::ArrayInsertBefore {
                    item_label: "Entry".to_owned(),
                },
                "schemaform.array.insert-before.label",
                "Insert Entry before",
                json!({ "item_label": "Entry" }),
            ),
            (
                BuiltinMessage::ArrayMoveUp {
                    item_label: "Entry".to_owned(),
                },
                "schemaform.array.move-up.label",
                "Move Entry up",
                json!({ "item_label": "Entry" }),
            ),
            (
                BuiltinMessage::ArrayMoveDown {
                    item_label: "Entry".to_owned(),
                },
                "schemaform.array.move-down.label",
                "Move Entry down",
                json!({ "item_label": "Entry" }),
            ),
            (
                BuiltinMessage::ArrayRemove {
                    item_label: "Entry".to_owned(),
                },
                "schemaform.array.remove.label",
                "Remove Entry",
                json!({ "item_label": "Entry" }),
            ),
            (
                BuiltinMessage::ArrayAdd {
                    item_label: "Entry".to_owned(),
                },
                "schemaform.array.add.label",
                "Add Entry",
                json!({ "item_label": "Entry" }),
            ),
            (
                BuiltinMessage::ArrayInsertBeforeAt {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.insert-before-position.label",
                "Insert Entry before position 2",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayMoveUpAt {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.move-up-position.label",
                "Move Entry at position 2 up",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayMoveDownAt {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.move-down-position.label",
                "Move Entry at position 2 down",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayRemoveAt {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.remove-position.label",
                "Remove Entry at position 2",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayInserted {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.inserted.announcement",
                "Entry inserted at position 2.",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayMovedUp {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.moved-up.announcement",
                "Entry moved up to position 2.",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayMovedDown {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.moved-down.announcement",
                "Entry moved down to position 2.",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayRemoved {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.removed.announcement",
                "Entry removed from position 2.",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayAdded {
                    item_label: "Entry".to_owned(),
                    position: 2,
                },
                "schemaform.array.added.announcement",
                "Entry added at position 2.",
                json!({ "item_label": "Entry", "position": 2 }),
            ),
            (
                BuiltinMessage::ArrayMaterialized {
                    array_label: "Entries".to_owned(),
                },
                "schemaform.array.materialized.announcement",
                "Entries added.",
                json!({ "array_label": "Entries" }),
            ),
            (
                BuiltinMessage::ArrayReplaced {
                    array_label: "Entries".to_owned(),
                },
                "schemaform.array.replaced.announcement",
                "Entries replaced.",
                json!({ "array_label": "Entries" }),
            ),
            (
                BuiltinMessage::ArrayCleared {
                    array_label: "Entries".to_owned(),
                },
                "schemaform.array.cleared.announcement",
                "Entries removed.",
                json!({ "array_label": "Entries" }),
            ),
            (
                BuiltinMessage::PresenceAdd {
                    label: "Field".to_owned(),
                },
                "schemaform.presence.add.label",
                "Add Field",
                json!({ "label": "Field" }),
            ),
            (
                BuiltinMessage::PresenceSet {
                    label: "Field".to_owned(),
                },
                "schemaform.presence.set.label",
                "Set Field",
                json!({ "label": "Field" }),
            ),
            (
                BuiltinMessage::PresenceSetNull {
                    label: "Field".to_owned(),
                },
                "schemaform.presence.set-null.label",
                "Set Field to null",
                json!({ "label": "Field" }),
            ),
            (
                BuiltinMessage::PresenceRemove {
                    label: "Field".to_owned(),
                },
                "schemaform.presence.remove.label",
                "Remove Field",
                json!({ "label": "Field" }),
            ),
            (
                BuiltinMessage::PresenceReplace {
                    label: "Field".to_owned(),
                },
                "schemaform.presence.replace.label",
                "Replace Field",
                json!({ "label": "Field" }),
            ),
            (
                BuiltinMessage::WriteOnlyReplace {
                    label: "Secret".to_owned(),
                },
                "schemaform.write-only.replace.label",
                "Replace Secret",
                json!({ "label": "Secret" }),
            ),
            (
                BuiltinMessage::WriteOnlyReplacementPlaceholder {
                    label: "Secret".to_owned(),
                },
                "schemaform.write-only.replacement-placeholder",
                "Choose replacement",
                json!({ "label": "Secret" }),
            ),
            (
                BuiltinMessage::BooleanFalse,
                "schemaform.boolean.false",
                "False",
                json!({}),
            ),
            (
                BuiltinMessage::BooleanTrue,
                "schemaform.boolean.true",
                "True",
                json!({}),
            ),
            (
                BuiltinMessage::WriteOnlyNotSet {
                    label: "Secret".to_owned(),
                },
                "schemaform.write-only.not-set.status",
                "Value is not set",
                json!({ "label": "Secret" }),
            ),
            (
                BuiltinMessage::WriteOnlyNeedsReplacement {
                    label: "Secret".to_owned(),
                },
                "schemaform.write-only.needs-replacement.status",
                "Value needs replacement",
                json!({ "label": "Secret" }),
            ),
            (
                BuiltinMessage::WriteOnlySet {
                    label: "Secret".to_owned(),
                },
                "schemaform.write-only.set.status",
                "Value is set",
                json!({ "label": "Secret" }),
            ),
        ];

        let mut keys = std::collections::BTreeSet::new();
        for (message, expected_key, expected_fallback, expected_parameters) in cases {
            let descriptor = message.descriptor();
            assert_eq!(descriptor.key.as_deref(), Some(expected_key));
            assert_eq!(descriptor.fallback, expected_fallback);
            assert_eq!(descriptor.parameters, expected_parameters);
            assert!(
                keys.insert(expected_key),
                "duplicate built-in key {expected_key}"
            );
        }
    }
}
