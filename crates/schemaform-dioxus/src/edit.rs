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
    handle::{ChoiceIdentity, ControlActions, FormHandle, HandleError, NodeProjection, NodeReader},
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
