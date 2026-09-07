use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::components::{ExampleSection, InlineCode, PageHeader, snippet_theme};
use crate::examples::advisory::AdvisorySubmissionExample;

#[component]
pub fn Advisory() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Submission",
            title: "Advisory submission",
            intro: "By default the form is the last word on validity: a submission with findings is blocked, the findings are shown, and focus moves to the summary. A host that decides validity itself — a draft that may be saved incomplete, a client whose server validates again, a probe that must send what the schema forbids — opts the whole form into advisory mode instead. Nothing is refused, and the host receives the data together with the findings it carried.",
        }
        ExampleSection {
            title: "A draft saved as it stands",
            intro: rsx! {
                "The title is already too short. Press Save draft: the summary and the control show the finding, focus stays where you left it, and the host is handed the data beside the finding through "
                InlineCode { "on_advisory_submit" }
                ", not "
                InlineCode { "on_submit" }
                ". The two callbacks carry different types, so a host cannot mistake an "
                InlineCode { "AdvisorySubmission" }
                " for a validated "
                InlineCode { "SubmissionSnapshot" }
                ". Type letters into Affected users to see the other rule: an unparseable number stays out of the data and arrives as a parse finding. The submit affordance the shell receives carries the mode as its "
                InlineCode { "AffordanceKind" }
                "; this page's shell reads it and labels the button for what it does, without reconstructing the rule."
            },
            demo: rsx! { AdvisorySubmissionExample {} },
            code: rsx! {
                Code { src: code!("src/examples/advisory.rs"), theme: snippet_theme() }
            },
        }
    }
}
