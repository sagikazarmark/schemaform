use dioxus::prelude::*;
use schemaform::{AdvisorySubmission, FormDefinition, InstanceIdentity, form::SubmissionBlocker};
use schemaform_dioxus::{
    AffordanceKind, FormHandle, RenderConfiguration, SchemaForm, ShellContext, ShellRenderer,
    StructureRenderers, SubmissionMode, use_form,
};
use serde_json::json;

use crate::components::{
    DemoPane, DemoSurface, SourcePanel, StatusChip,
    button::{Button, ButtonColor},
    schemaform_daisyui,
};

/// A draft the host saves as it stands. The form runs in
/// `SubmissionMode::Advisory`: every submit hands the host the current form
/// data together with every finding the gated path would have blocked on, and
/// nothing is refused. The findings still appear in the summary and beside the
/// controls so the reader sees what is going out unchecked, but focus stays
/// where the reader left it. An unparseable number stays out of the data and
/// arrives as a parse finding instead.
#[component]
pub fn AdvisorySubmissionExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(
        definition,
        json!({ "title": "Login", "severity": "medium", "affected_users": 0 }),
    )
    .expect("the advisory example form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(StructureRenderers::default().with_shell(DraftShell))
            .summary_presenter(schemaform_daisyui::findings())
            .build()
            .bind(&bound_form)
            .expect("built-in controls should bind")
    });
    let mut handed_off: Signal<Option<HandedOff>> = use_signal(|| None);

    rsx! {
        div { class: "space-y-6",
            SchemaForm {
                form: bound,
                submission_mode: SubmissionMode::Advisory,
                // Never called: an advisory form produces no validated snapshot.
                on_submit: move |_| {},
                on_advisory_submit: move |submission: AdvisorySubmission| {
                    handed_off.set(Some(HandedOff::new(&submission, &form)));
                },
                on_error: move |error| crate::examples::report_form_error(&error),
            }
            if let Some(handed_off) = handed_off() {
                HandedOffPanels { handed_off }
            }
        }
    }
}

/// A form shell that labels the submit button for what it does. The submit
/// affordance carries the form's submission mode as its kind, so the shell
/// does not reconstruct the rule: an `AdvisorySubmit` affordance reads
/// "Save draft"; anything else keeps the localized submit label.
struct DraftShell;

impl ShellRenderer for DraftShell {
    fn shell(&self, context: ShellContext) -> Element {
        let submit = context.submit;
        let label = match submit.kind {
            AffordanceKind::AdvisorySubmit => "Save draft".to_owned(),
            _ => submit.label.clone(),
        };
        rsx! {
            div { class: "grid gap-4", "data-draft-shell": "",
                {context.summary}
                {context.body}
                Button {
                    id: submit.id.clone(),
                    r#type: "submit",
                    color: ButtonColor::Primary,
                    class: "w-fit",
                    "{label}"
                }
            }
        }
    }
}

/// What one advisory submission handed the host, ready to display.
#[derive(Clone, PartialEq)]
struct HandedOff {
    data: String,
    findings: Vec<FindingRow>,
}

/// One finding an advisory submission carried: its classification, the form-data location it
/// is about, and its code.
#[derive(Clone, PartialEq)]
struct FindingRow {
    kind: &'static str,
    location: String,
    code: String,
}

impl HandedOff {
    fn new(submission: &AdvisorySubmission, form: &FormHandle) -> Self {
        Self {
            data: serde_json::to_string_pretty(submission.form_data())
                .expect("form data should serialize"),
            findings: submission
                .findings()
                .map(|finding| FindingRow::describe(finding, form))
                .collect(),
        }
    }
}

impl FindingRow {
    fn describe(finding: &SubmissionBlocker, form: &FormHandle) -> Self {
        match finding {
            SubmissionBlocker::Validation(finding) => Self {
                kind: "validation",
                location: finding.instance_location().to_string(),
                code: finding.code().to_owned(),
            },
            // A parse finding names the control whose edit buffer could not become form data.
            SubmissionBlocker::Parse { target, kind } => Self {
                kind: "parse",
                location: binding_of(form, *target),
                code: format!("{kind:?}"),
            },
            SubmissionBlocker::Capability(finding) => Self {
                kind: "capability",
                location: finding.instance_location().to_string(),
                code: finding.code().to_owned(),
            },
            SubmissionBlocker::External { source, finding } => Self {
                kind: "external",
                location: finding.instance_location().to_string(),
                code: format!("{source}: {}", finding.code()),
            },
            SubmissionBlocker::Indeterminate(reason) => Self {
                kind: "indeterminate",
                location: String::new(),
                code: reason.code().to_owned(),
            },
            SubmissionBlocker::ValidationFindingsTruncated { retained } => Self {
                kind: "validation",
                location: String::new(),
                code: format!("truncated after {retained} findings"),
            },
            other => Self {
                kind: "other",
                location: String::new(),
                code: format!("{other:?}"),
            },
        }
    }
}

/// The root-origin control binding of the node `target` identifies, or nothing once it is gone.
fn binding_of(form: &FormHandle, target: InstanceIdentity) -> String {
    form.node(target)
        .ok()
        .flatten()
        .and_then(|node| node.read().ok().flatten())
        .and_then(|projection| projection.binding)
        .map(|binding| binding.to_string())
        .unwrap_or_default()
}

#[component]
fn HandedOffPanels(handed_off: HandedOff) -> Element {
    let count = handed_off.findings.len();
    rsx! {
        DemoSurface {
            primary: rsx! {
                DemoPane { label: "Data handed to the host",
                    div { "data-advisory-data": "", SourcePanel { source: handed_off.data } }
                }
            },
            secondary: rsx! {
                DemoPane {
                    label: "Findings it carried",
                    accessory: rsx! { StatusChip { label: "{count}" } },
                    if handed_off.findings.is_empty() {
                        p {
                            "data-advisory-findings": "",
                            class: "rounded-xl border border-base-300 bg-base-100 p-3 text-sm text-base-content/70",
                            "None: the gated path would have submitted this data too."
                        }
                    } else {
                        ul {
                            "data-advisory-findings": "",
                            class: "divide-y divide-base-300 rounded-xl border border-base-300 bg-base-100 text-sm",
                            for finding in handed_off.findings {
                                li { class: "flex flex-wrap items-center gap-2 px-3 py-2",
                                    span { class: "badge badge-warning badge-sm", "{finding.kind}" }
                                    code { class: "font-mono text-xs", "{finding.location}" }
                                    span { class: "text-base-content/70", "{finding.code}" }
                                }
                            }
                        }
                    }
                }
            },
        }
    }
}

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "severity", "affected_users"],
        "properties": {
            "title": {
                "type": "string",
                "title": "Title",
                "description": "At least eight characters.",
                "minLength": 8
            },
            "severity": {
                "title": "Severity",
                "enum": ["low", "medium", "high"]
            },
            "affected_users": {
                "type": "integer",
                "title": "Affected users",
                "description": "Type letters here to see a parse finding.",
                "minimum": 0
            },
            "summary": {
                "type": "string",
                "title": "Summary",
                "maxLength": 200
            }
        }
    }))
    .expect("the advisory example schema should compile")
}

#[cfg(test)]
mod tests {
    use std::{any::Any, collections::HashMap, rc::Rc};

    use dioxus::core::{
        AttributeValue, ElementId, Event, NoOpMutations, Template, VirtualDom, WriteMutations,
    };
    use dioxus::html::{PlatformEventData, SerializedFormData, SerializedHtmlEventConverter};

    /// Records which element listens for `submit` under which dynamic `id`, so a test can
    /// dispatch the form's own submit event the way a browser would after a click on the shell's
    /// `type="submit"` button.
    #[derive(Default)]
    struct SubmitListeners {
        ids: HashMap<ElementId, String>,
        submit: Vec<ElementId>,
    }

    impl SubmitListeners {
        fn form(&self) -> ElementId {
            *self
                .submit
                .iter()
                .find(|id| {
                    self.ids
                        .get(id)
                        .is_some_and(|value| value.starts_with("schemaform-"))
                })
                .expect("the adapter's form element should listen for submit")
        }
    }

    impl WriteMutations for SubmitListeners {
        fn append_children(&mut self, _id: ElementId, _m: usize) {}
        fn assign_node_id(&mut self, _path: &'static [u8], _id: ElementId) {}
        fn create_placeholder(&mut self, _id: ElementId) {}
        fn create_text_node(&mut self, _value: &str, _id: ElementId) {}
        fn load_template(&mut self, _template: Template, _index: usize, _id: ElementId) {}
        fn replace_node_with(&mut self, _id: ElementId, _m: usize) {}
        fn replace_placeholder_with_nodes(&mut self, _path: &'static [u8], _m: usize) {}
        fn insert_nodes_after(&mut self, _id: ElementId, _m: usize) {}
        fn insert_nodes_before(&mut self, _id: ElementId, _m: usize) {}
        fn set_attribute(
            &mut self,
            name: &'static str,
            _ns: Option<&'static str>,
            value: &AttributeValue,
            id: ElementId,
        ) {
            if name == "id"
                && let AttributeValue::Text(value) = value
            {
                self.ids.insert(id, value.clone());
            }
        }
        fn set_node_text(&mut self, _value: &str, _id: ElementId) {}
        fn create_event_listener(&mut self, name: &'static str, id: ElementId) {
            if name == "submit" {
                self.submit.push(id);
            }
        }
        fn remove_event_listener(&mut self, _name: &'static str, _id: ElementId) {}
        fn remove_node(&mut self, _id: ElementId) {}
        fn push_root(&mut self, _id: ElementId) {}
    }

    /// Mounts the example as the browser would and returns the markup it settles on.
    fn render() -> String {
        let (mut dom, _) = mount();
        settle(&mut dom);
        html(&dom)
    }

    fn mount() -> (VirtualDom, SubmitListeners) {
        let mut dom = VirtualDom::new(super::AdvisorySubmissionExample);
        let mut listeners = SubmitListeners::default();
        dom.rebuild(&mut listeners);
        (dom, listeners)
    }

    fn settle(dom: &mut VirtualDom) {
        for _ in 0..4 {
            dom.render_immediate(&mut NoOpMutations);
        }
    }

    fn html(dom: &VirtualDom) -> String {
        let html = dioxus_ssr::render(dom);
        assert!(!html.contains("Encountered panic"), "{html}");
        html
    }

    #[test]
    fn example_schema_compiles() {
        super::definition();
    }

    /// Before the first submit there is nothing to show; the form renders through the example's
    /// shell, whose button reads the advisory affordance's kind and labels itself for what it
    /// does.
    #[test]
    fn the_example_renders_the_form_with_a_draft_button_and_no_panels_before_a_submit() {
        let html = render();

        assert!(html.contains("data-draft-shell"), "{html}");
        assert!(html.contains("name=\"/title\""), "{html}");
        assert!(html.contains(">Save draft</button>"), "{html}");
        assert!(!html.contains(">Submit</button>"), "{html}");
        assert!(!html.contains("data-advisory-data"), "{html}");
        assert!(!html.contains("data-advisory-findings"), "{html}");
    }

    /// Submitting the too-short draft hands the host the data as it stands beside the finding it
    /// carried, and the form presents the same finding in its summary.
    #[test]
    fn a_submit_shows_the_data_handed_to_the_host_beside_the_finding_it_carried() {
        let (mut dom, listeners) = mount();
        settle(&mut dom);

        dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
        let data: Rc<dyn Any> = Rc::new(PlatformEventData::new(Box::new(SerializedFormData::new(
            String::new(),
            Vec::new(),
        ))));
        dom.runtime()
            .handle_event("submit", Event::new(data, true), listeners.form());
        settle(&mut dom);
        let html = html(&dom);

        let data = html
            .find("data-advisory-data")
            .expect("the data pane should render after a submit");
        let findings = html
            .find("data-advisory-findings")
            .expect("the findings pane should render after a submit");
        assert!(
            data < findings,
            "the data sits beside the findings:\n{html}"
        );
        assert!(
            html[data..].contains("&#34;title&#34;: &#34;Login&#34;"),
            "{html}"
        );
        assert!(html[findings..].contains(">validation</span>"), "{html}");
        assert!(html[findings..].contains(">/title</code>"), "{html}");
        assert!(html[findings..].contains(">minLength</span>"), "{html}");
        assert!(
            html.contains("data-finding-summary") && html.contains("data-finding=\"minLength\""),
            "the form still presents the finding it sent past:\n{html}"
        );
    }
}
