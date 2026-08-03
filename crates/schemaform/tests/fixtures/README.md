# Official Draft 2020-12 fixtures

`draft202012-official-suite.json` is a filesystem-free bundle of the [JSON Schema Test Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite) at commit `c0b038ad7244712cf73650f44e90d0bc5704e8c7`.

It contains:

- All immediate JSON files under `tests/draft2020-12`.
- Optional `bignum.json`, `ecmascript-regex.json`, `float-overflow.json`, and `non-bmp-regex.json`.
- All resources under `remotes/draft2020-12`, assigned the suite's documented `http://localhost:1234/draft2020-12/` retrieval URIs.

The integration test asserts the pinned revision and exact file, group, and case counts. `JSON-Schema-Test-Suite-LICENSE` preserves the upstream MIT notice.
Suite file identifiers are stored as relative filenames so generated fixtures do not depend on the checkout location.
The bundle is generated with arbitrary-precision `serde_json`, not a JavaScript JSON parser, so official numeric literals remain exact.

## UI schema v1 fixtures

`ui-schema-v1/complete.json` is the canonical accepted stable-wire fixture. It
round-trips through `parse_ui_schema_v1`, validates against
`ui-schema-v1.schema.json`, and compiles through the public definition
builder. `ui-schema-v1/rejected.json` covers obsolete and unknown versions,
strict wire failures, invalid bindings and generation, duplicate ownership, and
extension consistency. Change these fixtures, the meta-schema, typed builders,
serializers, support profile, and compatibility documentation together whenever
the stable contract changes; a semantic or core-vocabulary change requires a
new integer wire major.

From a clean JSON Schema Test Suite checkout at the pinned revision, regenerate it with:

```sh
cargo run --locked -p schemaform --example bundle_official_suite -- \
  /path/to/JSON-Schema-Test-Suite \
  crates/schemaform/tests/fixtures/draft202012-official-suite.json
```
