# Desktop smoke checklist

`schemaform-dioxus` runs one code path in a browser and in a desktop WebView:
every DOM touch — moving focus and putting a canonical value back into a widget
after the core rejects a write — is a fixed script run through Dioxus's
`document::eval`. The browser suite is evidence for those *scripts*. This
checklist is evidence for the *transport*: that `document::eval` in each
platform's WebView actually reaches them, and that the WebView's own input
events reach the adapter. A person runs it; nothing automated drives a WebView.

What it does not cover, plainly:

- **No automated WebView run.** No CI job, Playwright project, or `wasm-bindgen`
  runner opens WebKitGTK, WKWebView, or WebView2. A cell in the results table is
  as recent as the date in it.
- **No artifact-size gate for the desktop binary.** The gate in
  `browser/workload-pack/artifact-manifest.json` measures the browser workload
  runner's WASM and JavaScript; the desktop executable has no cap and no
  recorded size.
- **Not a platform support claim.** Browser CSR remains the tested platform.
  The adapter README's platform boundary describes what a desktop host must
  provide (a `Document` provider) and what it gets (asynchronous focus).

## Running the demo as a desktop application

From `demo/`, with the CSS built and the pinned `dx` installed as the demo
README describes:

```console
dx serve --platform desktop
```

`dx build --platform desktop` produces the executable under
`target/dx/demo/debug/<os>/app/`. Both select the demo's `desktop` Cargo
feature; `dx serve` without a platform, `cargo test`, and the Dagger checks stay
web builds through the `default` feature.

The WebView is whatever the platform ships: **WebKitGTK** on Linux,
**WKWebView** on macOS, **WebView2** on Windows. Linux needs GTK 3, WebKitGTK
4.1, libsoup 3, OpenSSL, and libxdo at build time; the `devenv` shell provides
them. macOS and Windows need nothing beyond the system WebView.

The demo reports adapter failures with `form operation failed: …` on standard
error, which `dx serve` shows in its log; in a browser the same line goes to
the console.

To run without a display on Linux, prefix the command with `xvfb-run -a` (also
in the shell). WebKitGTK still needs an EGL display; on a host without a system
Mesa, point `__EGL_VENDOR_LIBRARY_FILENAMES` and `LIBGL_DRIVERS_PATH` at a Mesa
build. Debug builds enable WebKit's developer extras, so
`WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9223` exposes a remote inspector through
which `document.activeElement`, control values, and live-region text can be
read while the scenarios are driven with real input (`xdotool`).

## Scenarios

The pages are the ones the browser check in `demo/e2e/` drives; the affordance
names below are their accessible names. Where a step says "expected", that is
the adapter's promise, not the daisyUI component's. Start each group from a
fresh page.

### Focus after a blocked submission — `/daisyui`

| Id | Steps | Expected |
| --- | --- | --- |
| `blocked-submit` | Replace the Display name with `Ad` and press Tab. Select the Billing tab. Activate Submit. | Submission is blocked. Focus lands on the finding summary (the region labelled "Finding summary"), which lists one finding, "Value does not satisfy minLength." |
| `summary-to-target` | From `blocked-submit`, activate the finding's link in the summary. | The Profile tab is revealed and the Display name control has focus. This is the path that needs the focus script's animation-frame retries on a WebView: the tab click travels to Rust and the panel's edits come back over the socket before the target exists. |

### Collection affordances leave focus where the adapter promises — `/arrays`

Baseline: Tags `rust`, `dioxus`; Team members Ada, Lin. Each step announces
through the collection's live region (`[data-array-status]`).

| Id | Steps | Expected |
| --- | --- | --- |
| `array-insert` | Activate "Insert Tags item before position 1". | A `new-tag` item at position 1 with focus in its control; the two existing items keep their element ids. Announced: "Tags item inserted at position 1." |
| `array-move-down` | Activate "Move Tags item at position 1 down". | The item is at position 2. Focus stays on that same item's move-down affordance, now named "Move Tags item at position 2 down". Announced: "Tags item moved down to position 2." |
| `array-move-up` | Activate "Move Tags item at position 2 up". | The item is back at position 1. Its move-up affordance is withdrawn there, so focus falls to the same item's move-down affordance, "Move Tags item at position 1 down". Announced: "Tags item moved up to position 1." |
| `array-remove` | Activate "Remove Tags item at position 1". | Focus moves to the next item's control (`/tags/0`, now `rust`), which keeps its element id. Announced: "Tags item removed from position 1." |
| `array-append` | Activate "Add Team members item". | A third item whose Name control has focus. Announced: "Team members item added at position 3." |

### Resynchronisation after a write the core does not keep — `/daisyui`

The adapter runs the same two scripts after a rejected write and after a write
to a write-only control: one sets `value` on a text control or a choice
`<select>`, the other sets `checked` on a checkbox or the value of a boolean
`<select>`. The demo has no by-hand way to make the core refuse a boolean or
choice write, so its write-only controls are the trigger: they take the same
resynchronisation path with the same scripts, which is the transport this
checklist is evidence for. Two things stay unexercised by hand and rest on the
browser suite: the decision to resynchronise *because* a write was rejected, and
the script's checkbox branch (`checked =`), since the demo's write-only boolean
is a `<select>`. A rejected write on a text control needs the inspector, and
only where the engine allows it (see the note).

| Id | Steps | Expected |
| --- | --- | --- |
| `resync-boolean` | Security tab. In "Replace Two-factor authentication", choose an option other than the one chosen last. | The `<select>` rests on "Choose replacement" again after the write. |
| `resync-choice` | Security tab. In "Replace Recovery channel", choose an option other than the one chosen last. | The `<select>` rests on "Choose replacement" again after the write. |
| `resync-text` | Profile tab. Age is `36`. From the WebView's inspector console: `const age = document.querySelector('[name="/age"]'); age.value = "9".repeat(512 * 1024 + 1); age.dispatchEvent(new Event("input", { bubbles: true }));` | The core refuses the edit buffer (over 512 KiB). The control shows `36` again and the host log carries `form operation failed: …`. |

Note on `resync-text`: WebKit caps an `<input>` value at 524,288 characters —
by typing, by pasting, and through the `value` setter — which is exactly the
adapter's edit-buffer limit, so the refusal cannot be reached in WebKitGTK, and
WKWebView, on the same engine, is expected to behave the same. Record it as
blocked there; the value script's transport is exercised by `resync-choice`.
WebView2 (Chromium) is expected to accept the oversized value.

On a `<select>`, WebKit fires `change` only when the selection differs from the
last one it committed; a value the script put back does not count as
committed. Choosing the same option twice in a row therefore writes nothing the
second time, which is why the steps say "other than the one chosen last".

### IME composition — `/daisyui`

| Id | Steps | Expected |
| --- | --- | --- |
| `ime` | Profile tab. With an input method active (for example Japanese Hiragana or Chinese Pinyin), focus Display name, type a sequence that opens a composition, then commit it. | While composing, the control shows the composition and nothing overwrites it. On commit the composed text is the control's value; Submit then shows it in the status line. |

## Results

One column per WebView. A cell is `pass <date>`, `blocked <date> (why)` when
the run attempted the scenario and could not carry it out, or `unrun` when no
run has attempted it. Dates are the run's; the run log below has the details.

| Scenario | WebKitGTK | WKWebView | WebView2 |
| --- | --- | --- | --- |
| `blocked-submit` | pass 2026-09-07 | unrun | unrun |
| `summary-to-target` | pass 2026-09-07 | unrun | unrun |
| `array-insert` | pass 2026-09-07 | unrun | unrun |
| `array-move-down` | pass 2026-09-07 | unrun | unrun |
| `array-move-up` | pass 2026-09-07 | unrun | unrun |
| `array-remove` | pass 2026-09-07 | unrun | unrun |
| `array-append` | pass 2026-09-07 | unrun | unrun |
| `resync-boolean` | pass 2026-09-07 | unrun | unrun |
| `resync-choice` | pass 2026-09-07 | unrun | unrun |
| `resync-text` | blocked 2026-09-07 (WebKit caps `<input>` at 524,288 characters) | unrun | unrun |
| `ime` | blocked 2026-09-07 (no input method in the headless run) | unrun | unrun |

## Runs

### 2026-09-07 — WebKitGTK 2.52.6, Linux x86_64, headless

- Source: the commit that added this checklist, on top of `42a7351`.
  `dx` 0.7.10, `dioxus-desktop` 0.7.10, `wry` 0.53.5, `tao` 0.34.8, rustc
  1.98.0; GTK 3.24.52 and WebKitGTK 2.52.6 from the `devenv` shell on an
  Ubuntu 24.04 host.
- `dx build --platform desktop` and `dx serve --platform desktop --interactive
  false --watch false` both succeeded; `dx` selected `features: ["desktop"]`,
  launched the executable, and its devtools socket connected.
- Display: `xvfb-run` 1280×1024, Mesa 25.2.3 llvmpipe for EGL. GTK reported
  "Disabled hardware acceleration" and rendered in software. The window was
  resized to 1260×980 so the gallery used its desktop layout.
- Input: real X11 pointer and keyboard events through `xdotool` (clicks on
  affordances and tabs, Tab, arrow keys, typing). Observation: the WebKit remote
  inspector, reading `document.activeElement`, control values and ids, and
  `[data-array-status]`. `<select>` options were chosen with the arrow keys
  after focusing the control through its label, which avoids the native popup.
- `blocked-submit`: focus landed on `#schemaform-1-summary`
  (`role="region"`, "Finding summary"); one finding listed.
- `summary-to-target`: the Profile tab became `aria-selected="true"` and
  `[name="/name"]` had focus.
- `array-insert`, `array-move-down`, `array-move-up`, `array-remove`,
  `array-append`: focus and announcements exactly as expected; the moved and
  surviving items kept their element ids across every operation.
- `resync-boolean`, `resync-choice`: the `change` event carried the chosen
  option (`false`; `choice-1` = sms) and the `<select>` rested on the
  placeholder afterwards.
- `resync-text`: blocked. WebKitGTK truncated the value to 524,288 characters
  both through `execCommand("insertText")` and through the `value` setter (a
  `<textarea>` was not truncated), so the core saw a 512 KiB buffer, accepted
  it, and nothing was refused.
- `ime`: blocked — no input method is available in the headless environment,
  so no composition could be started.
- Nothing was written to the host log by `report_form_error` during the run,
  consistent with no write having been refused.
