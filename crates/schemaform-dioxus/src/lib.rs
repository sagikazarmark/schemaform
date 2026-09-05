//! Accessible Dioxus browser rendering for [`schemaform::FormDefinition`].
//!
//! The crate keeps Dioxus state out of the core engine and provides explicit
//! control renderer, structure renderer, finding presenter, localization, and
//! extension seams, plus headless [`edit`] hooks that give custom renderers the
//! built-in editing behaviour. [`SchemaForm`] renders unstyled semantic HTML and
//! submits immutable [`schemaform::SubmissionSnapshot`] values.
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
mod render_observation;

pub mod edit;
pub mod handle;
pub mod render;

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
///
/// The adapter owns the `<form>` element, the finding-summary region wrapper, and the submit
/// handling; the bound form's [`render::ShellRenderer`] arranges the summary, the body, and the
/// submit affordance inside it.
pub fn SchemaForm(props: SchemaFormProps) -> Element {
    let operation_errors = use_context_provider(OperationErrorHandler::default);
    operation_errors.set(Some(props.on_error));
    let form_id = props.form.inner.form_id.clone();
    let submit = use_submit_callback(&props);
    let submit_affordance = render::Affordance {
        kind: render::AffordanceKind::Submit,
        label: localize_builtin(&props.form, BuiltinMessage::Submit),
        id: format!("{form_id}-submit"),
        accessible_name: None,
        invoke: submit,
    };
    let contents = props
        .form
        .inner
        .structure
        .render_shell(render::ShellContext {
            form_id: form_id.clone(),
            summary: rsx! { FindingSummary { form: props.form.clone() } },
            body: rsx! { FormBody { form: props.form.clone() } },
            submit: submit_affordance,
        });
    let mut grid_styles = format!(
        "#{form_id} .schemaform-grid{{display:grid;grid-template-columns:repeat(12,minmax(0,1fr))}}",
    );
    for span in 1..=12 {
        grid_styles.push_str(&format!(
            "#{form_id} .schemaform-grid-cell[data-compact-span='{span}']{{grid-column:span {span} / span {span}}}",
        ));
    }
    grid_styles.push_str(&format!(
        "@media (min-width:{}px){{",
        props.form.inner.grid_wide_breakpoint_css_px
    ));
    for span in 1..=12 {
        grid_styles.push_str(&format!(
            "#{form_id} .schemaform-grid-cell[data-wide-span='{span}']{{grid-column:span {span} / span {span}}}",
        ));
    }
    grid_styles.push('}');

    rsx! {
        style { dangerous_inner_html: grid_styles }
        form {
            id: form_id,
            class: "schemaform",
            "data-schemaform": "",
            novalidate: true,
            tabindex: "-1",
            onsubmit: move |event| {
                event.prevent_default();
                submit.call(());
            },
            {contents}
        }
    }
}

/// Creates the hook-stable submission callback behind the submit affordance and the form's
/// `submit` event.
///
/// Invocation finalizes edit buffers and prepares submission: a ready snapshot reaches
/// `on_submit`, a blocked outcome focuses the finding summary, and an adapter failure reaches
/// `on_error`. The closure is refreshed every render so it always calls the current props while
/// the callback identity stays fixed. This is a hook: call it at the same position on every
/// render.
fn use_submit_callback(props: &SchemaFormProps) -> Callback<()> {
    let form = props.form.clone();
    let summary_focus =
        render::TargetFocusAction::new(format!("{}-summary", props.form.inner.form_id));
    let on_submit = props.on_submit;
    let on_error = props.on_error;
    use_callback(move |()| match form.handle().prepare_submission() {
        Ok(preparation) => match preparation.into_parts().1 {
            SubmissionOutcome::Ready(snapshot) => on_submit.call(snapshot),
            SubmissionOutcome::Blocked(_) => summary_focus.focus(),
        },
        Err(error) => on_error.call(error),
    })
}

#[derive(Props, Clone, PartialEq)]
struct BoundFormProps {
    form: render::BoundForm,
}

/// The adapter-owned finding summary region: the focusable wrapper a blocked submission targets
/// and the summary presenter's output inside it.
///
/// This component subscribes to the summary projection, so summary changes re-render it alone.
#[allow(non_snake_case)]
fn FindingSummary(props: BoundFormProps) -> Element {
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
    }
}

/// The form body: every root-level bound node, keyed by instance identity.
///
/// The render plan is bind-fixed, so this component re-renders only when its props change; each
/// node subscribes to its own state.
#[allow(non_snake_case)]
fn FormBody(props: BoundFormProps) -> Element {
    if props.form.handle().ensure_live().is_err() {
        return rsx! {};
    }
    rsx! {
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

/// Instantiates one item's bound subtree from the array's item template.
///
/// The read is untracked on purpose: this only consumes the projection's structure — the control
/// binding and the child identities — which the core derives from the definition tree and the
/// item's position. Position and count are already props of the item host, so a structural
/// change re-renders it through them, while an edit inside the item re-renders only the control
/// that owns the edited node. A tracked read here would subscribe the item host (and the
/// finding summary, which walks the same template) to every node in the item, and item hosts
/// would no longer memoize on their props as the contract promises.
fn instantiate_array_template(
    form: &render::BoundForm,
    template: &render::BoundTemplateNode,
    identity: schemaform::InstanceIdentity,
    array_element_id: &str,
) -> Option<render::BoundNode> {
    let projection = form
        .handle()
        .node(identity)
        .ok()??
        .read_untracked()
        .ok()??;
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
/// distinct ids. `presence` is the node's current presence affordances: scalar controls compute
/// them with [`scalar_presence_affordances`], homogeneous arrays in [`HomogeneousArray`], other
/// containers pass an empty list. `incompatible_value` is the serialized value shown beside a
/// replace affordance ([`incompatible_value`] for controls, [`container_incompatible_value`] for
/// containers).
fn node_presentation(
    form: &render::BoundForm,
    projection: &handle::NodeProjection,
    element_id: &str,
    presence: Vec<render::Affordance>,
    incompatible_value: Option<String>,
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
        incompatible_value,
    )
}

#[derive(Props, Clone, PartialEq)]
struct HomogeneousArrayProps {
    form: render::BoundForm,
    array: render::BoundArray,
}

/// Where focus goes after a collection mutation, resolved against the DOM once the mutation has
/// rendered.
#[derive(Clone)]
enum ArrayFocusRequest {
    /// The first element in the list that exists in the document.
    Element(Vec<String>),
    /// An item by its id stem `S` ([`array_item_input_id`]): the item root `#S` if it is
    /// focusable, else the first focusable element inside it, else the first focusable element
    /// inside the adapter's wrapper `#S-row`. Trying the item root first keeps focus after an
    /// insert, append, or remove on the item's control regardless of where a collection renderer
    /// places its own buttons.
    Item(String),
}

/// The signals every collection affordance needs to announce a change and move focus.
///
/// Signals are `Copy`, so one bundle is captured by value into each `use_callback` closure in the
/// array component and its item hosts.
#[derive(Clone, Copy, PartialEq)]
struct ArrayFeedback {
    announcement: Signal<(u64, Option<ArrayAnnouncement>)>,
    pending_announcement: Signal<Option<(u64, ArrayAnnouncement)>>,
    pending_focus_target: Signal<Option<ArrayFocusRequest>>,
}

impl ArrayFeedback {
    fn announce(self, event: ArrayAnnouncement) {
        set_array_announcement(self.announcement, self.pending_announcement, event);
    }

    fn focus(mut self, request: ArrayFocusRequest) {
        self.pending_focus_target.set(Some(request));
    }
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
    // Hooks run before the availability guards below so the hook order is identical on every
    // render, including renders where the node has already been removed or disposed.
    let reader = props
        .form
        .handle()
        .node(props.array.identity)
        .ok()
        .flatten();
    let projection = reader
        .as_ref()
        .and_then(|reader| reader.read().ok().flatten());
    let operation_errors = dioxus_core::try_consume_context::<OperationErrorHandler>();
    let announcement = use_signal(|| (0_u64, None::<ArrayAnnouncement>));
    let mut pending_announcement = use_signal(|| None::<(u64, ArrayAnnouncement)>);
    let mut focus_target = use_signal(|| None::<ArrayFocusRequest>);
    let mut pending_focus_target = use_signal(|| None::<ArrayFocusRequest>);
    let feedback = ArrayFeedback {
        announcement,
        pending_announcement,
        pending_focus_target,
    };
    use_effect(move || {
        let pending = *pending_announcement.read();
        if let Some(pending) = pending {
            pending_announcement.write().take();
            let mut announcement = announcement;
            announcement.set((pending.0, Some(pending.1)));
        }
    });
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

    // Container presence and append are a fixed set per array, so their callbacks are hook-stable
    // here; the per-item affordances live in `CollectionItemHost`, one scope per item.
    let element_id = props.array.element_id.clone();
    let actions = reader.as_ref().map(handle::NodeReader::actions);
    let collection = reader.as_ref().map(handle::NodeReader::collection_actions);
    let seed = projection
        .as_ref()
        .and_then(|projection| projection.creation_seed.clone());
    /// One container presence operation; `None` when its precondition (a seed) is absent.
    type ContainerOperation = fn(
        &handle::ControlActions,
        Option<&Value>,
    )
        -> Option<Result<schemaform::Transition, handle::HandleError>>;
    let container_callback = |operation: ContainerOperation, event: ArrayAnnouncement| {
        let actions = actions.clone();
        let seed = seed.clone();
        let error_route = operation_errors.clone();
        let element_id = element_id.clone();
        use_callback(move |()| {
            if let Some(actions) = &actions
                && let Some(result) = operation(actions, seed.as_ref())
                && report_operation(&error_route, result)
            {
                feedback.focus(ArrayFocusRequest::Element(vec![element_id.clone()]));
                feedback.announce(event);
            }
        })
    };
    let materialize = container_callback(
        |actions, _| Some(actions.materialize()),
        ArrayAnnouncement::Materialized,
    );
    let replace = container_callback(
        |actions, seed| seed.map(|value| actions.replace_value(value.clone())),
        ArrayAnnouncement::Replaced,
    );
    let remove_value = container_callback(
        |actions, _| Some(actions.remove_value()),
        ArrayAnnouncement::Cleared,
    );
    let append = {
        let collection = collection.clone();
        let reader = reader.clone();
        let error_route = operation_errors.clone();
        let element_id = element_id.clone();
        use_callback(move |()| {
            let (Some(collection), Some(reader)) = (&collection, &reader) else {
                return;
            };
            let before = array_children(reader);
            if report_operation(&error_route, collection.append()) {
                let after = array_children(reader);
                focus_new_item(feedback, &element_id, &before, &after);
                feedback.announce(ArrayAnnouncement::Added {
                    position: after.len(),
                });
            }
        })
    };

    let Some(mut projection) = projection else {
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

    let operations = projection.allowed_operations;
    let presence = container_presence_affordances(
        &props.form,
        &projection,
        &element_id,
        ContainerPresenceCallbacks {
            materialize,
            replace,
            remove_value,
        },
    );
    let presentation = node_presentation(
        &props.form,
        &projection,
        &element_id,
        presence,
        container_incompatible_value(&projection),
    );

    let append_id = format!("{element_id}-append");
    let append = operations.can_append_item().then(|| render::Affordance {
        kind: render::AffordanceKind::Append,
        label: localize_builtin(
            &props.form,
            BuiltinMessage::ArrayAdd {
                item_label: item_label.clone(),
            },
        ),
        id: append_id.clone(),
        accessible_name: None,
        invoke: append,
    });

    // One keyed host scope per item, keyed by instance identity: DOM identity follows the item
    // as it moves regardless of what the renderer emits, and each item's affordance callbacks are
    // hook-stable inside their own scope.
    //
    // The pairs come from the array's own projection rather than from a read of each item node:
    // the array's tracked read above already covers every structural change (the core marks the
    // array changed whenever its children or data change), and tracking each item node here
    // would only add redundant subscriptions.
    let item_identities = projection
        .collection_items
        .iter()
        .map(|item| (item.identity, item.item))
        .collect::<Vec<_>>();
    let count = item_identities.len();
    let gates = ItemGates {
        can_insert: operations.can_append_item(),
        can_move: operations.can_move_item(),
        can_remove: operations.can_remove_item(),
    };
    let items = rsx! {
        for (index, (identity, item)) in item_identities.into_iter().enumerate() {
            CollectionItemHost {
                key: "{array_item_input_id(&element_id, identity)}",
                form: props.form.clone(),
                array: props.array.clone(),
                identity,
                item,
                position: index + 1,
                count,
                item_label: item_label.clone(),
                gates,
                append_id: append_id.clone(),
                feedback,
            }
        }
    };

    let (announcement_sequence, announcement_event) = *announcement.read();
    let announcement_text = announcement_event
        .map(|event| {
            localize_builtin(
                &props.form,
                event.message(item_label.clone(), projection.label.clone()),
            )
        })
        .unwrap_or_default();
    let announcement = rsx! {
        div {
            "data-array-status": "",
            "data-announcement-sequence": "{announcement_sequence}",
            role: "status",
            "aria-live": "polite",
            "aria-atomic": "true",
            "{announcement_text}"
        }
    };

    props
        .form
        .inner
        .structure
        .render_collection(render::CollectionContext {
            presentation,
            item_label,
            count,
            items,
            append,
            announcement,
            extensions: props.array.extensions.clone(),
        })
}

/// The array's current item identities, or empty when the node is not readable.
fn array_children(reader: &handle::NodeReader) -> Vec<schemaform::InstanceIdentity> {
    reader
        .read()
        .ok()
        .flatten()
        .map(|view| view.children)
        .unwrap_or_default()
}

/// Requests focus on the one item present in `after` but not in `before`, if any.
///
/// Append and insert both create exactly one item; the core assigns its identity, so the new item
/// is found by difference rather than by position.
fn focus_new_item(
    feedback: ArrayFeedback,
    array_element_id: &str,
    before: &[schemaform::InstanceIdentity],
    after: &[schemaform::InstanceIdentity],
) {
    if let Some(identity) = after.iter().find(|identity| !before.contains(identity)) {
        feedback.focus(ArrayFocusRequest::Item(array_item_input_id(
            array_element_id,
            *identity,
        )));
    }
}

/// The id of the adapter-owned wrapper around the item whose root carries `stem`.
fn array_item_row_id(stem: &str) -> String {
    format!("{stem}-row")
}

/// Which item operations the collection currently allows, as the core reports them.
///
/// The core gates insertion and appending together (`can_append_item`), so `can_insert` is that
/// gate; first/last position gating for moves is applied per item by the host.
#[derive(Clone, Copy, PartialEq)]
struct ItemGates {
    can_insert: bool,
    can_move: bool,
    can_remove: bool,
}

#[derive(Props, Clone, PartialEq)]
struct CollectionItemHostProps {
    form: render::BoundForm,
    array: render::BoundArray,
    identity: schemaform::InstanceIdentity,
    item: schemaform::ItemIdentity,
    position: usize,
    count: usize,
    item_label: String,
    gates: ItemGates,
    append_id: String,
    feedback: ArrayFeedback,
}

/// The adapter-owned host for one array item.
///
/// It owns the keyed scope, the row wrapper carrying the adapter's row id, and the four item
/// affordances. The affordances are `use_callback`s: `Callback::new` registers with the scope
/// owner and is freed only when the scope drops, so building them per render in the array
/// component would accumulate N×4 callbacks until unmount; one scope per item with a fixed hook
/// count keeps them stable and bounded. The renderer's `collection_item` output is placed inside
/// the wrapper.
#[allow(non_snake_case)]
fn CollectionItemHost(props: CollectionItemHostProps) -> Element {
    let element_id = props.array.element_id.clone();
    let reader = props
        .form
        .handle()
        .node(props.array.identity)
        .ok()
        .flatten();
    let collection = reader.as_ref().map(handle::NodeReader::collection_actions);
    let operation_errors = dioxus_core::try_consume_context::<OperationErrorHandler>();
    let feedback = props.feedback;
    let item = props.item;
    let position = props.position;
    let stem = array_item_input_id(&element_id, props.identity);

    let insert_before = {
        let collection = collection.clone();
        let reader = reader.clone();
        let error_route = operation_errors.clone();
        let element_id = element_id.clone();
        use_callback(move |()| {
            let (Some(collection), Some(reader)) = (&collection, &reader) else {
                return;
            };
            let before = array_children(reader);
            if report_operation(&error_route, collection.insert_before(item)) {
                focus_new_item(feedback, &element_id, &before, &array_children(reader));
                feedback.announce(ArrayAnnouncement::Inserted { position });
            }
        })
    };
    let move_up = {
        let collection = collection.clone();
        let error_route = operation_errors.clone();
        let stem = stem.clone();
        use_callback(move |()| {
            let Some(collection) = &collection else {
                return;
            };
            if report_operation(&error_route, collection.move_up(item)) {
                feedback.focus(ArrayFocusRequest::Element(vec![
                    format!("{stem}-move-up"),
                    format!("{stem}-move-down"),
                    array_item_row_id(&stem),
                ]));
                feedback.announce(ArrayAnnouncement::MovedUp {
                    position: position - 1,
                });
            }
        })
    };
    let move_down = {
        let collection = collection.clone();
        let error_route = operation_errors.clone();
        let stem = stem.clone();
        use_callback(move |()| {
            let Some(collection) = &collection else {
                return;
            };
            if report_operation(&error_route, collection.move_down(item)) {
                feedback.focus(ArrayFocusRequest::Element(vec![
                    format!("{stem}-move-down"),
                    format!("{stem}-move-up"),
                    array_item_row_id(&stem),
                ]));
                feedback.announce(ArrayAnnouncement::MovedDown {
                    position: position + 1,
                });
            }
        })
    };
    let remove = {
        let collection = collection.clone();
        let reader = reader.clone();
        let handle = props.form.handle().clone();
        let error_route = operation_errors.clone();
        let element_id = element_id.clone();
        let append_id = props.append_id.clone();
        use_callback(move |()| {
            let (Some(collection), Some(reader)) = (&collection, &reader) else {
                return;
            };
            let children = array_children(reader);
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
                        .or_else(|| index.checked_sub(1).and_then(|index| children.get(index)))
                })
                .map(|identity| {
                    ArrayFocusRequest::Item(array_item_input_id(&element_id, *identity))
                })
                .unwrap_or_else(|| ArrayFocusRequest::Element(vec![append_id.clone()]));
            if report_operation(&error_route, collection.remove(item)) {
                feedback.focus(next_focus);
                feedback.announce(ArrayAnnouncement::Removed {
                    position: target_index.map_or(0, |index| index + 1),
                });
            }
        })
    };

    let Some(node) = instantiate_array_template(
        &props.form,
        &props.array.template,
        props.identity,
        &element_id,
    ) else {
        return rsx! {};
    };

    let item_label = props.item_label.clone();
    let affordance = |kind: render::AffordanceKind,
                      suffix: &str,
                      label: BuiltinMessage,
                      accessible_name: BuiltinMessage,
                      invoke: Callback<()>| render::Affordance {
        kind,
        label: localize_builtin(&props.form, label),
        id: format!("{stem}-{suffix}"),
        accessible_name: Some(localize_builtin(&props.form, accessible_name)),
        invoke,
    };
    let context = render::CollectionItemContext {
        row_id: array_item_row_id(&stem),
        position,
        count: props.count,
        item_label: item_label.clone(),
        children: rsx! {
            BoundNode {
                form: props.form.clone(),
                node,
            }
        },
        insert_before: props.gates.can_insert.then(|| {
            affordance(
                render::AffordanceKind::InsertBefore,
                "insert-before",
                BuiltinMessage::ArrayInsertBefore {
                    item_label: item_label.clone(),
                },
                BuiltinMessage::ArrayInsertBeforeAt {
                    item_label: item_label.clone(),
                    position,
                },
                insert_before,
            )
        }),
        move_up: (props.gates.can_move && position > 1).then(|| {
            affordance(
                render::AffordanceKind::MoveUp,
                "move-up",
                BuiltinMessage::ArrayMoveUp {
                    item_label: item_label.clone(),
                },
                BuiltinMessage::ArrayMoveUpAt {
                    item_label: item_label.clone(),
                    position,
                },
                move_up,
            )
        }),
        move_down: (props.gates.can_move && position < props.count).then(|| {
            affordance(
                render::AffordanceKind::MoveDown,
                "move-down",
                BuiltinMessage::ArrayMoveDown {
                    item_label: item_label.clone(),
                },
                BuiltinMessage::ArrayMoveDownAt {
                    item_label: item_label.clone(),
                    position,
                },
                move_down,
            )
        }),
        remove: props.gates.can_remove.then(|| {
            affordance(
                render::AffordanceKind::RemoveItem,
                "remove",
                BuiltinMessage::ArrayRemove {
                    item_label: item_label.clone(),
                },
                BuiltinMessage::ArrayRemoveAt {
                    item_label: item_label.clone(),
                    position,
                },
                remove,
            )
        }),
    };

    rsx! {
        div {
            id: array_item_row_id(&stem),
            class: "schemaform-array-item",
            "data-array-item": "",
            {props.form.inner.structure.render_collection_item(context)}
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
        ArrayFocusRequest::Item(stem) => {
            let _ = stem;
        }
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;

        const FOCUSABLE: &str = "input:not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])";

        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let as_html = |element: web_sys::Element| element.dyn_into::<web_sys::HtmlElement>().ok();
        let first_focusable_inside = |id: &str| {
            document
                .get_element_by_id(id)
                .and_then(|element| element.query_selector(FOCUSABLE).ok().flatten())
                .and_then(as_html)
        };
        let target = match target {
            ArrayFocusRequest::Element(targets) => targets
                .iter()
                .find_map(|target| document.get_element_by_id(target).and_then(as_html)),
            ArrayFocusRequest::Item(stem) => document
                .get_element_by_id(stem)
                .filter(|root| root.matches(FOCUSABLE).unwrap_or(false))
                .and_then(as_html)
                .or_else(|| first_focusable_inside(stem))
                .or_else(|| first_focusable_inside(&array_item_row_id(stem))),
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

/// Renders the built-in presence buttons for a fixed-object group.
///
/// Only [`FixedObjectGroup`] uses this: homogeneous arrays hand their container presence out as
/// affordances on [`render::NodePresentation::presence`]. When a `FixedObjectRenderer` seam lands,
/// fixed objects move to affordances the same way and this helper is deleted.
fn container_presence_actions(
    form: &render::BoundForm,
    actions: handle::ControlActions,
    projection: &handle::NodeProjection,
) -> Element {
    let materialize_actions = actions.clone();
    let replace_actions = actions.clone();
    let remove_actions = actions;
    let replacement = projection.creation_seed.clone();
    let operation_errors = dioxus_core::try_consume_context::<OperationErrorHandler>();
    let materialize_errors = operation_errors.clone();
    let replace_errors = operation_errors.clone();
    let remove_errors = operation_errors;
    let incompatible_value = container_incompatible_value(projection);
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
                        report_operation(&materialize_errors, materialize_actions.materialize());
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
                        report_operation(
                            &replace_errors,
                            replace_actions.replace_value(replacement.clone()),
                        );
                    },
                    "{replace_label}"
                }
            }
            if projection.allowed_operations.can_remove_value() {
                button {
                    r#type: "button",
                    "data-remove-value": "",
                    onclick: move |_| {
                        report_operation(&remove_errors, remove_actions.remove_value());
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
    let presence_actions = container_presence_actions(&props.form, reader.actions(), &projection);
    let presentation = node_presentation(
        &props.form,
        &projection,
        &props.group.element_id,
        Vec::new(),
        container_incompatible_value(&projection),
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
        None,
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
            accessible_name: None,
            invoke: callbacks.set,
        });
    }
    if operations.can_set_null() {
        presence.push(Affordance {
            kind: AffordanceKind::SetNull,
            label: localize_builtin(form, BuiltinMessage::PresenceSetNull { label: label() }),
            id: format!("{element_id}-set-null"),
            accessible_name: None,
            invoke: callbacks.set_null,
        });
    }
    if operations.can_remove_value() {
        presence.push(Affordance {
            kind: AffordanceKind::RemoveValue,
            label: localize_builtin(form, BuiltinMessage::PresenceRemove { label: label() }),
            id: format!("{element_id}-remove-value"),
            accessible_name: None,
            invoke: callbacks.remove_value,
        });
    }
    if operations.can_replace_value() && has_seed {
        presence.push(Affordance {
            kind: AffordanceKind::Replace,
            label: localize_builtin(form, BuiltinMessage::PresenceReplace { label: label() }),
            id: format!("{element_id}-replace-value"),
            accessible_name: None,
            invoke: callbacks.replace,
        });
    }
    presence
}

/// The hook-stable callbacks behind a container's presence affordances.
struct ContainerPresenceCallbacks {
    materialize: Callback<()>,
    replace: Callback<()>,
    remove_value: Callback<()>,
}

/// Computes the presence affordances the core allows for one container right now.
///
/// This is the container counterpart of [`scalar_presence_affordances`]: materialize and remove
/// value exactly when the core allows them; replace only while the core allows replacement and a
/// creation seed exists. The homogeneous array hands the list out through its presentation;
/// [`container_presence_actions`] still renders the same rules inline for fixed-object groups.
fn container_presence_affordances(
    form: &render::BoundForm,
    projection: &handle::NodeProjection,
    element_id: &str,
    callbacks: ContainerPresenceCallbacks,
) -> Vec<render::Affordance> {
    use render::{Affordance, AffordanceKind};

    let operations = projection.allowed_operations;
    let has_seed = projection.creation_seed.is_some();
    let label = || projection.label.clone();
    let mut presence = Vec::new();
    if operations.can_materialize() {
        presence.push(Affordance {
            kind: AffordanceKind::Materialize,
            label: localize_builtin(form, BuiltinMessage::PresenceAdd { label: label() }),
            id: format!("{element_id}-materialize"),
            accessible_name: None,
            invoke: callbacks.materialize,
        });
    }
    if operations.can_replace_value() && has_seed {
        presence.push(Affordance {
            kind: AffordanceKind::Replace,
            label: localize_builtin(form, BuiltinMessage::PresenceReplace { label: label() }),
            id: format!("{element_id}-replace-value"),
            accessible_name: None,
            invoke: callbacks.replace,
        });
    }
    if operations.can_remove_value() {
        presence.push(Affordance {
            kind: AffordanceKind::RemoveValue,
            label: localize_builtin(form, BuiltinMessage::PresenceRemove { label: label() }),
            id: format!("{element_id}-remove-value"),
            accessible_name: None,
            invoke: callbacks.remove_value,
        });
    }
    presence
}

/// Renders the built-in presence buttons for one scalar control from its affordances.
fn scalar_presence_actions(presence: &[render::Affordance]) -> Element {
    let buttons = presence.to_vec();
    rsx! {
        div { class: "schemaform-presence-actions",
            for affordance in buttons {
                {affordance.present()}
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
    let presentation = node_presentation(
        &props.form,
        &projection,
        &props.control.input_id,
        presence,
        incompatible_value(&projection),
    );
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

/// The value shown beside a container's replace affordance while its data is replaceable.
///
/// Containers have no value state; the core allows replacement exactly when the current data is
/// not the container shape the data schema expects, so replaceability alone selects the value.
fn container_incompatible_value(projection: &handle::NodeProjection) -> Option<String> {
    (projection.allowed_operations.can_replace_value() && !projection.write_only)
        .then(|| projection.current_data.as_ref().map(Value::to_string))
        .flatten()
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
    let incompatible_value = context.presentation().incompatible_value.clone();
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
        return chrome.read_only_output(projection.display_text());
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
    let incompatible_value = context.presentation().incompatible_value.clone();
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
        return chrome.read_only_output(projection.display_text());
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
        None => projection.display_text(),
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
    let display_value = projection.display_text();
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
    ChoiceIdentity, ChoiceOptionProjection, CollectionActions, CollectionItemProjection,
    ControlActions, FindingProjection, FormHandle, FormProjection, FormReader, HandleError,
    HandleTransactionError, NodeProjection, NodeReader, use_form,
};
pub use render::{
    Affordance, AffordanceKind, BUILTIN_CONTROL_PRIORITY, BindError, BindFinding, BooleanLabels,
    BoundForm, BuiltinCollection, BuiltinControlRenderer, BuiltinShell, CollectionContext,
    CollectionItemContext, CollectionRenderer, ControlFacets, ControlKind, ControlMatcher,
    ControlRegistry, ControlRenderContext, ControlRenderer, ExtensionHandler, ExtensionOccurrence,
    ExtensionPrepareError, ExtensionRenderContext, FindingCollectionContext,
    FindingCollectionPresenter, FindingDescriptor, FindingKind, FindingPresentation, Help,
    Localizer, MessageDescriptor, NodePresentation, PreparedExtension, PreparedExtensions,
    RenderConfiguration, RenderConfigurationBuilder, ShellContext, ShellRenderer,
    StructureRenderers, TargetFocusAction, WriteOnlyReplacement,
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
