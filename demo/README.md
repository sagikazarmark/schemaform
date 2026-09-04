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

The native tests (example data schemas and the `schemaform_daisyui` mapping) run
with `cargo test` from this directory; the Dagger pipeline runs them as the
`test` check next to the wasm bundle.

## Browser check

The daisyUI page has a Playwright + axe check in `e2e/`: it opens `/daisyui`
(and the RTL variant) from the built bundle, runs axe-core at named checkpoints
in both site themes with zero violations allowed, and verifies finding-summary
focus-to-target and presence repair on the daisyUI-rendered controls. The Dagger
pipeline runs it as the `accessibility` check; the `Dagger (demo)` workflow runs
every check on pushes to `main` and on every pull request, so a change to the
demo or to `schemaform-dioxus` (which the demo builds from the workspace) is
covered. To run it yourself, from this directory:

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
  `schemaform_daisyui`, the daisyUI control renderer built on them (see its
  README). They are committed; CI never runs `dx components add`.
- `style.css` is the Tailwind and daisyUI input that `npm run build` compiles
  into `assets/style.css`. It imports `src/forms.css`, the daisyUI theme for
  the built-in renderer's `schemaform-*` class hooks: daisyUI's component
  classes applied to the hooks with `@apply`, plus the few rules a class cannot
  express (the tab buttons, the checkbox row, and the opt-out a form inside
  `[data-schemaform-unstyled]` uses to render with no theme). `style.css` also
  darkens the light theme's `error` colour, which daisyUI ships too light to
  pass as text.
- `e2e/` is the Playwright + axe check for the daisyUI page, its own npm
  project with exact pins (see its README).

The gallery covers generated controls compiled from a data schema, homogeneous
arrays with stable item identity, authored UI schemas with layouts and tabs,
validation and submission outcomes, a form whose every control is
daisyUI-rendered through a custom control renderer with the built-in structure
themed through its class hooks (in both site themes, with a right-to-left
variant and an unstyled built-in comparison of the same definition and
baseline), and a
playground for editing a data schema and UI schema side by side with the
rendered form.
