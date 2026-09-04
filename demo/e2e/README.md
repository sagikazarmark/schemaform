# Playwright + axe for the daisyUI page

A browser check for the demo's daisyUI page: Playwright drives `/daisyui` and
`/daisyui/rtl` in the light and dark themes, runs [axe-core] at named
checkpoints, and verifies the two behaviours a custom control renderer is most
likely to break: finding-summary focus-to-target and presence repair.

It exists because the daisyUI controls are not the built-in renderer. The
adapter's own browser suite covers the built-in; this is the automated
accessibility coverage for a renderer that owns its whole presentation.

## What it checks

Per theme, in order, with an axe run at each checkpoint:

| Checkpoint             | State                                                                                                        |
| ---------------------- | ------------------------------------------------------------------------------------------------------------ |
| `profile`              | The page as loaded, Profile tab.                                                                             |
| `profile-invalid-name` | A two-character display name left behind: the control is `aria-invalid`, the summary lists the finding.      |
| `billing`              | Billing tab: radio group, compound select, price, billing address fixed object.                              |
| `blocked-submit`       | Submit clicked from Billing: blocked, focus moved to the finding summary.                                    |
| `focus-to-target`      | The summary's finding button clicked: the Profile tab is revealed and the display name control has focus.    |
| `presence-set`         | "Set Nickname" on the null nickname: an editable empty string, set-null and remove offered, text typed.      |
| `presence-set-null`    | "Set Nickname to null": null again, set and remove offered.                                                  |
| `presence-remove`      | "Remove Nickname": the value is gone, set and set-null offered. Set again afterwards.                        |
| `security`             | Security tab: write-only boolean, choice, and string.                                                        |
| `team`                 | Team tab: homogeneous arrays of fixed objects and strings.                                                   |
| `rtl-profile`          | `/daisyui/rtl` as loaded.                                                                                    |

Axe runs over the example region (`[role="region"][aria-label="Example demo"]`:
the form, its reset button, and its status line) with axe's default rule set,
and every checkpoint must report zero violations. The gallery shell around the
region is shared by every demo page and is not the subject of this check.

The theme is selected the way the site does it, through `localStorage`
`demo-theme`, and asserted on `<html data-theme>`. Before each axe run the
pointer is parked and CSS transitions are awaited, so a hover left by the last
click is not measured mid-fade.

The check fails (exit code 1) on any violation, any failed step, or any
uncaught page error.

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
