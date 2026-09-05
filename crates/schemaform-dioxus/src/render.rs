//! Render preflight, customization traits, and authority-limited render contexts.

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
    /// order.
    ///
    /// Scalar controls offer set, set null, remove value, and replace: set only while the
    /// value is missing or null and a creation seed exists; replace only while the core
    /// allows replacement and a seed exists; set null and remove value exactly when the core
    /// allows them. Homogeneous arrays offer materialize, replace, and remove value under the
    /// same seed rules; invoking one of them also announces the change and focuses the
    /// array's primary element. Renderers place these affordances; they do not reconstruct
    /// the rules. The list is empty for every other node kind.
    pub presence: Vec<Affordance>,
    /// The serialized current value while the node cannot edit it but the core allows
    /// replacement, as the built-in shows beside its replace affordance.
    ///
    /// `Some` for a scalar control whose value is incompatible (or null where null is not
    /// accepted) while text input is rejected, and for a container whose value is
    /// replaceable, unless the node is write-only. A renderer that shows it must not treat it
    /// as editable text.
    pub incompatible_value: Option<String>,
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
            .field("incompatible_value", &self.incompatible_value)
            .finish_non_exhaustive()
    }
}

impl NodePresentation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        form: BoundForm,
        element_id: String,
        label: String,
        label_visible: bool,
        help: Option<Help>,
        findings: Vec<FindingDescriptor>,
        presence: Vec<Affordance>,
        incompatible_value: Option<String>,
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
            incompatible_value,
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
/// Presence operations on scalar controls and homogeneous arrays, item operations on
/// homogeneous arrays, and the form's submit are produced today. The enum grows as further
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
    /// Materializes a missing container from its creation seed.
    Materialize,
    /// Appends one seeded item to a collection.
    Append,
    /// Inserts one seeded item before the item the affordance was computed for.
    InsertBefore,
    /// Moves the item one position towards the start of its collection.
    MoveUp,
    /// Moves the item one position towards the end of its collection.
    MoveDown,
    /// Removes the item from its collection.
    RemoveItem,
    /// Finalizes edit buffers and prepares the form for submission.
    ///
    /// A ready outcome reaches `SchemaForm::on_submit`; a blocked outcome presents the
    /// findings and focuses the finding summary instead.
    Submit,
}

/// A localized, pre-authorized user action handed to a renderer.
///
/// Invoking an affordance performs the core operation on the node it was computed for and
/// reports any failure to the host's `SchemaForm::on_error`; collection affordances also
/// announce the change through the collection's live region and move focus. The renderer only
/// places it. The adapter recomputes the list of affordances on every node render, so an
/// affordance is present exactly while the core allows its operation.
///
/// Two affordances compare equal when their `kind`, `label`, `id`, and `accessible_name` are
/// equal. `invoke` is excluded because an affordance's behaviour is fixed by its node and
/// kind, so a component that memoizes on an affordance and keeps an earlier `invoke` performs
/// the same operation — for as long as the node that produced it is mounted.
///
/// `invoke` is owned by the scope that computed it: the control host for presence affordances,
/// the item host for item affordances, the collection for the append affordance, and the form
/// for the submit affordance. Invoking it after that scope has been dropped (an item affordance
/// retained past the item's removal, for example) panics inside Dioxus. Renderers therefore
/// place affordances in the render that hands them out and do not store them in state that
/// outlives the node — a collection-level context menu, say, must be rebuilt from the current
/// contexts rather than from affordances captured earlier.
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
    /// by `-set-value`, `-set-null`, `-remove-value`, `-replace-value`, or `-materialize`.
    /// For the submit affordance it is [`ShellContext::form_id`] followed by `-submit`. For a
    /// collection's append affordance it is the array's element id followed by `-append`; for
    /// item affordances it is the item root's id followed by `-insert-before`, `-move-up`,
    /// `-move-down`, or `-remove` (see [`CollectionItemContext::row_id`]).
    pub id: String,
    /// Localized accessible name when it differs from the visible `label`.
    ///
    /// `None` means `label` is the accessible name. `Some` is used when the visible text
    /// alone would be ambiguous, such as a positional variant of an item action; a renderer
    /// then sets the triggering element's `aria-label` to this value, whether it shows
    /// `label` as text or renders an icon. The four item affordances carry `Some`; presence,
    /// append, and submit affordances carry `None`.
    pub accessible_name: Option<String>,
    /// Performs the operation and reports failures to the host's `SchemaForm::on_error`.
    ///
    /// Install this on an event callback rather than calling it during rendering.
    pub invoke: Callback<()>,
}

impl Affordance {
    /// Renders the affordance as the built-in does: a `button[type="button"]` carrying
    /// [`Affordance::id`], the built-in's marker attribute for its kind (`data-set-value`,
    /// `data-set-null`, `data-remove-value`, `data-replace-value`, `data-materialize`,
    /// `data-append-item`, `data-insert-item-before`, `data-move-item-up`,
    /// `data-move-item-down`, `data-remove-item`, `data-submit`), `aria-label` from
    /// [`Affordance::accessible_name`] when present, and [`Affordance::label`] as its text, with
    /// `onclick` installed on [`Affordance::invoke`].
    ///
    /// Renderers that want different markup render the fields themselves and keep the `id`, the
    /// accessible name, and the `invoke` on an event handler — the three things the adapter's
    /// focus, announcements, and operations depend on.
    pub fn present(&self) -> Element {
        let affordance = self.clone();
        let kind = affordance.kind;
        let marker = |expected: AffordanceKind| (kind == expected).then_some("");
        dioxus::prelude::rsx! {
            button {
                key: "{affordance.id}",
                id: affordance.id.clone(),
                r#type: "button",
                "data-set-value": marker(AffordanceKind::Set),
                "data-set-null": marker(AffordanceKind::SetNull),
                "data-remove-value": marker(AffordanceKind::RemoveValue),
                "data-replace-value": marker(AffordanceKind::Replace),
                "data-materialize": marker(AffordanceKind::Materialize),
                "data-append-item": marker(AffordanceKind::Append),
                "data-insert-item-before": marker(AffordanceKind::InsertBefore),
                "data-move-item-up": marker(AffordanceKind::MoveUp),
                "data-move-item-down": marker(AffordanceKind::MoveDown),
                "data-remove-item": marker(AffordanceKind::RemoveItem),
                "data-submit": marker(AffordanceKind::Submit),
                "aria-label": affordance.accessible_name.clone(),
                onclick: move |_| affordance.invoke.call(()),
                "{affordance.label}"
            }
        }
    }
}

impl PartialEq for Affordance {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.label == other.label
            && self.id == other.id
            && self.accessible_name == other.accessible_name
    }
}

impl fmt::Debug for Affordance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Affordance")
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("id", &self.id)
            .field("accessible_name", &self.accessible_name)
            .finish_non_exhaustive()
    }
}

/// Host-supplied presentation for the form shell: where the finding summary sits, how the
/// body is framed, and what triggers submission.
///
/// The adapter keeps the `<form>` element itself (its `novalidate`, submit handling,
/// focusability, and error-handler context) and the finding-summary region wrapper; the shell
/// renderer returns the form's *contents*. The method runs synchronously during Dioxus
/// rendering and is not a hook-safe call site; a renderer that needs hooks renders a child
/// component and passes the context as props ([`ShellContext`] is `PartialEq`).
///
/// Structure renderers are fixed at [`RenderConfiguration::bind`]. Unlike presenters and the
/// localizer they are not signal-swappable: their output is the parent template of every
/// node, so swapping one would remount every child scope. Changing a structure renderer means
/// rebinding the form.
pub trait ShellRenderer: 'static {
    /// Returns the contents of the adapter-owned `<form>` element.
    ///
    /// The output must include [`ShellContext::summary`] and [`ShellContext::body`], and
    /// should place [`ShellContext::submit`] once: either as a `type="submit"` button, which
    /// submits through the form element, or as any element that calls the affordance's
    /// `invoke`. Rendering both on one element submits twice.
    fn shell(&self, context: ShellContext) -> Element;
}

/// The adapter-computed context for one form shell.
///
/// Children arrive as pre-keyed elements; the renderer never sees bound nodes or recursion.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct ShellContext {
    /// DOM `id` of the adapter-owned `<form>` element that contains the shell's output.
    pub form_id: String,
    /// The finding summary region, including its adapter-owned wrapper element.
    ///
    /// The wrapper carries `{form_id}-summary`, `role="region"`, a localized `aria-label`,
    /// and `tabindex="-1"`; a blocked submission focuses it. Must be placed.
    pub summary: Element,
    /// Every root-level node of the form, in definition order. Must be placed.
    pub body: Element,
    /// The submit affordance: [`AffordanceKind::Submit`] with the localized submit label and
    /// the id `{form_id}-submit`.
    pub submit: Affordance,
}

impl fmt::Debug for ShellContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellContext")
            .field("form_id", &self.form_id)
            .field("submit", &self.submit)
            .finish_non_exhaustive()
    }
}

/// The built-in form shell: summary, body, then a `type="submit"` button.
///
/// The button carries the submit affordance's id and label and submits through the form
/// element, so pressing Enter in a text control and clicking the button take the same path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuiltinShell;

impl ShellRenderer for BuiltinShell {
    fn shell(&self, context: ShellContext) -> Element {
        dioxus::prelude::rsx! {
            {context.summary}
            {context.body}
            button { id: context.submit.id.clone(), r#type: "submit", "{context.submit.label}" }
        }
    }
}

/// Host-supplied presentation for one homogeneous array: the chrome around its items, each
/// item's chrome, the append and item affordances, and the empty state.
///
/// The adapter keeps ownership of everything a renderer could otherwise break: item identity
/// and keying (one keyed host scope per item, keyed by instance identity), the row wrapper
/// carrying [`CollectionItemContext::row_id`], focus after a mutation, and the live-region
/// announcement. Affordances already perform their operation, report failures, announce, and
/// move focus; the renderer places them and must give each triggering element its
/// [`Affordance::id`], because focus after a move targets those ids.
///
/// Both methods run synchronously during Dioxus rendering and are not hook-safe call sites; a
/// renderer that needs hooks or per-item state renders a child component and passes the
/// context as props (both contexts are `PartialEq`). The two methods are not called together:
/// the collection re-renders on every announcement while item hosts memoize on their props,
/// so `collection` may run without `collection_item` running for any item. A renderer must not
/// carry state between the two calls through a side table.
///
/// Structure renderers are fixed at [`RenderConfiguration::bind`]; see [`ShellRenderer`] for
/// why they are not signal-swappable.
pub trait CollectionRenderer: 'static {
    /// Renders the collection chrome around the adapter-keyed items.
    ///
    /// The root element must carry [`NodePresentation::element_id`] and should be focusable
    /// (`tabindex="-1"`), because the container presence affordances focus it. The output
    /// must place [`CollectionContext::items`] and [`CollectionContext::announcement`];
    /// omitting the announcement silently removes screen-reader feedback for every mutation.
    fn collection(&self, context: CollectionContext) -> Element;

    /// Renders one item's chrome around its children, inside the adapter-owned row wrapper.
    ///
    /// The output must place [`CollectionItemContext::children`]. The item root inside the
    /// children already carries the item's element id, and the adapter focuses it (or the
    /// first focusable element inside it) when an insert, append, or remove moves focus to an
    /// item, so the renderer's own controls may precede the children without stealing that
    /// focus.
    fn collection_item(&self, context: CollectionItemContext) -> Element;
}

/// The adapter-computed context for one homogeneous array node.
///
/// Items arrive as one pre-keyed element; the renderer never sees bound nodes, instance
/// identities, or recursion.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct CollectionContext {
    /// Localized label, help, findings, invalid state, and the container presence
    /// affordances (materialize, replace, remove value) for the array node.
    ///
    /// [`NodePresentation::incompatible_value`] is `Some` while the array's current data is
    /// not an array and the core allows replacement.
    pub presentation: NodePresentation,
    /// Localized singular noun for one item: the authored item label, or the built-in
    /// `{label} item`.
    ///
    /// The renderer cannot derive it (there is no localization service on the context), and
    /// composing it into a sentence is not grammatically safe across locales; use it as a
    /// title or a name, not as a sentence fragment.
    pub item_label: String,
    /// Number of items currently in the collection.
    ///
    /// `items` is opaque, so this is the only way a renderer can render an empty state.
    pub count: usize,
    /// Every item host in collection order, keyed by instance identity. Must be placed.
    pub items: Element,
    /// The append affordance ([`AffordanceKind::Append`], id `{element_id}-append`) while the
    /// core allows appending.
    pub append: Option<Affordance>,
    /// The adapter-owned live region (`role="status"`, `aria-live="polite"`,
    /// `aria-atomic="true"`) that announces mutations. Must be placed; wrapping it in a
    /// visually hidden element is fine.
    pub announcement: Element,
    /// Prepared extension values for the array's UI-schema element.
    pub extensions: PreparedExtensions,
}

impl fmt::Debug for CollectionContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CollectionContext")
            .field("presentation", &self.presentation)
            .field("item_label", &self.item_label)
            .field("count", &self.count)
            .field("append", &self.append)
            .finish_non_exhaustive()
    }
}

/// The adapter-computed context for one homogeneous-array item.
///
/// The four item affordances are `Some` exactly while the core allows the operation for this
/// item right now; `move_up` is additionally `None` for the first item and `move_down` for
/// the last, so a renderer never computes first/last itself.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct CollectionItemContext {
    /// DOM `id` of the adapter-owned wrapper element around this item's output.
    ///
    /// The adapter reserves this id (the wrapper already carries it, together with
    /// `data-array-item`), the item root's id inside `children`, and the four item affordance
    /// ids. Renderers use `row_id` only as a prefix for their own ids (`{row_id}-title`);
    /// such ids cannot collide with any adapter id.
    pub row_id: String,
    /// One-based position of the item in the collection.
    pub position: usize,
    /// Number of items in the collection.
    pub count: usize,
    /// Localized singular noun for one item; the same value as
    /// [`CollectionContext::item_label`].
    pub item_label: String,
    /// The item's instantiated template. Must be placed.
    pub children: Element,
    /// Insert-before affordance ([`AffordanceKind::InsertBefore`]) while insertion is allowed.
    pub insert_before: Option<Affordance>,
    /// Move-up affordance ([`AffordanceKind::MoveUp`]) while moving is allowed and the item is
    /// not first.
    pub move_up: Option<Affordance>,
    /// Move-down affordance ([`AffordanceKind::MoveDown`]) while moving is allowed and the
    /// item is not last.
    pub move_down: Option<Affordance>,
    /// Remove affordance ([`AffordanceKind::RemoveItem`]) while removal is allowed.
    pub remove: Option<Affordance>,
}

impl fmt::Debug for CollectionItemContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CollectionItemContext")
            .field("row_id", &self.row_id)
            .field("position", &self.position)
            .field("count", &self.count)
            .field("item_label", &self.item_label)
            .field("insert_before", &self.insert_before)
            .field("move_up", &self.move_up)
            .field("move_down", &self.move_down)
            .field("remove", &self.remove)
            .finish_non_exhaustive()
    }
}

/// The built-in collection chrome: a focusable `fieldset` with a legend, help, presence
/// buttons, the item rows, the append button, the live region, and the local findings; each
/// item renders its children followed by its action buttons.
///
/// Every button carries its affordance id and a `data-*` marker for its operation
/// (`data-append-item`, `data-insert-item-before`, `data-move-item-up`, `data-move-item-down`,
/// `data-remove-item`, `data-materialize`, `data-replace-value`, `data-remove-value`). The
/// markers are the built-in's own hooks, not part of the renderer contract.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuiltinCollection;

impl CollectionRenderer for BuiltinCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        let presentation = context.presentation;
        let described_by = presentation.described_by();
        let help = presentation.present_help();
        let findings = presentation.present_findings();
        let presence = presentation.presence.clone();
        let incompatible_value = presentation.incompatible_value.clone();
        dioxus::prelude::rsx! {
            fieldset {
                id: presentation.element_id.clone(),
                class: "schemaform-group schemaform-array",
                "data-schemaform-array": "",
                "aria-invalid": presentation.invalid,
                "aria-describedby": described_by,
                tabindex: "-1",
                legend { "{presentation.label}" }
                {help}
                div { class: "schemaform-presence-actions",
                    if let Some(value) = incompatible_value {
                        output { "data-incompatible-value": "", "{value}" }
                    }
                    for affordance in presence {
                        {affordance.present()}
                    }
                }
                {context.items}
                if let Some(append) = context.append {
                    {append.present()}
                }
                {context.announcement}
                {findings}
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
        let actions = [
            context.insert_before,
            context.move_up,
            context.move_down,
            context.remove,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        dioxus::prelude::rsx! {
            {context.children}
            for affordance in actions {
                {affordance.present()}
            }
        }
    }
}

/// The structure renderers a bound form renders its non-control nodes and shell through.
///
/// Each slot holds one renderer for one structural node kind; an unset slot is the built-in,
/// so a bundle that replaces only some slots degrades to accessible unstyled output rather
/// than a missing region. A package that ships structure renderers exports a fully populated
/// bundle (by convention `fn structure() -> StructureRenderers`), and a host composes slots
/// from several packages with the `with_*` setters. There is deliberately no supertrait
/// bundling every slot: adding a slot is then additive for every existing renderer
/// implementation.
///
/// The bundle is fixed at [`RenderConfiguration::bind`]. It is not signal-swappable and
/// [`RenderConfiguration::rebind_presentation`] does not touch it; changing a structure
/// renderer means rebinding the form.
#[derive(Clone)]
pub struct StructureRenderers {
    shell: Arc<dyn ShellRenderer>,
    collection: Arc<dyn CollectionRenderer>,
}

impl Default for StructureRenderers {
    /// Every slot is its built-in.
    fn default() -> Self {
        Self {
            shell: Arc::new(BuiltinShell),
            collection: Arc::new(BuiltinCollection),
        }
    }
}

impl StructureRenderers {
    /// Replaces the shell renderer.
    pub fn with_shell(mut self, renderer: impl ShellRenderer) -> Self {
        self.shell = Arc::new(renderer);
        self
    }

    /// Replaces the collection renderer.
    pub fn with_collection(mut self, renderer: impl CollectionRenderer) -> Self {
        self.collection = Arc::new(renderer);
        self
    }

    /// Renders the form shell through the configured [`ShellRenderer`].
    pub(crate) fn render_shell(&self, context: ShellContext) -> Element {
        self.shell.shell(context)
    }

    /// Renders one homogeneous array through the configured [`CollectionRenderer`].
    pub(crate) fn render_collection(&self, context: CollectionContext) -> Element {
        self.collection.collection(context)
    }

    /// Renders one array item through the configured [`CollectionRenderer`].
    pub(crate) fn render_collection_item(&self, context: CollectionItemContext) -> Element {
        self.collection.collection_item(context)
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
    ) -> impl ExactSizeIterator<Item = (&ExtensionNamespace, &Arc<dyn PreparedExtension>)> {
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
    pub(crate) fn local(findings: Vec<FindingDescriptor>, target_focus: TargetFocusAction) -> Self {
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
    structure: StructureRenderers,
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
            structure: StructureRenderers::default(),
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
        let prepared_extensions = self.prepare_extensions(core_form.definition(), &mut findings);
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
                structure: self.structure.clone(),
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
    /// Controls, structure renderers, extensions, generated DOM identity, and the grid
    /// breakpoint remain those selected by the original bind; call
    /// [`RenderConfiguration::bind`] for those changes. Core form state is not changed.
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

    /// Replaces the structure renderer bundle; unset slots are the built-ins.
    ///
    /// The bundle is fixed at [`RenderConfiguration::bind`] and is not swapped by
    /// [`RenderConfiguration::rebind_presentation`]; changing a structure renderer means
    /// rebinding.
    pub fn structure(mut self, structure: StructureRenderers) -> Self {
        self.configuration.structure = structure;
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
    /// Bind-fixed structure renderers; see [`StructureRenderers`].
    pub(crate) structure: StructureRenderers,
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
    /// Prepared extension values for the array's UI-schema element, handed to the collection
    /// renderer. Decorators are applied around the array by [`decorate_bound_node`].
    pub(crate) extensions: PreparedExtensions,
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
        DefinitionNodeKind::TabPanel => Some(BoundTemplateNode::TabPanel(BoundTemplateTabPanel {
            definition: id,
            label: node
                .label_reference()
                .expect("authored tab panels contain title references")
                .clone(),
            children: children(),
        })),
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
