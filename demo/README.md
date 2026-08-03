# Schemaform demo

A docs-by-example gallery for `schemaform` and `schemaform-dioxus`.

## Run locally

Install the pinned CSS dependencies and the Dioxus CLI version matching this
workspace, then run the web app from this directory:

```console
npm ci
npm run build
cargo install dioxus-cli --version 0.7.9 --locked
dx serve
```

Open the URL printed by `dx`. For live CSS rebuilding, run `npm run watch`
alongside `dx serve`.

## Project layout

- `src/examples/` contains the small runnable components shown in the gallery.
- `src/pages/` adds explanation and quotes each example's exact source.
- `src/components/` contains the responsive shell and documentation UI.
- `src/style.css` and `src/forms.css` build the Tailwind and daisyUI stylesheet
  into `build/`.

The gallery covers generated controls compiled from a data schema, homogeneous
arrays with stable item identity, authored UI schemas with layouts and tabs,
validation and submission outcomes, and a playground for editing a data schema
and UI schema side by side with the rendered form.
