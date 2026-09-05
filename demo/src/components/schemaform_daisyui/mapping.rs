//! The mapping between `schemaform-dioxus` edit hooks and presentation and the
//! `dioxus-field` convention the registry widgets speak.
//!
//! Everything correctness-critical about editing (IME composition, lifecycle
//! discard, DOM resynchronisation after a rejected write, error reporting)
//! stays in `schemaform-dioxus`. This module only translates shapes, and is
//! compiled against the demo's own `dioxus-field` so a version mismatch with the
//! copied registry widgets is a compile error rather than an unbound field.

use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_field::{Binding, ChangeOrigin, FieldMetaValues};
use dioxus_primitives::checkbox::CheckboxState;
use schemaform_dioxus::{
    BooleanEdit, ChoiceEdit, ChoiceIdentity, ControlFacets, NodePresentation, TextEdit,
    render::{FindingDescriptor, FindingKind},
};

/// The comparable identity of a binding built from one control's edit: its hook-stable handles.
///
/// An edit's own equality may also compare presentation state (a `TextEdit` compares
/// `read_only`, a `ChoiceEdit` its option list) that the core or the localizer may change while
/// the control's handles stay the same. Only the handles decide whether two bindings behave
/// interchangeably, so only they make up the identity: a binding built from the same control on
/// a later render compares equal even after such a change.
#[derive(Clone, PartialEq)]
struct EditHandles<V: 'static, W: 'static> {
    value: ReadSignal<V>,
    write: Callback<W>,
    blur: Callback<()>,
}

/// The Commit capability of every binding here: a no-op, because the core applies each edit as
/// it happens and has no separate interaction unit.
fn use_no_commit() -> Callback<()> {
    use_callback(|()| {})
}

/// The Focus Exit capability of every binding here: marks the control touched through the
/// edit's `blur`.
fn use_focus_exit(blur: Callback<()>) -> Callback<()> {
    use_callback(move |()| blur.call(()))
}

/// Adapts a [`TextEdit`] to the `dioxus-field` binding contract.
///
/// This is a hook: call it unconditionally in the renderer's child component, after
/// [`schemaform_dioxus::use_text_edit`]. Write applies the text through `TextEdit::input`
/// regardless of its change origin; Commit is a no-op because the core applies every keystroke;
/// Focus Exit marks the control touched through `TextEdit::blur`.
///
/// The binding's identity is the edit's hook-stable handles, so bindings built from the same
/// control on different renders compare equal and the registry widgets that receive them
/// neither re-render per keystroke nor re-register focus.
pub fn use_text_binding(edit: TextEdit) -> Binding<String> {
    let write = use_callback(move |(text, _origin): (String, ChangeOrigin)| {
        edit.input.call(text);
    });
    let commit = use_no_commit();
    let focus_exit = use_focus_exit(edit.blur);
    let identity = EditHandles {
        value: edit.value,
        write: edit.input,
        blur: edit.blur,
    };
    Binding::new_with_identity(edit.value, write, commit, identity)
        .with_focus_exit_using_identity(focus_exit)
}

/// Adapts a [`BooleanEdit`] to a tri-state `dioxus-field` binding.
///
/// This is a hook: call it unconditionally in the renderer's child component, after
/// [`schemaform_dioxus::use_boolean_edit`]. The value is the edit's own tri-state: `None` is
/// JSON null, and a write-only control always reads `None`. Write applies the state through
/// `BooleanEdit::set`, which picks set null, set value, or replace value at event time; Commit is
/// a no-op because choosing a state is one complete interaction unit the core already applied;
/// Focus Exit marks the control touched through `BooleanEdit::blur`.
pub fn use_boolean_binding(edit: BooleanEdit) -> Binding<Option<bool>> {
    let write = use_callback(move |(checked, _origin): (Option<bool>, ChangeOrigin)| {
        edit.set.call(checked);
    });
    let commit = use_no_commit();
    let focus_exit = use_focus_exit(edit.blur);
    Binding::new_with_identity(edit.checked, write, commit, boolean_handles(edit))
        .with_focus_exit_using_identity(focus_exit)
}

/// Adapts a [`BooleanEdit`] to the registry `Checkbox`'s tri-state binding, with JSON null
/// shown as the indeterminate state.
///
/// This is a hook: call it unconditionally in the renderer's child component, after
/// [`schemaform_dioxus::use_boolean_edit`]. The `CheckboxState` mapping lives here and nowhere
/// else: `Some(true)` is checked, `Some(false)` is unchecked, `None` is indeterminate, and a
/// write in either direction follows the same table, so writing indeterminate sets null.
/// Commit is a no-op and Focus Exit marks the control touched, as for
/// [`use_boolean_binding`].
pub fn use_checkbox_binding(edit: BooleanEdit) -> Binding<CheckboxState> {
    let state = use_memo(move || checkbox_state(edit.checked.cloned()));
    let read = use_hook(|| ReadSignal::new(state));
    let write = use_callback(move |(state, _origin): (CheckboxState, ChangeOrigin)| {
        edit.set.call(checked_of(state));
    });
    let commit = use_no_commit();
    let focus_exit = use_focus_exit(edit.blur);
    Binding::new_with_identity(read, write, commit, boolean_handles(edit))
        .with_focus_exit_using_identity(focus_exit)
}

/// A boolean edit's handles, the identity of every binding built from it.
fn boolean_handles(edit: BooleanEdit) -> EditHandles<Option<bool>, Option<bool>> {
    EditHandles {
        value: edit.checked,
        write: edit.set,
        blur: edit.blur,
    }
}

/// The checkbox state that shows a boolean edit's tri-state: null is indeterminate.
fn checkbox_state(checked: Option<bool>) -> CheckboxState {
    match checked {
        Some(true) => CheckboxState::Checked,
        Some(false) => CheckboxState::Unchecked,
        None => CheckboxState::Indeterminate,
    }
}

/// The boolean edit's tri-state a checkbox state writes: indeterminate is null.
fn checked_of(state: CheckboxState) -> Option<bool> {
    match state {
        CheckboxState::Checked => Some(true),
        CheckboxState::Unchecked => Some(false),
        CheckboxState::Indeterminate => None,
    }
}

/// Adapts a [`ChoiceEdit`] to the binding a registry `NativeSelect` or `Select` over
/// [`ChoiceIdentity`] speaks.
///
/// This is a hook: call it unconditionally in the renderer's child component, after
/// [`schemaform_dioxus::use_choice_edit`]. The value is the selected option's opaque identity,
/// `None` while the data matches no option and always for a write-only control. Write hands the
/// identity to `ChoiceEdit::select`, which sets null for the null option and picks set value or
/// replace value at event time for the others; Commit is a no-op because choosing an option is
/// one complete interaction unit the core already applied; Focus Exit marks the control touched
/// through `ChoiceEdit::blur`.
pub fn use_choice_binding(edit: ChoiceEdit) -> Binding<Option<ChoiceIdentity>> {
    let select = edit.select;
    let write = use_callback(
        move |(identity, _origin): (Option<ChoiceIdentity>, ChangeOrigin)| {
            select.call(identity);
        },
    );
    let commit = use_no_commit();
    let focus_exit = use_focus_exit(edit.blur);
    Binding::new_with_identity(edit.selected, write, commit, choice_handles(&edit))
        .with_focus_exit_using_identity(focus_exit)
}

/// Adapts a [`ChoiceEdit`] to the string binding a registry `RadioGroup` speaks.
///
/// This is a hook: call it unconditionally in the renderer's child component, after
/// [`schemaform_dioxus::use_choice_edit`]. The value is the selected option's identity as
/// [`ChoiceIdentity::as_str`], or the empty string while nothing is selected, so an item whose
/// `value` is its option's identity string is the checked one. Write maps the string back to the
/// option carrying it and selects that option; a string no option carries selects nothing, which
/// the edit treats as a no-op. Commit and Focus Exit behave as for [`use_choice_binding`].
pub fn use_radio_binding(edit: ChoiceEdit) -> Binding<String> {
    let selected = edit.selected;
    let value = use_memo(move || {
        selected
            .read()
            .as_ref()
            .map(|identity| identity.as_str().to_owned())
            .unwrap_or_default()
    });
    let read = use_hook(|| ReadSignal::new(value));
    let identity = choice_handles(&edit);
    let select = edit.select;
    let blur = edit.blur;
    // `use_callback` keeps the handle and replaces the closure on every render, so the lookup
    // always sees the option list of the latest render.
    let options = edit.options;
    let write = use_callback(move |(value, _origin): (String, ChangeOrigin)| {
        let identity = options
            .iter()
            .find(|option| option.identity.as_str() == value)
            .map(|option| option.identity.clone());
        select.call(identity);
    });
    let commit = use_no_commit();
    let focus_exit = use_focus_exit(blur);
    Binding::new_with_identity(read, write, commit, identity)
        .with_focus_exit_using_identity(focus_exit)
}

fn choice_handles(
    edit: &ChoiceEdit,
) -> EditHandles<Option<ChoiceIdentity>, Option<ChoiceIdentity>> {
    EditHandles {
        value: edit.selected,
        write: edit.select,
        blur: edit.blur,
    }
}

/// Field metadata for one control, from its node presentation and control facets.
///
/// `invalid` is the adapter's verdict rather than derived from `errors`, and `errors` holds
/// only the findings [`is_field_error`] accepts. The remaining findings are the renderer's to
/// present separately.
pub fn field_meta_values(
    presentation: &NodePresentation,
    control: &ControlFacets,
) -> FieldMetaValues {
    FieldMetaValues {
        id: Some(Rc::from(presentation.element_id.as_str())),
        name: Some(Rc::from(control.name.as_str())),
        required: control.required,
        disabled: control.disabled,
        invalid: Some(presentation.invalid),
        errors: presentation
            .findings
            .iter()
            .filter(|finding| is_field_error(finding))
            .map(|finding| Rc::from(finding.text.as_str()))
            .collect(),
        touched: control.touched,
        dirty: control.dirty,
    }
}

/// Whether a finding belongs in `FieldMetaValues::errors`: it blocks submission and it is about
/// the value at the control, that is a parse blocker, a validation finding, or a blocking
/// external finding.
///
/// The registry's `FieldError` presents errors only while the field is invalid, and these are
/// the findings whose text belongs in that region. Advisory external findings do not make the
/// node invalid. Capability findings describe what the form can present rather than the value
/// the user entered, so even a blocking one is presented as a description instead; the node is
/// still exposed as invalid through `FieldMetaValues::invalid`.
pub fn is_field_error(finding: &FindingDescriptor) -> bool {
    finding.blocking
        && matches!(
            finding.kind,
            FindingKind::Parse | FindingKind::Validation | FindingKind::External
        )
}
