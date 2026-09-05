# Schemaform demo

A docs-by-example gallery for `schemaform` and `schemaform-dioxus`.

## Run locally

Install the pinned CSS dependencies and the Dioxus CLI version matching this
workspace, then run the web app from this directory:

```console
npm ci
npm run build
cargo install dioxus-cli --version 0.7.10 --locked
dx serve
```

Open the URL printed by `dx`. For live CSS rebuilding, run `npm run watch`
alongside `dx serve`.

The native tests run with `cargo test` from this directory; the Dagger pipeline
runs them as the `test` check next to the wasm bundle. The examples' tests sit
beside the examples; the `schemaform_daisyui` component's tests live in
`tests/schemaform_daisyui/` rather than in the component directory, so what a
`dx components add` install would copy ships no test code.

Those tests, and the browser check below, are split in two groups along one
line: what asserts the **adapter's contract** through the daisyUI package as a
real consumer (`tests/schemaform_daisyui/contract.rs`, the `contract-*`
Playwright scenarios) stays with schemaform when the component moves to its
registry; what asserts the **component's own presentation** (the other test
modules, the `presentation-*` scenarios) moves with it. The first group is how
schemaform is tested end to end against a renderer that owns its whole
presentation. It comes with a coupling to accept: once the component is
consumed from the registry at a pinned revision, a breaking change in
`schemaform-dioxus` fails this workspace's build until the registry catches
up — which is the signal a consumer test is for.

## Browser check

The daisyUI form and the arrays page have a Playwright + axe check in `e2e/`:
it opens `/daisyui` (and the RTL variant) and `/arrays` from the built bundle,
runs axe-core at named checkpoints in both site themes with zero violations
allowed, and verifies finding-summary focus-to-target and presence repair on the
daisyUI-rendered controls and add, insert, move, and remove with their focus and
announcements on the daisyUI-rendered arrays. The Dagger pipeline runs it as the
`accessibility` check; the `Dagger (demo)` workflow runs every check on pushes
to `main` and on every pull request, so a change to the demo or to
`schemaform-dioxus` (which the demo builds from the workspace) is covered. To
run it yourself, from this directory:

```console
dagger check accessibility
```

`e2e/README.md` lists the checkpoints and how to run the script against a
locally served app.

## Project layout

- `src/examples/` contains the small runnable components shown in the gallery.
- `src/pages/` adds explanation and quotes each example's exact source.
- `src/components/` contains the responsive shell and documentation UI, plus
  the `dx components` members: `button`, `checkbox`, `field`, `input`,
  `native_select`, `radio_group`, and `select` copied verbatim from the
  `dioxus-daisyui-components` registry at a pinned revision, and
  `schemaform_daisyui`, the daisyUI control renderer, structure bundle
  (collection and shell), and finding presenter built on them (see its README).
  They are committed; CI never runs `dx components add`.
- `src/lib.rs` exposes those modules as a library so that `tests/` can mount
  them; `src/main.rs` only launches the app. `tests/schemaform_daisyui/` holds
  the component's native tests.
- `style.css` is the Tailwind and daisyUI input that `npm run build` compiles
  into `assets/style.css`. It imports `src/forms.css`, the daisyUI theme for
  the `schemaform-*` class hooks of the built-in structure no renderer seam
  exists for yet (layouts, groups, tabs, and the built-in controls): daisyUI's
  component classes applied to the hooks with `@apply`, plus the few rules a
  class cannot express (the tab buttons, the checkbox row, and the opt-out a
  form inside `[data-schemaform-unstyled]` uses to render with no theme).
  Arrays, the form shell, and the finding summary are not in it; every gallery
  page renders those through the `schemaform_daisyui` seams. `style.css` also
  darkens the light theme's `error` colour, which daisyUI ships too light to
  pass as text.
- `e2e/` is the Playwright + axe check for the daisyUI form and the arrays
  page, its own npm project with exact pins (see its README).

The gallery covers generated controls compiled from a data schema, homogeneous
arrays with stable item identity rendered as daisyUI cards through the
collection seam, authored UI schemas with layouts and tabs, validation and
submission outcomes, a form whose controls, arrays, shell, and finding summary
are daisyUI-rendered through the renderer seams with the remaining built-in
structure themed through its class hooks (in both site themes, with a
right-to-left variant and an unstyled built-in comparison of the same
definition and baseline), and a playground for editing a data schema and UI
schema side by side with the rendered form.
