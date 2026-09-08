//! The adapter's only DOM bridge: every focus movement and every rejected-write
//! resynchronisation goes through [`dioxus::document::Document::eval`], so one implementation
//! serves the browser and desktop and mobile WebViews alike.
//!
//! The scripts below are fixed source; element ids, values and checked state reach them as data
//! through `dioxus.recv()`, never by formatting into the source. Ids may derive from a host's
//! schema and values from user input, and the platform's channel is responsible for encoding.
//!
//! The scripts never `return` early: the web renderer appends `dioxus.close()` after the script
//! body, and an early return would leave the channel registered.
//!
//! Every `Eval` result is dropped: on a native `VirtualDom` without a document provider (the
//! adapter's headless tests), `NoOpDocument` answers `Err(EvalError::Finished)` to `send()`, and
//! on a real platform a failed focus is best-effort by contract.

use dioxus::document::Document;
use serde_json::{Value, json};

/// Focuses the first resolvable target after revealing the tabs that contain it.
///
/// Request: `{ "tabs": [id], "targets": [id], "item": id | null, "row": id | null }`. Every
/// `tabs` id is clicked first, in order, so a tab button's own handler selects the panel. The
/// element to focus is then the first `targets` id present in the document — or, when that
/// element carries `data-focus-first-descendant`, the first focusable element inside it, falling
/// back to the element itself, as the built-in multiple choice's fieldset focuses its first
/// checkbox —
/// or, when `item` is set, the item root `#item` if it is focusable, else the first focusable
/// element inside it, else the first focusable element inside `#row`.
///
/// Absent request members compare with `== null` rather than `=== null`: the web channel
/// delivers JSON `null` faithfully, but a transport that drops nulls would deliver `undefined`.
///
/// Focus is retried in two phases because the target may not exist yet: a re-render triggered by
/// a tab click completes within microtasks on the web renderer, while on a WebView the click
/// travels to Rust and the resulting edits return over a socket, which needs animation frames.
const FOCUS_SCRIPT: &str = r#"
const request = await dioxus.recv();
const FOCUSABLE = "input:not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])";
for (const id of request.tabs) {
    const tab = document.getElementById(id);
    if (tab instanceof HTMLElement) tab.click();
}
const firstFocusableInside = (id) => {
    const root = document.getElementById(id);
    return root ? root.querySelector(FOCUSABLE) : null;
};
const resolve = () => {
    if (request.item != null) {
        const root = document.getElementById(request.item);
        if (root && root.matches(FOCUSABLE)) return root;
        return firstFocusableInside(request.item) ?? firstFocusableInside(request.row);
    }
    for (const id of request.targets) {
        const element = document.getElementById(id);
        if (!element) continue;
        if (element.hasAttribute("data-focus-first-descendant")) {
            return firstFocusableInside(id) ?? element;
        }
        return element;
    }
    return null;
};
const attempt = () => {
    const element = resolve();
    if (!(element instanceof HTMLElement)) return false;
    element.focus();
    return document.activeElement === element;
};
let focused = attempt();
for (let i = 0; !focused && i < 4; i += 1) {
    await Promise.resolve();
    focused = attempt();
}
for (let i = 0; !focused && i < 10; i += 1) {
    await new Promise((next) => requestAnimationFrame(next));
    focused = attempt();
}
"#;

/// Puts a canonical textual value back into a control.
///
/// Request: `{ "id": id, "value": string }`. Used for text controls and for a choice `<select>`,
/// whose value is the selected option's identity.
const RESYNCHRONIZE_VALUE_SCRIPT: &str = r#"
const request = await dioxus.recv();
const control = document.getElementById(request.id);
if (control) control.value = request.value;
"#;

/// Puts a canonical boolean back into a checkbox or a tri-state `<select>`.
///
/// Request: `{ "id": id, "checked": true | false | null }`. A `<select>` takes `""` for null and
/// `"true"`/`"false"` otherwise; any other control takes `checked === true`.
const RESYNCHRONIZE_BOOLEAN_SCRIPT: &str = r#"
const request = await dioxus.recv();
const control = document.getElementById(request.id);
if (control instanceof HTMLSelectElement) {
    control.value = request.checked == null ? "" : String(request.checked);
} else if (control) {
    control.checked = request.checked === true;
}
"#;

/// Where a focus request resolves in the DOM.
#[derive(Clone, Copy)]
pub(crate) enum Target<'a> {
    /// One element by id.
    Element(&'a str),
    /// The first of these element ids that exists in the document.
    FirstOf(&'a [String]),
    /// An item by its root id and its wrapper row id: the item root if focusable, else the first
    /// focusable element inside it, else the first focusable element inside the row.
    Item { root: &'a str, row: &'a str },
}

/// Evaluates `script` and hands it `request` through `dioxus.recv()`.
fn eval_with_request(document: &dyn Document, script: &str, request: Value) {
    let eval = document.eval(script.to_owned());
    let _ = eval.send(request);
}

/// Reveals `tabs` by clicking each tab button, then focuses `target`, retrying briefly while the
/// revealed panel renders.
pub(crate) fn focus(document: &dyn Document, tabs: &[&str], target: Target<'_>) {
    let (targets, item, row) = match target {
        Target::Element(id) => (json!([id]), Value::Null, Value::Null),
        Target::FirstOf(ids) => (json!(ids), Value::Null, Value::Null),
        Target::Item { root, row } => (json!([]), json!(root), json!(row)),
    };
    eval_with_request(
        document,
        FOCUS_SCRIPT,
        json!({ "tabs": tabs, "targets": targets, "item": item, "row": row }),
    );
}

/// Focuses `target` in the current runtime's document.
pub(crate) fn focus_in_runtime(target: Target<'_>) {
    focus(&*dioxus::document::document(), &[], target);
}

/// Puts `value` back into the control with `id`.
pub(crate) fn resynchronize_control_value(id: &str, value: &str) {
    eval_with_request(
        &*dioxus::document::document(),
        RESYNCHRONIZE_VALUE_SCRIPT,
        json!({ "id": id, "value": value }),
    );
}

/// Puts `checked` back into the boolean control with `id`.
pub(crate) fn resynchronize_boolean(id: &str, checked: Option<bool>) {
    eval_with_request(
        &*dioxus::document::document(),
        RESYNCHRONIZE_BOOLEAN_SCRIPT,
        json!({ "id": id, "checked": checked }),
    );
}
