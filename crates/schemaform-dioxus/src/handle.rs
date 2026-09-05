//! Browser-local form ownership, reactive readers, and node-scoped actions.

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
    CapabilityFinding, DataRevision, DataSchemaAnnotations, ExternalFinding, ExternalFindingBatch,
    Form, FormBuildError, FormDefinition, InstanceIdentity, ItemIdentity, JsonPointer,
    StateRevision, SubmissionPreparation, Transition, ValidationFinding,
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

    pub(crate) fn remove_value(&self, target: InstanceIdentity) -> Result<Transition, HandleError> {
        self.apply_user_operation(|form| form.user().remove_value(target))
    }

    pub(crate) fn materialize(&self, target: InstanceIdentity) -> Result<Transition, HandleError> {
        self.apply_user_operation(|form| form.user().materialize(target))
    }

    pub(crate) fn blur(&self, target: InstanceIdentity) -> Result<Transition, HandleError> {
        self.apply_user_operation(|form| form.user().blur(target))
    }

    fn apply_user_operation(
        &self,
        operation: impl FnOnce(&mut Form) -> Result<Transition, schemaform::form::UserOperationError>,
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
                    node_versions.insert(identity, Signal::new_in_scope(0, self.inner.owner_scope));
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

/// Projects the form's visible findings.
///
/// `FindingView` is `#[non_exhaustive]`; a family this adapter does not know is left out of the
/// projection rather than aborting the application from inside a mutation. The debug assertion
/// keeps the lockstep release of the two crates honest: a new core family fails the adapter's
/// tests until it is projected.
fn project_visible_findings(form: &Form) -> Vec<FindingProjection> {
    form.view()
        .visible_findings()
        .filter_map(|finding| match finding {
            schemaform::FindingView::Validation { target, finding } => {
                Some(FindingProjection::Validation {
                    target,
                    finding: finding.clone(),
                })
            }
            schemaform::FindingView::ValidationFindingsTruncated { target, retained } => {
                Some(FindingProjection::ValidationFindingsTruncated { target, retained })
            }
            schemaform::FindingView::Indeterminate { target, reason } => {
                Some(FindingProjection::Indeterminate {
                    target,
                    reason: reason.clone(),
                })
            }
            schemaform::FindingView::Capability { target, finding } => {
                Some(FindingProjection::Capability {
                    target,
                    finding: finding.clone(),
                })
            }
            schemaform::FindingView::External {
                target,
                source,
                finding,
            } => Some(FindingProjection::External {
                target,
                source: source.to_owned(),
                finding: finding.clone(),
            }),
            schemaform::FindingView::Parse { target, kind } => {
                Some(FindingProjection::Parse { target, kind })
            }
            _ => {
                debug_assert!(
                    false,
                    "the adapter must project every core finding family; one is missing"
                );
                None
            }
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

impl NodeProjection {
    /// The text a control shows for this node when it is not being edited, as the built-in
    /// controls show it: the retained edit buffer or canonical spelling in [`Self::value`],
    /// else [`Self::current_data`] spelled as JSON, else nothing.
    ///
    /// A write-only value is never spelled out; only an in-progress edit buffer is shown, so a
    /// renderer that presents a read-only or constant node through this method keeps the
    /// write-only rule without restating it.
    pub fn display_text(&self) -> String {
        if self.write_only && self.edit_buffer.is_none() {
            return String::new();
        }
        self.value.clone().unwrap_or_else(|| {
            self.current_data
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_default()
        })
    }
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
        ExtensionNamespace, FindingVisibility, FindingVisibilityPolicy, JsonPointer, WidgetSymbol,
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
            let definition =
                FormDefinition::compiler(json!({
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
                Control::new(Binding::root(JsonPointer::parse("/rows").unwrap())).item_template(
                    UiElement::Control(
                        Control::new(Binding::item(JsonPointer::parse("").unwrap()))
                            .widget(item_widget.clone()),
                    ),
                ),
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
