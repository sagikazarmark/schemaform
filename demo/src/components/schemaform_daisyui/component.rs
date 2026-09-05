//! The daisyUI control renderer and the registry that selects it, and the component that binds a
//! form through every seam this package fills.

use std::{rc::Rc, sync::Arc};

use dioxus::prelude::*;
use schemaform::{
    SubmissionSnapshot, WidgetSymbol,
    definition::{DefinitionNodeView, SemanticKind},
};
use schemaform_dioxus::{
    BUILTIN_CONTROL_PRIORITY, BuiltinControlRenderer, ControlKind, ControlMatcher, ControlRegistry,
    ControlRenderContext, ControlRenderer, FindingCollectionPresenter, FormHandle, HandleError,
    RenderConfiguration, SchemaForm, StructureRenderers,
};

use super::Appearance;
use super::boolean::BooleanControl;
use super::choice::{NativeSelectControl, RadioGroupControl, SelectControl};
use super::collection::DaisyuiCollection;
use super::constant::ConstantControl;
use super::findings::DaisyuiFindings;
use super::shell::DaisyuiShell;
use super::text::TextControl;

/// A form rendered through every seam this package fills: the daisyUI control registry, the
/// structure bundle, and the finding presenter in both presenter slots.
///
/// The form is bound once, when the component mounts, as the adapter requires of structure
/// renderers; changing the form handle means remounting. A host that wants one seam to differ —
/// its own shell, say, or the built-in controls under the daisyUI collection — composes a
/// [`RenderConfiguration`] from [`controls`], [`structure`], and [`findings`] instead and renders
/// `SchemaForm` itself; [`configuration`] is the composition this component uses.
///
/// A bind failure is a render error: it reports a definition the renderers cannot present, which
/// is a programming error of the host rather than a state of the form.
#[component]
pub fn SchemaformDaisyui(
    /// The form to present.
    form: FormHandle,
    /// Receives the submission snapshot of a submission that passed preparation.
    on_submit: EventHandler<SubmissionSnapshot>,
    /// Receives adapter operation failures; failures are dropped when it is not set.
    #[props(default)]
    on_error: EventHandler<HandleError>,
    /// Whether the renderers emit their layout utilities. Fixed at mount, like the renderers.
    #[props(default)]
    appearance: Appearance,
) -> Element {
    let bound = use_hook(|| configuration_with(appearance).bind(&form).map_err(Rc::new));
    let bound = match bound {
        Ok(bound) => bound,
        Err(error) => return Err(dioxus::core::CapturedError::from_display(error).into()),
    };
    rsx! {
        SchemaForm { form: bound, on_submit, on_error }
    }
}

/// The render configuration [`SchemaformDaisyui`] binds a form with: [`controls`] as the control
/// registry, [`structure`] as the structure bundle, and [`findings`] in both presenter slots.
pub fn configuration() -> RenderConfiguration {
    configuration_with(Appearance::default())
}

/// [`configuration`] with every renderer at `appearance`.
pub fn configuration_with(appearance: Appearance) -> RenderConfiguration {
    RenderConfiguration::builder()
        .controls(controls_with(appearance))
        .structure(structure_with(appearance))
        .summary_presenter(findings_with(appearance))
        .local_presenter(findings_with(appearance))
        .build()
}

/// Matcher priority at which [`controls`] registers [`DaisyuiControlRenderer`].
///
/// Above [`BUILTIN_CONTROL_PRIORITY`] so the daisyUI renderer wins every control it accepts
/// and the built-in keeps the rest.
pub const DAISYUI_CONTROL_PRIORITY: i32 = BUILTIN_CONTROL_PRIORITY + 10;

/// The widget symbol a UI schema control names to render a choice as a radio group.
pub const RADIO_WIDGET: &str = "daisyui:radio";

/// The widget symbol a UI schema control names to render a choice as the registry's compound
/// select rather than the native one.
pub const SELECT_WIDGET: &str = "daisyui:select";

/// A control registry in which every control kind renders as a daisyUI field.
///
/// The registry starts from the built-ins, so structural nodes keep their built-in
/// presentation and a control the daisyUI renderer does not accept still renders. Choices render
/// as a native select unless the UI schema names [`RADIO_WIDGET`] or [`SELECT_WIDGET`] for them.
pub fn controls() -> ControlRegistry {
    controls_with(Appearance::default())
}

/// [`controls`] with every renderer at `appearance`.
pub fn controls_with(appearance: Appearance) -> ControlRegistry {
    let renderer =
        |choice| DaisyuiControlRenderer::with_choice_widget(choice).appearance(appearance);
    ControlRegistry::with_builtins()
        .matcher(
            DAISYUI_CONTROL_PRIORITY,
            Arc::new(DaisyuiControls),
            Arc::new(renderer(ChoiceWidget::NativeSelect)),
        )
        .widget(
            widget_symbol(RADIO_WIDGET),
            Arc::new(renderer(ChoiceWidget::RadioGroup)),
        )
        .widget(
            widget_symbol(SELECT_WIDGET),
            Arc::new(renderer(ChoiceWidget::Select)),
        )
}

/// The registry key for one of this component's widget symbols.
fn widget_symbol(symbol: &str) -> WidgetSymbol {
    WidgetSymbol::parse(symbol).expect("the daisyUI widget symbols are non-empty")
}

/// The structure renderers this component ships: the daisyUI form shell and homogeneous-array
/// collection.
///
/// Every slot the component does not implement stays the built-in, so a form bound with this
/// bundle degrades to the adapter's accessible unstyled output for those node kinds rather than
/// losing a region.
pub fn structure() -> StructureRenderers {
    structure_with(Appearance::default())
}

/// [`structure`] with both renderers at `appearance`.
pub fn structure_with(appearance: Appearance) -> StructureRenderers {
    StructureRenderers::default()
        .with_shell(DaisyuiShell::default().appearance(appearance))
        .with_collection(DaisyuiCollection::default().appearance(appearance))
}

/// The finding presenter for both presenter slots of a render configuration.
///
/// Wire it as the summary presenter for the alert and as the local presenter so the findings a
/// built-in container or this component's collection renders through
/// `NodePresentation::present_findings` are daisyUI-styled as well.
pub fn findings() -> Arc<dyn FindingCollectionPresenter> {
    findings_with(Appearance::default())
}

/// [`findings`] at `appearance`.
pub fn findings_with(appearance: Appearance) -> Arc<dyn FindingCollectionPresenter> {
    Arc::new(DaisyuiFindings::default().appearance(appearance))
}

/// Accepts exactly the definition nodes [`DaisyuiControlRenderer`] presents itself: those the
/// adapter derives a control kind from.
struct DaisyuiControls;

impl ControlMatcher for DaisyuiControls {
    fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
        matches!(
            definition.semantic_kind(),
            Some(
                SemanticKind::String
                    | SemanticKind::Number
                    | SemanticKind::Integer
                    | SemanticKind::Boolean
                    | SemanticKind::Choice
                    | SemanticKind::Null
            )
        )
    }
}

/// The widget a [`DaisyuiControlRenderer`] presents a selectable choice with.
///
/// An exact widget request never reaches the renderer at render time, so the registry carries
/// one renderer per symbol and the symbol's meaning travels here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChoiceWidget {
    /// The registry's `NativeSelect`: a native `select` at native-control weight.
    #[default]
    NativeSelect,
    /// The registry's `RadioGroup`: one `RadioItem` per option.
    RadioGroup,
    /// The registry's compound `Select`: a trigger and a dropdown listbox.
    Select,
}

/// Renders every control kind with the registry's `Field` parts and widgets.
///
/// The renderer owns the whole control region: label, widget, help, findings, and presence
/// affordances. Should a host register it for a control kind this component does not know, that
/// control is handed to [`BuiltinControlRenderer`] rather than to an editable widget the mapping
/// does not cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiControlRenderer {
    choice: ChoiceWidget,
    appearance: Appearance,
}

impl DaisyuiControlRenderer {
    /// A renderer presenting choices with `choice`; [`Default`] presents them natively.
    pub fn with_choice_widget(choice: ChoiceWidget) -> Self {
        Self {
            choice,
            appearance: Appearance::default(),
        }
    }

    /// The same renderer at `appearance`.
    pub fn appearance(self, appearance: Appearance) -> Self {
        Self { appearance, ..self }
    }
}

impl ControlRenderer for DaisyuiControlRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let appearance = self.appearance;
        // The kind is definition-stable, so a node always renders the same child component and
        // the hooks inside it are called unconditionally.
        match context.control().kind {
            ControlKind::String | ControlKind::Number | ControlKind::Integer => {
                rsx! { TextControl { context, appearance } }
            }
            ControlKind::Boolean => rsx! { BooleanControl { context, appearance } },
            ControlKind::Choice => match self.choice {
                ChoiceWidget::NativeSelect => rsx! { NativeSelectControl { context, appearance } },
                ChoiceWidget::RadioGroup => rsx! { RadioGroupControl { context, appearance } },
                ChoiceWidget::Select => rsx! { SelectControl { context, appearance } },
            },
            ControlKind::Constant => rsx! { ConstantControl { context, appearance } },
            _ => BuiltinControlRenderer.render(context),
        }
    }
}
