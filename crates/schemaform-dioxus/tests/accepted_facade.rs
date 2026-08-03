use std::sync::Arc;

use dioxus::prelude::Element;
use schemaform::{ExtensionNamespace, WidgetSymbol, definition::DefinitionNodeView};
use schemaform_dioxus::{
    ControlMatcher, ControlRegistry, ControlRenderContext, ControlRenderer, ExtensionHandler,
    ExtensionOccurrence, ExtensionPrepareError, ExtensionRenderContext, FindingCollectionPresenter,
    Localizer, PreparedExtension, RenderConfiguration,
    render::{BUILTIN_CONTROL_PRIORITY, FindingCollectionContext, FindingKind, MessageDescriptor},
};

struct Renderer;

impl ControlRenderer for Renderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let _restricted_capabilities = (
            context.node(),
            context.actions(),
            context.accessibility(),
            context.label(),
            context.is_label_visible(),
            context.help(),
            context.extensions(),
        );
        dioxus::prelude::rsx! {}
    }
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
        .matcher(BUILTIN_CONTROL_PRIORITY + 10, Arc::new(Matcher), renderer);
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
