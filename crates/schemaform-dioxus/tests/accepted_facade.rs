use std::sync::Arc;

use dioxus::prelude::Element;
use schemaform::{ExtensionNamespace, WidgetSymbol, definition::DefinitionNodeView};
use schemaform_dioxus::{
    Affordance, AffordanceKind, BooleanEdit, BuiltinControlRenderer, ChoiceEdit, ChoiceOption,
    ControlFacets, ControlKind, ControlMatcher, ControlRegistry, ControlRenderContext,
    ControlRenderer, ExtensionHandler, ExtensionOccurrence, ExtensionPrepareError,
    ExtensionRenderContext, FindingCollectionPresenter, Localizer, NodePresentation,
    PreparedExtension, RenderConfiguration, TextEdit,
    render::{BUILTIN_CONTROL_PRIORITY, FindingCollectionContext, FindingKind, MessageDescriptor},
};

struct Renderer;

impl ControlRenderer for Renderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let presentation: &NodePresentation = context.presentation();
        let _presentation_fields = (
            presentation.element_id.as_str(),
            presentation.label.as_str(),
            presentation.label_visible,
            presentation
                .help
                .as_ref()
                .map(|help| (help.id.as_str(), help.text.as_str())),
            presentation.findings.iter().map(|finding| {
                (
                    finding.stable_id.as_str(),
                    finding.kind,
                    finding.blocking,
                    finding.text.as_str(),
                )
            }),
            presentation.invalid,
            presentation.described_by(),
        );
        let _affordances = presentation.presence.iter().map(|affordance: &Affordance| {
            (
                matches!(
                    affordance.kind,
                    AffordanceKind::Set
                        | AffordanceKind::SetNull
                        | AffordanceKind::RemoveValue
                        | AffordanceKind::Replace
                ),
                affordance.label.as_str(),
                affordance.id.as_str(),
                affordance.invoke,
                affordance.clone() == *affordance,
            )
        });
        let _reported: Option<()> = context.report(Ok(()));
        let control: &ControlFacets = context.control();
        let _control_fields = (
            control.kind == ControlKind::String,
            control.name.as_str(),
            control.required,
            control.disabled,
            control.read_only,
            control.write_only,
            control.touched,
            control.dirty,
            control.nullable,
            control
                .write_only_replacement
                .as_ref()
                .map(|replacement| (replacement.label.as_str(), replacement.placeholder.as_str())),
            control.write_only_status.as_deref(),
            control
                .boolean_labels
                .as_ref()
                .map(|labels| (labels.false_label.as_str(), labels.true_label.as_str())),
        );
        let _restricted_capabilities = (
            context.node(),
            context.actions(),
            context.extensions(),
            context.clone() == context,
        );
        dioxus::prelude::rsx! {
            {presentation.present_help()}
            {presentation.present_findings()}
        }
    }
}

/// The headless edit handles a renderer's child component obtains from the hooks. Hooks need a
/// component scope, so this only pins the accepted field set.
#[allow(dead_code)]
fn headless_edit_handles(text: &TextEdit, boolean: &BooleanEdit, choice: &ChoiceEdit) {
    let _text = (
        text.value,
        text.input,
        text.composition_start,
        text.composition_end,
        text.blur,
        text.read_only,
    );
    let _boolean = (boolean.checked, boolean.set, boolean.blur);
    let _choice = (choice.selected, choice.select, choice.blur);
    let _options = choice.options.iter().map(|option: &ChoiceOption| {
        (
            option.identity.as_str(),
            option.label.as_str(),
            option.is_null,
            option.disabled,
        )
    });
}

struct Matcher;

impl ControlMatcher for Matcher {
    fn matches(&self, _definition: DefinitionNodeView<'_>) -> bool {
        true
    }
}

struct Presenter;

impl FindingCollectionPresenter for Presenter {
    fn render(&self, context: FindingCollectionContext) -> Element {
        let _collection_kind = context.is_summary();
        for entry in context.entries() {
            let finding = entry.finding();
            let _descriptor = (
                finding.stable_id.as_str(),
                finding.kind == FindingKind::Validation,
                finding.code.as_str(),
                finding.text.as_str(),
                finding.blocking,
                &finding.parameters,
                entry.target_focus(),
            );
        }
        dioxus::prelude::rsx! {}
    }
}

struct TestLocalizer;

impl Localizer for TestLocalizer {
    fn localize(&self, message: &MessageDescriptor) -> String {
        message.fallback.clone()
    }
}

struct Extension;

impl ExtensionHandler for Extension {
    fn prepare(
        &self,
        occurrence: ExtensionOccurrence<'_>,
    ) -> Result<Arc<dyn PreparedExtension>, ExtensionPrepareError> {
        let _definition_only_input = (
            occurrence.namespace,
            occurrence.definition_node,
            occurrence.value,
        );
        Ok(Arc::new(Self))
    }
}

impl PreparedExtension for Extension {
    fn decorate(&self, context: ExtensionRenderContext, child: Element) -> Element {
        let _read_only_context = (
            context.definition_node(),
            context.instance(),
            context.namespace(),
        );
        child
    }
}

#[test]
fn accepted_adapter_customization_types_build_an_immutable_configuration() {
    let renderer = Arc::new(Renderer);
    let controls = ControlRegistry::with_builtins()
        .widget(
            WidgetSymbol::parse("company:text").expect("the widget symbol should be valid"),
            renderer.clone(),
        )
        .widget(
            WidgetSymbol::parse("company:plain").expect("the widget symbol should be valid"),
            Arc::new(BuiltinControlRenderer),
        )
        .matcher(
            BUILTIN_CONTROL_PRIORITY + 10,
            Arc::new(Matcher),
            renderer.clone(),
        );
    let _without_builtins =
        ControlRegistry::empty().matcher(BUILTIN_CONTROL_PRIORITY, Arc::new(Matcher), renderer);
    let _configuration = RenderConfiguration::builder()
        .controls(controls)
        .local_presenter(Arc::new(Presenter))
        .summary_presenter(Arc::new(Presenter))
        .localizer(Arc::new(TestLocalizer))
        .extension(
            ExtensionNamespace::parse("https://example.com/accepted").unwrap(),
            Arc::new(Extension),
        )
        .build();
}
