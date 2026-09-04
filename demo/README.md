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
  into `assets/style.css`; `src/forms.css` styles the built-in renderer's
  `schemaform-*` class hooks.

The gallery covers generated controls compiled from a data schema, homogeneous
arrays with stable item identity, authored UI schemas with layouts and tabs,
validation and submission outcomes, a form whose every control is
daisyUI-rendered through a custom control renderer, and a playground for
editing a data schema and UI schema side by side with the rendered form.
