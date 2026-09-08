//! Headless edit hooks that give a custom control renderer the built-in editing behaviour.
//!
//! Each hook is called inside the renderer's own child component with the
//! [`crate::render::ControlRenderContext`] it received, and returns hook-stable callbacks plus a
//! derived read signal the component wires to its widget. The hooks own the correctness-critical
//! parts of editing so renderers place widgets rather than reimplementing IME composition,
//! lifecycle discard, or DOM resynchronisation after the core rejects input.

use std::{fmt, rc::Rc};

use dioxus::prelude::{
    Callback, Memo, ReadSignal, ReadableExt, Signal, WritableExt, use_callback, use_effect,
    use_hook, use_memo, use_signal,
};
use schemaform::form::AllowedOperations;
use serde_json::Value;

use crate::{
    handle::{
        ChoiceIdentity, ChoiceOptionProjection, ControlActions, FormHandle, HandleError,
        NodeProjection, NodeReader,
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
        crate::dom::resynchronize_control_value(&self.node.element_id, &self.canonical_text());
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
        .map(|projection| projection.display_text())
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
            crate::dom::resynchronize_boolean(
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
    /// The selectable options in the core's compiled order, with localized labels: an `enum`
    /// lists its null option first, a constant choice keeps its authored branch order.
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

/// One option of a choice or multiple-choice control, as a widget should present it.
///
/// Options are compiled from the definition, so their identities, order, and descriptions are
/// fixed for the lifetime of the bound form; `label` follows the configured localizer and
/// `disabled` follows the operations the core allows right now. The struct is non-exhaustive
/// and only ever constructed by [`use_choice_edit`] and [`use_multiple_choice_edit`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ChoiceOption {
    /// Opaque identity to hand back to [`ChoiceEdit::select`] or [`MultipleChoiceEdit::toggle`];
    /// its [`ChoiceIdentity::as_str`] form is a safe DOM value.
    pub identity: ChoiceIdentity,
    /// Localized plain-text label: the core's compiled label (a constant choice's branch
    /// `title`, otherwise the value's spelling) through the configured localizer as a keyless
    /// message. The one exception is an untitled null option, which carries the adapter's
    /// `schemaform.choice.null` message (`None` by default) instead of the core's JSON
    /// spelling; a null option with an authored title keeps that title like every other option.
    pub label: String,
    /// The core's compiled per-option description, unlocalized: a constant choice's branch
    /// `description`. `enum` and `const` options have none. The built-in select does not
    /// render it, since an HTML `option` has no slot for one; a radio group or a combobox can.
    pub description: Option<String>,
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

/// The state both option-bearing hooks track, tracked through one memo over the node.
///
/// `read_state` builds the hook's state from a node read with labels localized through the
/// configured localizer. The memo, rather than the calling component, subscribes to the node and
/// the localizer; `None` means the node cannot be read right now. `selected_of` and `options_of`
/// project the two values a widget reads: `selected` is handed out as a read signal so a widget
/// that receives it as a prop stays wired to the live control, while the calling component
/// subscribes to the options alone, so it re-renders when a label or an option's availability
/// changes, not on every node change.
struct OptionHookState<State: 'static, Selected: PartialEq + 'static> {
    state: Memo<Option<State>>,
    selected: ReadSignal<Selected>,
    options: Vec<ChoiceOption>,
}

/// One node read with a label localizer, as the option-bearing hooks consume it.
type LocalizedRead<State> = fn(
    Result<Option<NodeProjection>, HandleError>,
    &mut dyn FnMut(ChoiceLabel<'_>) -> String,
) -> Option<State>;

fn use_option_hook_state<State, Selected>(
    context: &ControlRenderContext,
    read_state: LocalizedRead<State>,
    selected_of: fn(&State) -> Selected,
    options_of: fn(&State) -> Vec<ChoiceOption>,
) -> OptionHookState<State, Selected>
where
    State: Clone + PartialEq + 'static,
    Selected: Clone + Default + PartialEq + 'static,
{
    let state = {
        let reader = context.node().clone();
        let form = context.presentation().form().clone();
        use_memo(move || {
            read_state(reader.read(), &mut |label| match label {
                ChoiceLabel::Null => {
                    crate::localize_builtin(&form, crate::BuiltinMessage::ChoiceNull)
                }
                ChoiceLabel::Compiled(label) => crate::localize_text(&form, None, label),
            })
        })
    };
    let selected_memo =
        use_memo(move || state.read().as_ref().map(selected_of).unwrap_or_default());
    let selected = use_hook(|| ReadSignal::new(selected_memo));
    let options = use_memo(move || state.read().as_ref().map(options_of).unwrap_or_default())
        .read()
        .clone();
    OptionHookState {
        state,
        selected,
        options,
    }
}

/// Labels for the state a hook decides against in an event handler: a fresh read never shows
/// labels, so they stay unlocalized.
fn unlocalized(label: ChoiceLabel<'_>) -> String {
    match label {
        ChoiceLabel::Null => String::new(),
        ChoiceLabel::Compiled(label) => label.to_owned(),
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
    let OptionHookState {
        state,
        selected,
        options,
    } = use_option_hook_state(
        context,
        choice_state_of,
        |state: &ChoiceState| state.selected.clone(),
        ChoiceState::options,
    );
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
        choice_state_of(self.node.read_untracked(), &mut unlocalized)
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
        crate::dom::resynchronize_control_value(&self.node.element_id, selected);
    }
}

/// What `choice_state_of` asks its caller to localize for one option: an untitled null option
/// is the adapter's own message, since the core spells it as JSON (`null`), which is not a
/// label for a person; every other option, including a null option with an authored title,
/// carries the core's compiled plain-text label.
enum ChoiceLabel<'a> {
    Null,
    Compiled(&'a str),
}

impl<'a> ChoiceLabel<'a> {
    /// Classifies `option` by whether the core's source authored a title for it, which the
    /// core reports directly, so a null option titled literally `null` reads as written.
    fn of(option: &'a ChoiceOptionProjection) -> Self {
        if option.value.is_null() && option.title.is_none() {
            return Self::Null;
        }
        Self::Compiled(&option.label)
    }
}

/// The choice state of a node read, with labels passed through `localize`, or `None` when the
/// node could not be read.
fn choice_state_of(
    read: Result<Option<NodeProjection>, HandleError>,
    localize: &mut dyn FnMut(ChoiceLabel<'_>) -> String,
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
                    label: localize(ChoiceLabel::of(option)),
                    description: option.description.clone(),
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

/// Headless editing behaviour for one multiple-choice control: a `uniqueItems` array of a
/// finite choice, presented as one toggle per option.
///
/// Obtained from [`use_multiple_choice_edit`]. The callbacks keep their identity across
/// renders and `selected` is a read signal, so a widget that receives this value as a prop does
/// not re-render per edit and stays wired to the live control.
///
/// Two values compare equal when they come from the same hook call site with the same options;
/// `selected`'s current members are not compared. The struct is non-exhaustive so later
/// releases can add fields without breaking renderers; it is only ever constructed by the hook.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct MultipleChoiceEdit {
    /// The options the widget should show as checked right now, in option order.
    ///
    /// Empty while the array is absent or holds no option, and always for a write-only
    /// control, whose members are never echoed. A member the data repeats appears once; a
    /// member that is no option appears nowhere here and is shown through
    /// [`NodePresentation::incompatible_value`](crate::render::NodePresentation::incompatible_value)
    /// instead. It is derived through a memo that subscribes to the node, so the first render
    /// after a transition already sees the new membership.
    pub selected: ReadSignal<Vec<ChoiceIdentity>>,
    /// The options in the core's compiled order, with localized labels. Every option is
    /// `disabled` while the core allows no toggle — the control is read-only, or its array
    /// would have to be created inside an absent parent — or the control is write-only. An
    /// absent array below a present object disables nothing: a multiple choice is a leaf
    /// control, and the first toggle creates its array as typing into an absent string creates
    /// the string. `minItems` and `maxItems` never disable an option; they remain findings.
    pub options: Vec<ChoiceOption>,
    /// Toggles one option's membership through [`ControlActions::toggle_choice`]: a member is
    /// removed together with every duplicate of it, a non-member is inserted in option order,
    /// and on an absent array the first toggle creates the array holding that one member.
    /// An identity that is not among `options` runs no core operation. A failure is reported
    /// to `SchemaForm::on_error` and the checkbox carrying [`Self::option_element_id`] has its
    /// `checked` property restored to the node's membership, so a native checkbox stays in step
    /// with the node.
    pub toggle: Callback<ChoiceIdentity>,
    /// Marks the control touched through [`ControlActions::blur`].
    pub blur: Callback<()>,
    element_id: String,
}

impl MultipleChoiceEdit {
    /// The DOM id the widget for `option` must carry: the node's element id followed by the
    /// option identity, `{element_id}-{identity}`. The hook resynchronises that element after
    /// a rejected toggle, and the built-in gives each checkbox this id.
    pub fn option_element_id(&self, option: &ChoiceIdentity) -> String {
        option_element_id(&self.element_id, option)
    }
}

impl fmt::Debug for MultipleChoiceEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The signal is owned by the component that called the hook; a handle that outlives
        // it must still be printable.
        let selected = self
            .selected
            .try_peek()
            .ok()
            .map(|selected| selected.clone());
        formatter
            .debug_struct("MultipleChoiceEdit")
            .field("selected", &selected)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

fn option_element_id(element_id: &str, option: &ChoiceIdentity) -> String {
    format!("{element_id}-{}", option.as_str())
}

/// What a multiple-choice widget needs from one node read.
#[derive(Clone, PartialEq)]
struct MultipleChoiceState {
    selected: Vec<ChoiceIdentity>,
    entries: Vec<ChoiceEntry>,
}

impl MultipleChoiceState {
    fn options(&self) -> Vec<ChoiceOption> {
        self.entries
            .iter()
            .map(|entry| entry.option.clone())
            .collect()
    }
}

/// Owns the built-in multiple-choice editing behaviour for the control behind `context`.
///
/// This is a Dioxus hook: call it unconditionally, in a stable order, inside the renderer's
/// own child component. [`render::ControlRenderer::render`](crate::render::ControlRenderer)
/// itself is not a hook-safe call site.
///
/// The returned [`MultipleChoiceEdit`] reproduces the built-in multiple choice, a fieldset of
/// checkboxes, exactly: `toggle`
/// maps an opaque option identity to the core's toggle, reports a rejected write to
/// `SchemaForm::on_error`, and restores the checkbox to the node's membership. The same handles
/// drive a listbox with `aria-multiselectable` or a set of toggle buttons.
///
/// ```rust,no_run
/// use dioxus::prelude::*;
/// use schemaform_dioxus::{ControlRenderContext, ControlRenderer, use_multiple_choice_edit};
///
/// struct CheckboxGroupRenderer;
///
/// impl ControlRenderer for CheckboxGroupRenderer {
///     fn render(&self, context: ControlRenderContext) -> Element {
///         rsx! { CheckboxGroup { context } }
///     }
/// }
///
/// #[component]
/// fn CheckboxGroup(context: ControlRenderContext) -> Element {
///     let edit = use_multiple_choice_edit(&context);
///     let presentation = context.presentation();
///     let control = context.control();
///     let selected = edit.selected.read().clone();
///     let options = edit.options.clone();
///     rsx! {
///         fieldset {
///             id: presentation.element_id.clone(),
///             legend { "{presentation.label}" }
///             for option in options {
///                 div {
///                     input {
///                         id: edit.option_element_id(&option.identity),
///                         name: control.name.clone(),
///                         r#type: "checkbox",
///                         checked: selected.contains(&option.identity),
///                         disabled: option.disabled,
///                         "aria-invalid": presentation.invalid,
///                         "aria-describedby": presentation.described_by(),
///                         oninput: {
///                             let identity = option.identity.clone();
///                             move |_| edit.toggle.call(identity.clone())
///                         },
///                         onblur: move |_| edit.blur.call(()),
///                     }
///                     label { r#for: edit.option_element_id(&option.identity), "{option.label}" }
///                 }
///             }
///             {presentation.present_help()}
///             {presentation.present_findings()}
///         }
///     }
/// }
/// ```
pub fn use_multiple_choice_edit(context: &ControlRenderContext) -> MultipleChoiceEdit {
    let OptionHookState {
        state,
        selected,
        options,
    } = use_option_hook_state(
        context,
        multiple_choice_state_of,
        |state: &MultipleChoiceState| state.selected.clone(),
        MultipleChoiceState::options,
    );
    let target = Rc::new(MultipleChoiceEditTarget {
        node: EditTarget::new(context),
        state,
    });

    let toggle = {
        let target = target.clone();
        use_callback(move |identity: ChoiceIdentity| target.toggle(identity))
    };
    let blur = use_callback(move |()| target.node.blur());

    MultipleChoiceEdit {
        selected,
        options,
        toggle,
        blur,
        element_id: context.presentation().element_id.clone(),
    }
}

/// The node one [`use_multiple_choice_edit`] call edits, with the state its callbacks share.
struct MultipleChoiceEditTarget {
    node: EditTarget,
    /// State tracked through the node; `None` while the node is unreadable.
    state: Memo<Option<MultipleChoiceState>>,
}

impl MultipleChoiceEditTarget {
    /// The state to decide and resynchronise against: a fresh read, or the last rendered
    /// state when the form cannot be read right now. Decisions never read labels, so a fresh
    /// read leaves them unlocalized.
    fn current_state(&self) -> Option<MultipleChoiceState> {
        multiple_choice_state_of(self.node.read_untracked(), &mut unlocalized)
            .or_else(|| self.state.peek().clone())
    }

    fn toggle(&self, identity: ChoiceIdentity) {
        let Some(state) = self.current_state() else {
            return;
        };
        let Some(entry) = state
            .entries
            .iter()
            .find(|entry| entry.option.identity == identity)
        else {
            return;
        };
        let result = self.node.actions.toggle_choice(entry.value.clone());
        if !self.node.report(result) {
            self.resynchronize(&state, &identity);
        }
    }

    fn resynchronize(&self, state: &MultipleChoiceState, identity: &ChoiceIdentity) {
        let checked = state.selected.contains(identity);
        crate::dom::resynchronize_boolean(
            &option_element_id(&self.node.element_id, identity),
            Some(checked),
        );
    }
}

/// Whether a multiple choice accepts no toggle right now: the core allows none (the control is
/// read-only, or its array would have to be conjured inside an absent parent), or the control
/// is write-only and must not echo its members. An absent array below a present object is not
/// a reason: the first toggle creates it. The one statement behind the built-in's `disabled`
/// facet and every option's `disabled`.
pub(crate) fn multiple_choice_is_disabled(projection: &NodeProjection) -> bool {
    projection.write_only || !projection.allowed_operations.can_toggle_choice()
}

/// The multiple-choice state of a node read, with labels passed through `localize`, or `None`
/// when the node could not be read.
fn multiple_choice_state_of(
    read: Result<Option<NodeProjection>, HandleError>,
    localize: &mut dyn FnMut(ChoiceLabel<'_>) -> String,
) -> Option<MultipleChoiceState> {
    let projection = read.ok().flatten()?;
    let write_only = projection.write_only;
    let disabled = multiple_choice_is_disabled(&projection);
    let selected = if write_only {
        Vec::new()
    } else {
        projection
            .choice_options
            .iter()
            .filter(|option| option.selected)
            .map(|option| option.identity.clone())
            .collect()
    };
    let entries = projection
        .choice_options
        .iter()
        .map(|option| ChoiceEntry {
            option: ChoiceOption {
                identity: option.identity.clone(),
                label: localize(ChoiceLabel::of(option)),
                description: option.description.clone(),
                is_null: option.value.is_null(),
                disabled,
            },
            value: option.value.clone(),
        })
        .collect();
    Some(MultipleChoiceState { selected, entries })
}
