# Playwright + axe for the daisyUI pages

A browser check for the demo's daisyUI-rendered pages: Playwright drives
`/daisyui`, `/daisyui/rtl`, and `/arrays` in the light and dark themes, runs
[axe-core] at named checkpoints, and verifies the behaviours the component's
native tests cannot see, since they observe server-rendered markup rather than
a DOM.

The scenarios fall in two groups, kept apart in the script so the second can
move to the component's registry with it:

- **Adapter contract** (`contract-form`, `contract-arrays`) — what
  `schemaform-dioxus` promises every renderer package, exercised through a real
  one: finding visibility, the edit buffer behind a parse blocker,
  resynchronisation after a rejected write, blocked submission and summary
  focus, focus-to-target, presence affordances, write-only widgets resting
  after every write, and item identity, focus and announcements across array
  mutations. Every locator is a control's binding, an affordance's accessible
  name, a collection's label, or an adapter-owned attribute, so these fail when
  schemaform breaks a seam and not when daisyUI changes its markup. **This
  group stays with schemaform**: it is how the project is tested end to end
  against a consumer that owns its whole presentation. The adapter's own
  browser suite covers the same contract with test renderers; this is the
  real-consumer check on top of it.
- **daisyUI presentation** (`presentation-form`, `presentation-arrays`) — what
  `schemaform_daisyui` itself decides: which registry widget a kind renders as
  and how it behaves when driven (the native checkbox, the registry `Checkbox`
  showing null as indeterminate, the radio group, the compound select), the
  empty state, the item cards, and the chrome under right-to-left writing.
  **This group moves with the component.**

The axe checkpoints are the demo's and stay in both groups: they measure the
component together with the demo's theme, which is what the deployed site
shows. When the presentation group moves, the registry replaces them with its
own harness.

## What it checks

Per theme, in order, with an axe run at each checkpoint. Every scenario opens
its page afresh.

### Adapter contract

| Checkpoint             | State                                                                                                        |
| ---------------------- | ------------------------------------------------------------------------------------------------------------ |
| `profile`              | `/daisyui` as loaded, Profile tab.                                                                           |
| —                      | `abc` typed into the age: kept as typed, `aria-invalid`, the element `aria-errormessage` references names the parse blocker; then `40`. An over-limit edit rejected: resynchronised to `40`, the failure logged by the host. Submitted: the status line shows `"age": 40`. Reset to baseline. |
| `profile-invalid-name` | A two-character display name left behind: the control is `aria-invalid`, the summary lists the finding.      |
| `blocked-submit`       | Submit clicked from the Billing tab: blocked, focus moved to the finding summary.                             |
| `focus-to-target`      | The summary's finding button clicked: the Profile tab is revealed and the display name control has focus.    |
| `presence-set`         | "Set Nickname" on the null nickname: an editable empty string, set-null and remove offered, text typed.      |
| `presence-set-null`    | "Clear Nickname" (set null): null again, set and remove offered.                                             |
| `presence-remove`      | "Remove Nickname": the value is gone, set and set-null offered. Set again afterwards.                        |
| `security-write-only`  | Security tab: a value chosen in the write-only boolean's replacement select and in the write-only choice, both resting on their placeholder again; the write-only string an empty password input. |
| `arrays`               | `/arrays` as loaded: the Tags and Team members collections as labelled groups.                               |
| `arrays-inserted`      | "Insert Tags item before position 1" clicked: the seeded tag has focus and the insertion is announced.       |
| `arrays-mutated`       | The inserted tag moved down (focus stays on its move-down button) and removed (focus moves to the next tag), then a team member appended (focus in the new item); each announced. |
| —                      | Every tag removed: the removal is announced, append is still offered.                                        |
| `arrays-min-items`     | Team members reduced to one: the core withdraws removal and no remove button remains.                        |

### daisyUI presentation

| Checkpoint        | State                                                                                                             |
| ----------------- | ----------------------------------------------------------------------------------------------------------------- |
| `profile-widgets` | `/daisyui`: the native checkbox unchecked; the nullable registry `Checkbox` taken from indeterminate to checked by a click, back to null through "Clear", and checked again, with the indeterminate and checked marks both drawn and distinct; submitted, and the status line shows both writes. |
| `billing`         | Billing tab: radio group, compound select, price, billing address fixed object.                                   |
| `billing-widgets` | "monthly" clicked in the radio group (checked, "yearly" unchecked); the compound select opened and "us" chosen (trigger shows it, listbox closed). |
| `security`        | Security tab: write-only boolean, choice, and string.                                                             |
| `team`            | Team tab: homogeneous arrays of fixed objects and strings as daisyUI collections.                                 |
| `rtl-profile`     | `/daisyui/rtl` as loaded: the same form with its chrome mirrored.                                                 |
| `arrays-empty`    | `/arrays`: the first tag is a card named by noun and position; every tag removed, the empty state stands in for the cards. |

Axe runs over the example region (`[role="region"][aria-label="Example demo"]`:
the form, its reset button where the page has one, and its status line) with
axe's default rule set,
and every checkpoint must report zero violations. The gallery shell around the
region is shared by every demo page and is not the subject of this check.

The theme is selected the way the site does it, through `localStorage`
`demo-theme`, and asserted on `<html data-theme>`. Before each axe run the
pointer is parked and CSS transitions are awaited, so a hover left by the last
click is not measured mid-fade.

The four scenarios run separately within a theme, so a failure in one does not
hide another's checkpoints. The check fails (exit code 1) on any violation, any
failed step, or any uncaught page error.

## Running it

In CI, the demo's Dagger pipeline runs it as the `accessibility` check: it
mounts the `build` output into the Playwright image matching the version pinned
here and runs this script against it. From `demo/`:

```console
dagger check accessibility
```

Locally, against the served app (`dx serve` from `demo/`, see the demo README):

```console
cd e2e
npm ci
npx playwright install chromium
node daisyui.mjs --url http://127.0.0.1:8080
```

or against a built bundle (the directory holding `index.html`), which the
script serves itself with the deployed worker's single-page fallback:

```console
node daisyui.mjs --site ../target/dx/demo/release/web/public
```

`--out <dir>` (default `./artifacts`, git-ignored) receives the full axe report
and a full-page screenshot per checkpoint, the browser console and page errors
per theme, and `report.json`. The Dagger check keeps only the script's output:
a violation is reported there with its rule, impact, and the selectors of the
offending elements, which is enough to reproduce it locally with the commands
above. The browser is Chromium, the reference for axe.

## Pins

`package.json` pins `playwright` and `axe-core` exactly. The Dagger check reads
the Playwright version from it to pick the `mcr.microsoft.com/playwright`
image, so bumping the package bumps the browsers with it; a mismatch fails at
browser launch rather than silently testing another browser build.

[axe-core]: https://github.com/dequelabs/axe-core
