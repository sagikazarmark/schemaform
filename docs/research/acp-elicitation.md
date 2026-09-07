# ACP elicitation: stabilization, wire shape, and the permitted JSON Schema subset

Date: 2026-09-07

## Summary

"First-class" ACP elicitation means that `elicitation/create` (request) and
`elicitation/complete` (notification) are now part of the **stable** ACP v1
protocol surface: they appear in the stable `schema/v1/meta.json` and
`schema/v1/schema.json`, the RFD moved to *Completed* on 2026-07-24, and the
SDKs dropped their `unstable_elicitation` feature flags / `unstable_` method
prefixes (schema crate 1.7.0 and TypeScript SDK 1.4.0 on 2026-08-20; Rust SDK
2.1.0 on 2026-09-04). The ACP protocol version integer did not change (still
`1`); the feature is additive and gated by a client capability. A form renderer
that wants to cover 100% of ACP form-mode elicitation must handle a **flat**
object schema (`type: "object"`, `properties`, optional `required`, optional
schema-level `title`/`description`) whose property schemas are one of five
`type`s: `string` (with `title`, `description`, `minLength`, `maxLength`,
`pattern`, `format` ∈ {`email`,`uri`,`date`,`date-time`}, `default`, and the
single-select idioms `enum: [string]` or `oneOf: [{const, title, description?}]`),
`number` and `integer` (`title`, `description`, `minimum`, `maximum`, `default`),
`boolean` (`title`, `description`, `default`), and `array` for multi-select only
(`title`, `description`, `minItems`, `maxItems`, `default: [string]`, and
`items` being either `{type: "string", enum: [string]}` or
`{anyOf: [{const, title, description?}]}`). ACP is a superset of MCP's
`PrimitiveSchemaDefinition` (adds `pattern`, schema-level `title`/`description`,
`description` on enum options, `_meta`) and a strict subset in one respect: it
does **not** support MCP's deprecated `enumNames`. Nested objects, arrays of
non-enum items, `$ref`, `$schema`, conditionals, and every other JSON Schema
keyword are outside the subset; unknown property `type` values must be preserved
but not rendered.

---

## 1. What changed and when

### 1.1 The stabilization event

- The ACP **Elicitation RFD** moved to *Completed*, "stabilizing
  `elicitation/create` and the optional `elicitation/complete` notification".
  The announcement is dated **July 24, 2026**.
  [Elicitation is stabilized (announcement)](https://agentclientprotocol.com/announcements/elicitation-stabilized)
- RFD lifecycle: Draft (2026-01-12 initial draft) → Preview (**July 9, 2026**) →
  Completed (**July 24, 2026**).
  [RFD Updates](https://agentclientprotocol.com/rfds/updates) ·
  [RFD: Elicitation, "Revision history"](https://agentclientprotocol.com/rfds/elicitation#revision-history)
  (the RFD's own revision history lists the Completed entry as 2026-07-22; the
  PR merged and the announcement was published 2026-07-24 — see below).
- The implementing PR is
  [agentclientprotocol/agent-client-protocol#1779 "feat(schema): stabilize elicitation"](https://github.com/agentclientprotocol/agent-client-protocol/pull/1779),
  merged **2026-07-24T14:10:49Z**, merge commit
  `2c66decf67a2e5c6da9fabb44cc8b56f4a3840ce`. Its diff:
  - removes the `unstable_elicitation` Cargo feature from
    `agent-client-protocol-schema/Cargo.toml`;
  - removes `#[cfg(feature = "unstable_elicitation")]` from
    `agent-client-protocol-schema/src/v1/mod.rs` (and `v2/mod.rs`) so
    `mod elicitation; pub use elicitation::*;` are unconditional;
  - adds `elicitation_create` / `elicitation_complete` to `clientMethods` in
    `schema/v1/meta.json` and `schema/v2/meta.json`;
  - adds ~1,100 lines of elicitation definitions to the stable
    `schema/v1/schema.json` and `schema/v2/schema.json`;
  - adds `docs/protocol/v1/elicitation.mdx`, `docs/protocol/v2/elicitation.mdx`,
    and `docs/announcements/elicitation-stabilized.mdx`.

  Verbatim `schema/v1/meta.json` hunk from the PR:

  ```diff
       "terminal_wait_for_exit": "terminal/wait_for_exit",
  -    "terminal_kill": "terminal/kill"
  +    "terminal_kill": "terminal/kill",
  +    "elicitation_create": "elicitation/create",
  +    "elicitation_complete": "elicitation/complete"
     },
  ```

  Verbatim `agent-client-protocol-schema/src/v1/mod.rs` hunk:

  ```diff
   mod content;
  -#[cfg(feature = "unstable_elicitation")]
   mod elicitation;
   ...
  -#[cfg(feature = "unstable_elicitation")]
   pub use elicitation::*;
  ```

### 1.2 Release artifacts that contain the stabilized feature

| Artifact | Version | Date | Evidence |
| --- | --- | --- | --- |
| Repo tag / root `CHANGELOG.md` | `v1.7.0` | 2026-08-20 | "`*(schema)* stabilize elicitation (#1779)`" under *Added* — [CHANGELOG.md](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/CHANGELOG.md), [release v1.7.0](https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/v1.7.0) |
| JSON Schema artifact `schema/v1` | `schema-v1.21.0` | 2026-08-20 | same line in [schema/v1/CHANGELOG.md](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v1/CHANGELOG.md), [release schema-v1.21.0](https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/schema-v1.21.0) |
| JSON Schema artifact `schema/v2` | `schema-v2.0.0-alpha.3` | 2026-08-20 | [release](https://github.com/agentclientprotocol/agent-client-protocol/releases/tag/schema-v2.0.0-alpha.3) |
| Rust crate `agent-client-protocol-schema` | `1.7.0` | 2026-08-20 | crates.io feature list for 1.7.0 no longer contains `unstable_elicitation`; 1.6.0 (2026-07-21) still did — [crates.io](https://crates.io/crates/agent-client-protocol-schema) |
| Rust crate `agent-client-protocol` (SDK, separate repo `agentclientprotocol/rust-sdk`) | `2.1.0` | 2026-09-04 | feature list for 2.1.0 drops `unstable_elicitation` (present in 2.0.0, 2026-07-23); depends on `agent-client-protocol-schema =1.7.0`; release notes: "`*(acp)* update schema to 1.7 (#331)`" — [crates.io](https://crates.io/crates/agent-client-protocol), [rust-sdk release v2.1.0](https://github.com/agentclientprotocol/rust-sdk/releases/tag/v2.1.0) |
| TypeScript `@agentclientprotocol/sdk` | `1.4.0` | 2026-08-20 | "Stabilize elicitation APIs (#242)" — [typescript-sdk CHANGELOG.md](https://github.com/agentclientprotocol/typescript-sdk/blob/main/CHANGELOG.md), [PR #242](https://github.com/agentclientprotocol/typescript-sdk/pull/242) ("Updates schema to 1.21.0"), which renames `unstable_createElicitation` → `createElicitation` and `unstable_completeElicitation` → `completeElicitation` on `Client` and `AgentSideConnection` and removes the `**UNSTABLE**`/`@experimental` doc blocks. npm publish time 2026-08-20T23:00:39Z — [npm registry](https://registry.npmjs.org/@agentclientprotocol/sdk) |

The **ACP protocol version** negotiated in `initialize` is still the integer
`1`; the docs describe it as "a single integer that identifies a **MAJOR**
protocol version ... only incremented when breaking changes are introduced".
Elicitation is additive and capability-gated, so no version bump accompanied
it.
[Initialization → Protocol version](https://agentclientprotocol.com/protocol/v1/initialization#protocol-version)

### 1.3 Pre-stabilization history (all `(unstable)` entries in the root CHANGELOG)

- `0.11.3` (2026-03-18): "initial implementation: elicitation (#769)";
  "More robust schema for elicitation types (#771)".
- `0.11.5` (2026-04-09): "elicitation for session, tool call, and requests (#792)".
- `0.11.6` (2026-04-14): "Move elicitation scope into mode variants (#966)".
- `1.3.0` (2026-07-06): "remove URL elicitation error (#1574)".
- `1.4.0` (2026-07-06): "Add descriptions to elicitation enum options (#1397)".
- `1.7.0` (2026-08-20): "stabilize elicitation (#1779)".

[CHANGELOG.md](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/CHANGELOG.md)

TypeScript SDK: "**unstable:** Initial unstable elicitation support (#113)"
appears earlier in its changelog; stabilized in 1.4.0 (#242).
[typescript-sdk CHANGELOG.md](https://github.com/agentclientprotocol/typescript-sdk/blob/main/CHANGELOG.md)

### 1.4 Relationship to MCP

ACP's protocol page says elicitation "is based on the locked MCP 2026-07-28
release-candidate elicitation specification" and lists deliberate differences.
[Elicitation (v1)](https://agentclientprotocol.com/protocol/v1/elicitation)
As of today the MCP site lists **2026-07-28 as the current version**.
[MCP Versioning](https://modelcontextprotocol.io/specification/versioning)

Both `protocol/v1/elicitation` and `protocol/v2/elicitation` are present and
textually identical apart from the capability path (`clientCapabilities` in v1
vs `capabilities` in v2) and anchor links. The v2 stable `schema.json` carries
the same elicitation `$defs`.
[Elicitation (v2)](https://agentclientprotocol.com/protocol/v2/elicitation) ·
[schema/v2/schema.json](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v2/schema.json)

---

## 2. Exact wire shape

All quotations below are from the stable
[`schema/v1/schema.json`](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v1/schema.json)
(`$defs`) unless noted. Human-readable reference:
[Schema → elicitation/create](https://agentclientprotocol.com/protocol/v1/schema#elicitation%2Fcreate).

### 2.1 Method names

Client-side methods (Agent → Client), from `schema/v1/meta.json`:

```json
"clientMethods": {
  ...
  "elicitation_create": "elicitation/create",
  "elicitation_complete": "elicitation/complete"
}
```

- `elicitation/create` — JSON-RPC **request**; `x-side: "client"`.
- `elicitation/complete` — JSON-RPC **notification** (URL mode only);
  `x-side: "client"`.

[schema/v1/meta.json](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v1/meta.json)

Note the difference from MCP: MCP's method is also `elicitation/create`, but
in MCP 2026-07-28 it is carried *inside* an `InputRequiredResult` (MRTR
pattern), and MCP's completion notification was
`notifications/elicitation/complete` (2025-11-25 only; removed in 2026-07-28).
ACP sends a direct request and keeps its own `elicitation/complete`.
[RFD → Alignment with MCP](https://agentclientprotocol.com/rfds/elicitation#alignment-with-mcp) ·
[MCP 2026-07-28 changelog, Minor changes #11](https://modelcontextprotocol.io/specification/2026-07-28/changelog)

### 2.2 Client capability advertisement

In `initialize` params, `clientCapabilities.elicitation`:

```json
{
  "clientCapabilities": {
    "elicitation": {
      "form": {},
      "url": {}
    }
  }
}
```

Schema:

```json
"ElicitationCapabilities": {
  "description": "Elicitation capabilities supported by the client.",
  "type": "object",
  "properties": {
    "form": {
      "description": "Whether the client supports form-based elicitation.\n\nOptional. Omitted and `null` are equivalent and mean form support is not advertised.\nSupplying `{}` explicitly advertises form support.",
      "anyOf": [ { "$ref": "#/$defs/ElicitationFormCapabilities" }, { "type": "null" } ],
      "x-deserialize-default-on-error": true
    },
    "url": {
      "description": "Whether the client supports URL-based elicitation.\n\nOptional. Omitted or `null` both mean the client does not advertise support.\nSupplying `{}` means the client supports URL-based elicitation.",
      "anyOf": [ { "$ref": "#/$defs/ElicitationUrlCapabilities" }, { "type": "null" } ],
      "x-deserialize-default-on-error": true
    },
    "_meta": { ... }
  }
}
```

`ElicitationFormCapabilities` and `ElicitationUrlCapabilities` are objects with
only an optional `_meta`. Semantics (normative, from the protocol page):

> - An omitted or `null` top-level `elicitation` field means elicitation is unsupported.
> - Form support exists only when `form` is explicitly present and non-null. URL support exists only when `url` is explicitly present and non-null.
> - A present capability object may advertise zero, one, or both modes. `{}` and `{"form":null,"url":null}` advertise no supported modes.
>
> This deliberately differs from the MCP 2026-07-28 release candidate, where an empty elicitation capability retains the original form-only meaning.

[Elicitation → Checking support](https://agentclientprotocol.com/protocol/v1/elicitation#checking-support) ·
[Initialization → Elicitation capability](https://agentclientprotocol.com/protocol/v1/initialization#client-capabilities)

### 2.3 `elicitation/create` request params

```json
"CreateElicitationRequest": {
  "type": "object",
  "properties": {
    "message": { "description": "A human-readable message describing what input is needed.", "type": "string" },
    "_meta": { ... }
  },
  "anyOf": [
    { "properties": { "mode": { "const": "form" } }, "required": ["mode"], "allOf": [ { "$ref": "#/$defs/ElicitationFormMode" } ] },
    { "properties": { "mode": { "const": "url" } },  "required": ["mode"], "allOf": [ { "$ref": "#/$defs/ElicitationUrlMode" } ] },
    { "title": "other", "properties": { "mode": { "type": "string" } }, "required": ["mode"], ... "additionalProperties"/unevaluatedProperties: true }
  ],
  "required": ["message"],
  "x-side": "client",
  "x-method": "elicitation/create"
}
```

`mode` is **required** in ACP ("ACP also requires `mode` explicitly; it does
not apply MCP's omitted-mode form default").
[Elicitation → Creating an elicitation](https://agentclientprotocol.com/protocol/v1/elicitation#creating-an-elicitation)

Form mode adds `requestedSchema` plus a flattened **scope**:

```json
"ElicitationFormMode": {
  "type": "object",
  "properties": {
    "requestedSchema": {
      "description": "A JSON Schema describing the form fields to present to the user.",
      "allOf": [ { "$ref": "#/$defs/ElicitationSchema" } ]
    }
  },
  "anyOf": [
    { "title": "Session", "allOf": [ { "$ref": "#/$defs/ElicitationSessionScope" } ] },
    { "title": "Request", "allOf": [ { "$ref": "#/$defs/ElicitationRequestScope" } ] }
  ],
  "required": ["requestedSchema"]
}
```

```json
"ElicitationSessionScope": {
  "properties": {
    "sessionId": { "allOf": [ { "$ref": "#/$defs/SessionId" } ] },
    "toolCallId": { "anyOf": [ { "$ref": "#/$defs/ToolCallId" }, { "type": "null" } ] }
  },
  "required": ["sessionId"]
},
"ElicitationRequestScope": {
  "properties": { "requestId": { "allOf": [ { "$ref": "#/$defs/RequestId" } ] } },
  "required": ["requestId"]
}
```

Canonical example (protocol page):

```json
{
  "jsonrpc": "2.0",
  "id": 43,
  "method": "elicitation/create",
  "params": {
    "sessionId": "sess_abc123",
    "mode": "form",
    "message": "How should I approach this refactoring?",
    "requestedSchema": {
      "type": "object",
      "properties": {
        "strategy": { "type": "string", "enum": ["conservative", "balanced", "aggressive"] }
      },
      "required": ["strategy"]
    }
  }
}
```

Rust type (stable, no `cfg` gate), from
[`agent-client-protocol-schema/src/v1/elicitation.rs`](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/agent-client-protocol-schema/src/v1/elicitation.rs):

```rust
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ElicitationMode {
    Form(ElicitationFormMode),
    Url(ElicitationUrlMode),
    #[serde(untagged)]
    Other(OtherElicitationMode),
}
```

### 2.4 `elicitation/create` response

```json
"CreateElicitationResponse": {
  "type": "object",
  "properties": { "_meta": { ... } },
  "anyOf": [
    { "description": "The user accepted and provided content.", "properties": { "action": { "const": "accept" } }, "required": ["action"], "allOf": [ { "$ref": "#/$defs/ElicitationAcceptAction" } ] },
    { "description": "The user declined the elicitation.",      "properties": { "action": { "const": "decline" } }, "required": ["action"] },
    { "description": "The elicitation was cancelled.",          "properties": { "action": { "const": "cancel" } },  "required": ["action"] },
    { "title": "other", "properties": { "action": { "type": "string" } }, "required": ["action"], "additionalProperties": true, "not": { ...accept|decline|cancel... } }
  ],
  "x-side": "client",
  "x-method": "elicitation/create"
}
```

```json
"ElicitationAcceptAction": {
  "type": "object",
  "properties": {
    "content": {
      "description": "The user-provided content, if any, as an object matching the requested schema.",
      "type": ["object", "null"],
      "additionalProperties": { "$ref": "#/$defs/ElicitationContentValue" }
    }
  }
}
```

```json
"ElicitationContentValue": {
  "anyOf": [
    { "title": "String",      "type": "string" },
    { "title": "Integer",     "type": "integer", "format": "int64" },
    { "title": "Number",      "type": "number",  "format": "double" },
    { "title": "Boolean",     "type": "boolean" },
    { "title": "StringArray", "type": "array", "items": { "type": "string" } }
  ]
}
```

TypeScript (generated, `@agentclientprotocol/sdk` `src/schema/types.gen.ts`):

```ts
export type ElicitationContentValue =
  string | number | number | boolean | Array<string>;

export type ElicitationAcceptAction = {
  content?: { [key: string]: ElicitationContentValue; } | null;
};
```

[types.gen.ts](https://github.com/agentclientprotocol/typescript-sdk/blob/main/src/schema/types.gen.ts)

Response content is therefore restricted to one level of primitive/string-array
values; the Rust SDK has tests `response_accept_rejects_non_object_content` and
`response_accept_rejects_nested_object_content` confirming nested objects are
rejected on deserialization.
[elicitation.rs tests](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/agent-client-protocol-schema/src/v1/elicitation.rs)

### 2.5 `elicitation/complete` notification (URL mode)

```json
"CompleteElicitationNotification": {
  "description": "Notification sent by the agent when a URL-based elicitation is complete.",
  "type": "object",
  "properties": {
    "elicitationId": { "allOf": [ { "$ref": "#/$defs/ElicitationId" } ] },
    "_meta": { ... }
  },
  "required": ["elicitationId"],
  "x-side": "client",
  "x-method": "elicitation/complete"
}
```

`ElicitationId` is `{ "type": "string" }`.

---

## 3. The permitted JSON Schema subset for `requestedSchema`

ACP does **not** simply defer to MCP; it pins its own closed set of types in
`schema.json` (a fixed, non-generic model) and documents the deltas from MCP:

> ACP extends MCP's restricted schema subset with `pattern`, schema-level
> `title` and `description`, descriptions on titled enum options, and `_meta`.
> ACP preserves unknown schema types and response actions for forward
> compatibility, and does not support MCP's deprecated `enumNames`.
>
> ACP omits MCP's optional `$schema` field because ACP fixes the exact
> restricted schema subset rather than allowing the sender to select or annotate
> a schema dialect.

[RFD → Alignment with MCP](https://agentclientprotocol.com/rfds/elicitation#alignment-with-mcp)

### 3.1 Root schema

```json
"ElicitationSchema": {
  "description": "Type-safe elicitation schema for requesting structured user input.\n\nThis represents a JSON Schema object with primitive-typed properties,\nas required by the elicitation specification.",
  "type": "object",
  "properties": {
    "type": {
      "description": "Type discriminator. Always `\"object\"`.",
      "x-deserialize-default-on-error": true,
      "default": "object",
      "allOf": [ { "$ref": "#/$defs/ElicitationSchemaType" } ]
    },
    "title":       { "type": ["string", "null"], "x-deserialize-default-on-error": true },
    "properties": {
      "description": "Property definitions (must be primitive types).",
      "type": "object",
      "default": {},
      "additionalProperties": { "$ref": "#/$defs/ElicitationPropertySchema" }
    },
    "required": {
      "description": "List of required property names.\n\nOptional. Omitted and `null` are equivalent and mean no property names are required.",
      "type": ["array", "null"],
      "items": { "type": "string" }
    },
    "description": { "type": ["string", "null"], "x-deserialize-default-on-error": true },
    "_meta":       { "type": ["object", "null"], "additionalProperties": true, ... }
  }
}
```

```json
"ElicitationSchemaType": {
  "oneOf": [ { "description": "Object schema type.", "type": "string", "const": "object" } ]
}
```

Sender and reader rules (RFD):

> Senders MUST include both `type: "object"` and `properties` in every
> `requestedSchema`. For compatibility, ACP readers tolerate an omitted, `null`,
> or malformed `type` by treating it as `"object"`, and tolerate omitted
> `properties` by treating it as an empty map; `null` is not valid for
> `properties`. This reader tolerance does not relax the sender requirements.

[RFD → Restricted JSON Schema](https://agentclientprotocol.com/rfds/elicitation#restricted-json-schema)

Root-level: **no** `additionalProperties`, `$schema`, `$ref`, `$defs`,
`allOf`/`anyOf`/`oneOf`, `if/then/else`, `dependentRequired`, `patternProperties`,
`minProperties`, etc. are defined. The Rust `ElicitationSchema` struct has no
`deny_unknown_fields`, so unknown root keywords are silently dropped by the
reference deserializer (test `request_tolerates_extra_fields`).
[elicitation.rs](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/agent-client-protocol-schema/src/v1/elicitation.rs)

### 3.2 Property schema discriminator

```json
"ElicitationPropertySchema": {
  "description": "Property schema for elicitation form fields.\n\nEach variant corresponds to a JSON Schema `\"type\"` value.\nSingle-select enums use the `String` variant with `enum` or `oneOf` set.\nMulti-select enums use the `Array` variant.",
  "anyOf": [
    { "properties": { "type": { "const": "string" } },  "required": ["type"], "allOf": [ { "$ref": "#/$defs/StringPropertySchema" } ] },
    { "properties": { "type": { "const": "number" } },  "required": ["type"], "allOf": [ { "$ref": "#/$defs/NumberPropertySchema" } ] },
    { "properties": { "type": { "const": "integer" } }, "required": ["type"], "allOf": [ { "$ref": "#/$defs/IntegerPropertySchema" } ] },
    { "properties": { "type": { "const": "boolean" } }, "required": ["type"], "allOf": [ { "$ref": "#/$defs/BooleanPropertySchema" } ] },
    { "properties": { "type": { "const": "array" } },   "required": ["type"], "allOf": [ { "$ref": "#/$defs/MultiSelectPropertySchema" } ] },
    { "title": "other",
      "description": "Custom or future elicitation property schema.\n\nValues beginning with `_` are reserved for implementation-specific\nextensions. Unknown values that do not begin with `_` are reserved for\nfuture ACP variants.\n\nClients that do not understand this property schema type should preserve\nthe raw schema when storing, replaying, proxying, or forwarding\nelicitation requests. They MUST NOT render it as a known input control.",
      "properties": { "type": { "type": "string" } }, "required": ["type"],
      "not": { "anyOf": [ ...string|number|integer|boolean|array... ] },
      "additionalProperties": true }
  ]
}
```

TypeScript equivalent:

```ts
export type ElicitationPropertySchema =
  | (StringPropertySchema & { type: "string" })
  | (NumberPropertySchema & { type: "number" })
  | (IntegerPropertySchema & { type: "integer" })
  | (BooleanPropertySchema & { type: "boolean" })
  | (MultiSelectPropertySchema & { type: "array" })
  | { type: string; [key: string]: unknown };
```

Consequences: `type` is **required** on every property and must be a single
string (no `type: ["string","null"]` unions); there is **no `object` property
type** and **no `null` type**; `array` exists only as the multi-select enum
container.

### 3.3 `string` (also single-select enum)

```json
"StringPropertySchema": {
  "description": "Schema for string properties in an elicitation form.\n\nWhen `enum` or `oneOf` is set, this represents a single-select enum\nwith `\"type\": \"string\"`.",
  "type": "object",
  "properties": {
    "title":       { "type": ["string", "null"], "x-deserialize-default-on-error": true },
    "description": { "type": ["string", "null"], "x-deserialize-default-on-error": true },
    "minLength":   { "type": ["integer", "null"], "format": "uint32", "minimum": 0 },
    "maxLength":   { "type": ["integer", "null"], "format": "uint32", "minimum": 0 },
    "pattern":     { "description": "Pattern the string must match.", "type": ["string", "null"] },
    "format":      { "anyOf": [ { "$ref": "#/$defs/StringFormat" }, { "type": "null" } ] },
    "default":     { "type": ["string", "null"], "x-deserialize-default-on-error": true },
    "enum": {
      "description": "Enum values for untitled single-select enums.",
      "type": ["array", "null"], "items": { "type": "string" }
    },
    "oneOf": {
      "description": "Titled enum options for titled single-select enums.",
      "type": ["array", "null"], "items": { "$ref": "#/$defs/EnumOption" }
    },
    "_meta": { ... }
  }
}
```

```json
"StringFormat": {
  "description": "String format types for string properties in elicitation schemas.",
  "oneOf": [
    { "description": "Email address format.",         "type": "string", "const": "email" },
    { "description": "URI format.",                   "type": "string", "const": "uri" },
    { "description": "Date format (YYYY-MM-DD).",     "type": "string", "const": "date" },
    { "description": "Date-time format (ISO 8601).",  "type": "string", "const": "date-time" }
  ]
}
```

```json
"EnumOption": {
  "description": "A titled enum option with a const value, human-readable title, and optional description.",
  "type": "object",
  "properties": {
    "const":       { "description": "The constant value for this option.", "type": "string" },
    "title":       { "description": "Human-readable title for this option.", "type": "string" },
    "description": { "type": ["string", "null"], "x-deserialize-default-on-error": true },
    "_meta":       { ... }
  },
  "required": ["const", "title"]
}
```

Rust field list (verbatim shape): `title`, `description`, `min_length: Option<u32>`,
`max_length: Option<u32>`, `pattern: Option<String>`, `format: Option<StringFormat>`,
`default: Option<String>`, `enum_values` (`#[serde(rename = "enum")]`),
`one_of` (`#[serde(rename = "oneOf")]`), `meta`.
[elicitation.rs `StringPropertySchema`](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/agent-client-protocol-schema/src/v1/elicitation.rs)

Normative notes on strings (RFD):

> Because `pattern` is supplied by an Agent, implementations that evaluate it
> MUST use a regex engine or execution limits that bound evaluation time and
> resource use. A malicious or pathological pattern MUST NOT be allowed to block
> Client UI or unboundedly consume resources.
>
> Known formats include `email`, `uri`, `date`, and `date-time`. Other string
> format values are annotations. Implementations MUST preserve unknown formats
> when storing, replaying, proxying, or forwarding schemas and MUST NOT reject a
> schema solely because its string format is unknown.

[RFD → Restricted JSON Schema → String Schema](https://agentclientprotocol.com/rfds/elicitation#restricted-json-schema)

Note a tension: the JSON Schema for `StringFormat` is a closed `oneOf` of four
constants (so a strict validator would reject `format: "uuid"`), while the RFD
says unknown formats MUST be preserved and MUST NOT cause rejection. The Rust
`format: Option<StringFormat>` field is not `DefaultOnError`, so an unknown
format would fail Rust deserialization. See Open questions.

### 3.4 `number` and `integer`

```json
"NumberPropertySchema": {
  "description": "Schema for number (floating-point) properties in an elicitation form.",
  "type": "object",
  "properties": {
    "title":       { "type": ["string", "null"], ... },
    "description": { "type": ["string", "null"], ... },
    "minimum":     { "description": "Minimum value (inclusive).", "type": ["number", "null"], "format": "double" },
    "maximum":     { "description": "Maximum value (inclusive).", "type": ["number", "null"], "format": "double" },
    "default":     { "type": ["number", "null"], "format": "double", "x-deserialize-default-on-error": true },
    "_meta": { ... }
  }
}
```

```json
"IntegerPropertySchema": {
  "description": "Schema for integer properties in an elicitation form.",
  "type": "object",
  "properties": {
    "title":       { "type": ["string", "null"], ... },
    "description": { "type": ["string", "null"], ... },
    "minimum":     { "description": "Minimum value (inclusive).", "type": ["integer", "null"], "format": "int64" },
    "maximum":     { "description": "Maximum value (inclusive).", "type": ["integer", "null"], "format": "int64" },
    "default":     { "type": ["integer", "null"], "format": "int64", "x-deserialize-default-on-error": true },
    "_meta": { ... }
  }
}
```

Unlike MCP (where `NumberSchema` covers both `"number" | "integer"` with
`number`-typed bounds), ACP splits them and requires **integer-valued**
`minimum`/`maximum`/`default` for `type: "integer"`. No `exclusiveMinimum`,
`exclusiveMaximum`, or `multipleOf`.

### 3.5 `boolean`

```json
"BooleanPropertySchema": {
  "description": "Schema for boolean properties in an elicitation form.",
  "type": "object",
  "properties": {
    "title":       { "type": ["string", "null"], ... },
    "description": { "type": ["string", "null"], ... },
    "default":     { "type": ["boolean", "null"], "x-deserialize-default-on-error": true },
    "_meta": { ... }
  }
}
```

### 3.6 `array` — multi-select enum only

```json
"MultiSelectPropertySchema": {
  "description": "Schema for multi-select (array) properties in an elicitation form.",
  "type": "object",
  "properties": {
    "title":       { "type": ["string", "null"], ... },
    "description": { "type": ["string", "null"], ... },
    "minItems":    { "description": "Minimum number of items to select.", "type": ["integer", "null"], "format": "uint64", "minimum": 0 },
    "maxItems":    { "description": "Maximum number of items to select.", "type": ["integer", "null"], "format": "uint64", "minimum": 0 },
    "items":       { "description": "The items definition describing allowed values.", "allOf": [ { "$ref": "#/$defs/MultiSelectItems" } ] },
    "default": {
      "description": "Default selected values.",
      "type": ["array", "null"], "items": { "type": "string" },
      "x-deserialize-default-on-error": true,
      "x-deserialize-skip-invalid-items": true
    },
    "_meta": { ... }
  },
  "required": ["items"]
}
```

```json
"MultiSelectItems": {
  "anyOf": [
    { "description": "Multi-select string items with plain string values.",
      "properties": { "type": { "const": "string" } }, "required": ["type"],
      "allOf": [ { "$ref": "#/$defs/StringMultiSelectItems" } ] },
    { "title": "other", "description": "Custom or future typed multi-select items.",
      "properties": { "type": { "type": "string" } }, "required": ["type"],
      "not": { "anyOf": [ { "properties": { "type": { "const": "string" } }, "required": ["type"] } ] },
      "additionalProperties": true },
    { "title": "titled", "description": "Titled multi-select items with human-readable labels.",
      "allOf": [ { "$ref": "#/$defs/TitledMultiSelectItems" } ] }
  ]
},
"StringMultiSelectItems": {
  "properties": { "enum": { "description": "Allowed enum values.", "type": "array", "items": { "type": "string" } }, "_meta": { ... } },
  "required": ["enum"]
},
"TitledMultiSelectItems": {
  "properties": { "anyOf": { "description": "Titled enum options.", "type": "array", "items": { "$ref": "#/$defs/EnumOption" } }, "_meta": { ... } },
  "required": ["anyOf"]
}
```

Rust:

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MultiSelectItems {
    String(StringMultiSelectItems),
    #[serde(untagged)]
    Other(OtherMultiSelectItems),
    #[serde(untagged)]
    Titled(TitledMultiSelectItems),
}
```

So `items` is either `{ "type": "string", "enum": [...] }` (untitled) or
`{ "anyOf": [ {const,title,description?}, ... ] }` (titled — note **no `type`**
on titled items, matching MCP). The Rust test
`titled_multi_select_items_reject_one_of` asserts that `items.oneOf` is
**not** accepted for multi-select (`anyOf` is required). `uniqueItems`,
`contains`, arrays of numbers/objects, and tuple `prefixItems` are all outside
the subset.

The RFD also states:

> Unknown multi-select `items.type` values are reserved for future ACP variants
> or implementation-specific extensions. Implementation-specific values MUST
> begin with `_`. Clients that do not understand a multi-select item type should
> preserve the raw `items` schema when storing, replaying, proxying, or
> forwarding elicitation requests, but MUST NOT render it as string multi-select
> items.

### 3.7 Explicitly out of scope

From the RFD's "**Not supported** (to simplify client implementation)":

> - Complex nested objects/arrays (beyond enum arrays)
> - Conditional validation

And the FAQ:

> MCP intentionally restricts elicitation schemas to flat objects with primitive
> properties to simplify client implementation and user experience. Complex
> nested structures, arrays of objects (beyond enum arrays), and advanced JSON
> Schema features are explicitly not supported. Future MCP changes do not
> automatically change ACP; adopting them requires a separate ACP protocol
> change.

[RFD → Restricted JSON Schema](https://agentclientprotocol.com/rfds/elicitation#restricted-json-schema) ·
[RFD → FAQ "Should elicitation support complex nested data structures?"](https://agentclientprotocol.com/rfds/elicitation#frequently-asked-questions)

Plus the explicit exclusions already quoted: `$schema` (omitted by design) and
`enumNames` (not supported). Nothing in the ACP schema mentions `$ref`,
`additionalProperties`, `allOf`, `const` at property level, `enum` on
non-string types, `examples`, `readOnly`, `deprecated`, `nullable`, or
`x-*` vendor keywords — they are simply not part of the model; the reference
Rust deserializer would drop them.

### 3.8 MCP's definition, for comparison

MCP introduced elicitation in **2025-06-18** ("Add support for elicitation,
enabling servers to request additional information from users during
interactions").
[MCP 2025-06-18 changelog](https://modelcontextprotocol.io/specification/2025-06-18/changelog)

2025-06-18 `schema.ts` — note **no** `default` on string/number and the
`enumNames` idiom:

```ts
export type PrimitiveSchemaDefinition =
  StringSchema | NumberSchema | BooleanSchema | EnumSchema;

export interface StringSchema {
  type: "string";
  title?: string;
  description?: string;
  minLength?: number;
  maxLength?: number;
  format?: "email" | "uri" | "date" | "date-time";
}

export interface NumberSchema {
  type: "number" | "integer";
  title?: string;
  description?: string;
  minimum?: number;
  maximum?: number;
}

export interface BooleanSchema {
  type: "boolean";
  title?: string;
  description?: string;
  default?: boolean;
}

export interface EnumSchema {
  type: "string";
  title?: string;
  description?: string;
  enum: string[];
  enumNames?: string[]; // Display names for enum values
}
```

[schema/2025-06-18/schema.ts](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/schema/2025-06-18/schema.ts)

**MCP 2025-11-25** is the version that changed enum handling and added defaults:

> 5. Update `ElicitResult` and `EnumSchema` to use a more standards-based
>    approach and support titled, untitled, single-select, and multi-select
>    enums ([SEP-1330]).
> 6. Added support for URL mode elicitation ([SEP-1036])
> ...
> 9. Add support for default values in all primitive types (string, number,
>    enum) for elicitation schemas ([SEP-1034]).

[MCP 2025-11-25 changelog](https://modelcontextprotocol.io/specification/2025-11-25/changelog)

SEP-1330 (Status: Final, created 2025-08-11) states the intent: "deprecating
the non-standard `enumNames` property in favor of JSON Schema-compliant
patterns, and introducing additional support for multi-select enum schemas";
`LegacyEnumSchema` is kept "until a protocol-wide deprecation strategy is
implemented".
[seps/1330](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/seps/1330-elicitation-enum-schema-improvements-and-standards.md)

2025-11-25 / 2026-07-28 `schema.ts` (identical for these types):

```ts
export type PrimitiveSchemaDefinition =
  StringSchema | NumberSchema | BooleanSchema | EnumSchema;

export interface StringSchema {
  type: "string"; title?: string; description?: string;
  minLength?: number; maxLength?: number;
  format?: "email" | "uri" | "date" | "date-time";
  default?: string;
}
export interface NumberSchema {
  type: "number" | "integer"; title?: string; description?: string;
  minimum?: number; maximum?: number; default?: number;
}
export interface BooleanSchema {
  type: "boolean"; title?: string; description?: string; default?: boolean;
}
export interface UntitledSingleSelectEnumSchema {
  type: "string"; title?: string; description?: string;
  enum: string[]; default?: string;
}
export interface TitledSingleSelectEnumSchema {
  type: "string"; title?: string; description?: string;
  oneOf: Array<{ const: string; title: string; }>;
  default?: string;
}
export interface UntitledMultiSelectEnumSchema {
  type: "array"; title?: string; description?: string;
  minItems?: number; maxItems?: number;
  items: { type: "string"; enum: string[]; };
  default?: string[];
}
export interface TitledMultiSelectEnumSchema {
  type: "array"; title?: string; description?: string;
  minItems?: number; maxItems?: number;
  items: { anyOf: Array<{ const: string; title: string; }>; };
  default?: string[];
}
/**
 * Use {@link TitledSingleSelectEnumSchema} instead.
 * This interface will be removed in a future version.
 */
export interface LegacyTitledEnumSchema {
  type: "string"; title?: string; description?: string;
  enum: string[];
  /** (Legacy) Display names for enum values. Non-standard according to JSON schema 2020-12. */
  enumNames?: string[];
  default?: string;
}
export type EnumSchema =
  SingleSelectEnumSchema | MultiSelectEnumSchema | LegacyTitledEnumSchema;
```

Request/response in MCP 2026-07-28:

```ts
export interface ElicitRequestFormParams {
  mode?: "form";
  message: string;
  requestedSchema: {
    $schema?: string;
    type: "object";
    properties: { [key: string]: PrimitiveSchemaDefinition; };
    required?: string[];
  };
}
export interface ElicitResult {
  action: "accept" | "decline" | "cancel";
  content?: { [key: string]: string | number | boolean | string[] };
}
```

[schema/2025-11-25/schema.ts](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/schema/2025-11-25/schema.ts) ·
[schema/2026-07-28/schema.ts](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/schema/2026-07-28/schema.ts)

MCP's `LegacyTitledEnumSchema` is still present in the 2026-07-28 schema with
a "will be removed in a future version" comment, but it does **not** carry a
`@deprecated` JSDoc tag and does **not** appear in the MCP deprecated-features
registry (which, as of this writing, lists no elicitation items). Its status is
therefore "legacy/soft-deprecated" in MCP, and outright unsupported in ACP.
[MCP 2026-07-28 deprecated registry](https://modelcontextprotocol.io/specification/2026-07-28/deprecated)

**ACP vs MCP delta table (form-mode `requestedSchema`)**

| Aspect | MCP 2026-07-28 | ACP v1 stable |
| --- | --- | --- |
| `$schema` on root | optional | not modeled (dropped) |
| root `title` / `description` | not modeled | optional |
| `_meta` on root, properties, enum options, items | no | optional everywhere |
| `pattern` on string | no | optional (with bounded-regex MUST) |
| `enumNames` | legacy, kept | not supported |
| `description` on `{const,title}` option | no | optional |
| `number`/`integer` | one `NumberSchema` with `number` bounds | separate; integer bounds are int64 |
| `minLength`/`maxLength` | `number` | uint32 |
| `minItems`/`maxItems` | `number` | uint64 |
| unknown property `type` | not modeled | "other" variant: preserve, do not render |
| `mode` omitted | defaults to form | required |
| `content` on accept | omitted for URL mode | optional; `null` ≡ omitted |

---

## 4. Semantics clients must implement

All quotations from the ACP protocol page or RFD unless noted.

### 4.1 User interaction (MUST/SHOULD)

> Clients **MUST** clearly identify the Agent requesting information, respect
> user privacy, and provide clear decline and cancel controls. For form mode,
> Clients **MUST** let users review and modify responses before sending them.
> For URL mode, Clients **MUST** display the target host and obtain consent
> before navigating to it. Clients **SHOULD** present the request's `message`
> so users understand what is requested and why.

[Elicitation → User interaction requirements](https://agentclientprotocol.com/protocol/v1/elicitation#user-interaction-requirements)

### 4.2 Validation and defaults

> Form schemas are flat objects whose properties use the supported primitive
> and enum schemas. Clients **SHOULD** validate submitted values before
> responding, and Agents **SHOULD** validate them again.
>
> Clients that support schema defaults **SHOULD** pre-populate form fields
> with their declared default values.

[Elicitation → Form mode](https://agentclientprotocol.com/protocol/v1/elicitation#form-mode)

RFD FAQ adds: "For v1, we recommend starting with JSON Schema validation only."
and, if an Agent needs more validation, it "can fail the turn with an error"
and the "Client can then re-prompt the user".
[RFD → FAQ "What about validating user input on the client side?"](https://agentclientprotocol.com/rfds/elicitation#frequently-asked-questions)

Regex safety (already quoted in §3.3): evaluators of `pattern` MUST bound
evaluation time/resources.

### 4.3 Response actions, partial content

> - `accept`: the user submitted or consented to the interaction.
> - `decline`: the user explicitly declined.
> - `cancel`: the user dismissed the interaction without choosing.
>
> The `content` field is optional on `accept`; receivers treat omission and
> `null` equivalently. It is only meaningful for `accept`; receivers ignore it
> for `decline` and `cancel`. For an accepted form elicitation, `content`
> **SHOULD** conform to the requested schema. For an accepted URL elicitation,
> clients normally omit `content` because the interaction happens out of band.

[Elicitation → Responses](https://agentclientprotocol.com/protocol/v1/elicitation#responses)

Partial content is therefore not forbidden by a MUST — conformance to
`requestedSchema` (including its `required` list) is a SHOULD. The RFD's FAQ
table clarifies "schema's `required` array determines mandatory fields" and
that defaults are optional for elicitation (unlike session config options).
[RFD → FAQ "How does this differ from session config options?"](https://agentclientprotocol.com/rfds/elicitation#frequently-asked-questions)

Agent-side (not the renderer's job, but relevant to fallback UX):

> Agents **MUST NOT** assume an elicitation succeeds. They **MUST** handle
> `decline`, `cancel`, and failures by safely falling back, retrying, or
> failing the originating operation as appropriate.

RFD FAQ on `cancel`: "If the user dismisses the elicitation without making an
explicit choice (closes the dialog, presses Escape, etc.), the client responds
with `action: "cancel"`."

### 4.4 Unknown values / forward compatibility

> Unknown property schema `type` values are reserved for future ACP variants or
> implementation-specific extensions. Implementation-specific values MUST begin
> with `_`. Clients that do not understand a property schema type should
> preserve the raw schema when storing, replaying, proxying, or forwarding
> elicitation requests, but MUST NOT render it as a known input control.

Same pattern for unknown `mode` (MUST NOT render as `form`/`url`), unknown
multi-select `items.type`, and unknown response `action`. Unknown string
`format` values MUST be preserved and MUST NOT cause rejection (§3.3).
[RFD → Restricted JSON Schema](https://agentclientprotocol.com/rfds/elicitation#restricted-json-schema) ·
[RFD → Elicitation Modes](https://agentclientprotocol.com/rfds/elicitation#elicitation-modes)

Reader tolerance encoded in `schema.json` via `x-deserialize-default-on-error`
on `type` (root), `title`, `description`, `default`, `_meta`, and capability
sub-objects; `x-deserialize-skip-invalid-items` on multi-select `default`.
Constraint fields (`minLength`, `maxLength`, `pattern`, `format`, `minimum`,
`maximum`, `minItems`, `maxItems`, `enum`, `oneOf`, `items`) are **not**
error-tolerant in the schema — a malformed value there is a deserialization
error in the reference implementation.

### 4.5 Errors

> Requests using a mode the Client has not advertised produce JSON-RPC
> `-32602` (Invalid params).

> Agents **MUST NOT** request a mode the Client has not advertised. Agents
> **SHOULD** provide a graceful fallback when the required mode is unavailable.

[Elicitation → URL completion / Checking support](https://agentclientprotocol.com/protocol/v1/elicitation)

### 4.6 Security (form mode)

> Form mode **MUST NOT** be used to request secrets or credentials that grant
> access or authorize transactions, such as passwords, API keys, access or
> refresh tokens, private keys, recovery codes, or payment credentials.
> Ordinary profile information such as a name, email address, or username is
> not categorically prohibited. Agents **MUST** use URL mode for sensitive
> interactions. If the Client does not support URL mode, the Agent **MUST NOT**
> fall back to form mode; it must use another safe flow or fail the operation.

> Agents **SHOULD NOT** include URLs intended to be clickable in any field of a
> form-mode request.
> Clients **SHOULD NOT** render a URL as clickable in any elicitation field
> except the `url` field of a URL-mode request ...

Renderer implication: do not auto-linkify `message`, `title`, `description`, or
option text in form mode. Also note the RFD removed a `password` input type
during MCP alignment ("Removed `password` type (MCP prohibits sensitive data in
form mode)", revision 2026-02-05) — there is no masked-input affordance in the
subset.
[Elicitation → Form mode / URL security](https://agentclientprotocol.com/protocol/v1/elicitation) ·
[RFD → Revision history](https://agentclientprotocol.com/rfds/elicitation#revision-history)

Agent-side binding requirement (context for hosts):

> Agents **MUST** bind each elicitation and related state to the receiving
> Client connection and, when authentication exists, the verified user
> identity. A `sessionId` alone or an unverified Client-provided identity is
> not sufficient.

---

## 5. URL-mode elicitation (brief)

- Request: `mode: "url"`, `elicitationId` (string, opaque to Client, unique
  among outstanding URL elicitations on the connection), `url` (string,
  `format: uri`), `message`, plus the same flattened scope (`sessionId`
  [+`toolCallId`] or `requestId`). Schema `ElicitationUrlMode` requires
  `elicitationId` and `url`.
- `accept` means the user consented to open the URL, not that the interaction
  completed; `content` is normally omitted.
- Agent MAY send `elicitation/complete { elicitationId }` afterwards, only to
  the same Client; Clients MUST ignore unknown/already-completed IDs and SHOULD
  offer manual retry/cancel controls.
- Safe-URL rules adapted from MCP: Client MUST NOT prefetch, MUST show the full
  URL and obtain explicit consent, MUST open in a context the Client/LLM cannot
  inspect, SHOULD highlight the domain and warn on Punycode; Agent MUST NOT put
  credentials/PII/pre-authenticated access in the URL, SHOULD use HTTPS, MUST
  verify the completing user is the initiating user.
- Divergence from MCP 2026-07-28: MCP removed `elicitationId` and the
  completion notification in favour of MRTR retries; ACP keeps both because it
  has a persistent bidirectional connection.

[Elicitation → URL mode / URL completion / URL security](https://agentclientprotocol.com/protocol/v1/elicitation) ·
[MCP 2026-07-28 changelog #11](https://modelcontextprotocol.io/specification/2026-07-28/changelog)

---

## Schema-subset checklist

Everything a form renderer must handle to cover 100% of ACP v1 form-mode
`requestedSchema`. Source of truth: `schema/v1/schema.json` `$defs`
`ElicitationSchema`, `ElicitationPropertySchema`, `StringPropertySchema`,
`NumberPropertySchema`, `IntegerPropertySchema`, `BooleanPropertySchema`,
`MultiSelectPropertySchema`, `MultiSelectItems`, `StringMultiSelectItems`,
`TitledMultiSelectItems`, `EnumOption`, `StringFormat`.

**Root object**
- `type: "object"` — sender MUST send; reader treats omitted/`null`/malformed as `"object"`.
- `properties: { name: <property schema> }` — sender MUST send; reader treats omitted as `{}`; `null` invalid. **Field order is not guaranteed on the wire**: the reference Rust type is `pub properties: BTreeMap<String, ElicitationPropertySchema>` (sorted by key), so an Agent using the Rust SDK emits properties alphabetically regardless of authoring order, and a Rust Client re-sorts whatever it receives. No spec text defines display order; a renderer should preserve received JSON order but must not assume it is meaningful.
- `required: [string]` — optional; determines mandatory fields; `null` ≡ omitted.
- `title` (string|null) — optional form heading; ACP extension over MCP.
- `description` (string|null) — optional form help text; ACP extension.
- `_meta` (object|null) — opaque; must not be interpreted.
- Not present / must not be relied on: `$schema`, `$ref`, `$defs`, `additionalProperties`, `patternProperties`, `allOf`/`anyOf`/`oneOf`/`not`, `if`/`then`/`else`, `dependentRequired`, `min/maxProperties`.

**Every property schema**
- `type` — required, single string; dispatch on `string` | `number` | `integer` | `boolean` | `array`.
- Unknown `type` (incl. `_`-prefixed) — preserve raw schema; MUST NOT render as a known control (render as unsupported/placeholder).
- `title` (string|null) — label; falls back to property key.
- `description` (string|null) — help text.
- `_meta` — ignore.
- No `type` arrays/unions, no `null` type, no `object` type, no `const` at property level (only inside enum options), no `examples`, `readOnly`, `writeOnly`, `deprecated`.

**`string`** (free text when neither `enum` nor `oneOf` is present)
- `minLength` (uint32) / `maxLength` (uint32) — inclusive length bounds; validate before submit.
- `pattern` (ECMA-262 regex string) — validate with a bounded/linear-time engine or execution limit (MUST); never let a pathological pattern block the UI.
- `format`: `email` | `uri` | `date` (YYYY-MM-DD) | `date-time` (ISO 8601) — the closed set in `StringFormat`; RFD says these are the "known" formats and others are annotations to preserve, not reject. Use appropriate input control; validating them is a SHOULD-level "validate before responding".
- `default` (string) — pre-populate (SHOULD).
- Value on accept is a JSON string.

**Single-select enum (`type: "string"` + `enum` or `oneOf`)**
- `enum: [string]` — untitled options; display value as label.
- `oneOf: [{ const: string, title: string, description?: string, _meta? }]` — titled options; `const` and `title` required; `description` is an ACP extension.
- `default` (string) — must be one of the option values; pre-populate.
- Not supported: `enumNames` (ACP explicitly does not support MCP's legacy idiom — treat as unknown keyword / ignore), non-string `const`, `enum` on `number`/`integer`/`boolean`.
- Behaviour when both `enum` and `oneOf` are present is unspecified (see Open questions).
- Value on accept is a JSON string.

**`number`**
- `minimum` / `maximum` (double, inclusive).
- `default` (double).
- No `exclusiveMinimum`/`exclusiveMaximum`/`multipleOf`.
- Value on accept is a JSON number.

**`integer`**
- `minimum` / `maximum` (int64, inclusive) — must themselves be integers.
- `default` (int64).
- Value on accept is a JSON integer (`ElicitationContentValue::Integer(i64)`).

**`boolean`**
- `default` (boolean).
- Value on accept is a JSON boolean. Whether an unset, non-required boolean is omitted or sent as `false` is unspecified.

**Multi-select enum (`type: "array"`)**
- `items` — required; one of:
  - `{ "type": "string", "enum": [string] }` — untitled;
  - `{ "anyOf": [{ const, title, description?, _meta? }] }` — titled; **no `type`** key; `oneOf` here is rejected by the reference SDK;
  - `{ "type": "<unknown>" , ... }` — preserve; MUST NOT render as string multi-select.
- `minItems` / `maxItems` (uint64) — selection-count bounds; validate before submit.
- `default: [string]` — pre-select; reader skips non-string items.
- Not supported: `uniqueItems`, `contains`, `prefixItems`, `items` of `number`/`integer`/`boolean`/`object`, arrays of free-text strings (an `items` without `enum`/`anyOf` has no defined rendering).
- Value on accept is a JSON array of strings.

**Response construction**
- `{"action":"accept","content":{...}}` with values restricted to string | integer | number | boolean | string[]; omit fields the user left blank rather than sending `null` (no `null` value type exists in `ElicitationContentValue`).
- `{"action":"decline"}` for explicit refusal; `{"action":"cancel"}` for dismissal/Escape/close.
- `content` SHOULD conform to `requestedSchema` (types, `required`, constraints); validate client-side first (SHOULD).
- Never linkify text in form-mode fields (SHOULD NOT).

---

## Open questions / things I could not verify from primary sources

1. **`enum` + `oneOf` on the same string property.** The ACP schema allows both
   to be present simultaneously (`StringPropertySchema` has both fields, no
   mutual exclusion) and neither the protocol page nor the RFD says which wins
   or whether a validator should reject it. MCP's TypeScript union likewise
   doesn't forbid it structurally.
2. **Unknown `format` values.** The RFD says implementations MUST preserve
   unknown string formats and MUST NOT reject a schema for them, but the
   published `schema.json` models `format` as a closed `oneOf` of four
   constants and the Rust `format: Option<StringFormat>` field is not
   `DefaultOnError`, so a strict JSON Schema validator or the Rust SDK would
   reject `format: "uuid"`. I could not find an issue or test reconciling
   these. Treat it as "accept and preserve unknown formats; render as plain
   text" to satisfy the prose.
3. **Root-level `additionalProperties` / `$schema` / other keywords.** ACP
   never mentions `additionalProperties` or `$ref`. The reference Rust
   deserializer drops unknown keywords silently (no `deny_unknown_fields`;
   `request_tolerates_extra_fields` test). Whether a Client *may* reject a
   schema containing e.g. `$ref` or nested `type: "object"` is not stated;
   the "other" variant only speaks to unknown `type` strings. For nested
   objects the `type: "object"` property would fall into the "other" variant
   (since `object` is not one of the five known types) and thus MUST NOT be
   rendered as a known control.
4. **Omitted non-required fields on accept.** Whether a blank optional field
   should be omitted from `content` or sent with an empty string / `false`
   is unspecified; there is no `null` content value, so omission is the only
   representable "no answer".
5. **Date discrepancy.** The RFD revision history says "2026-07-22: Moved to
   Completed", while PR #1779 merged 2026-07-24 and the announcement/RFD
   Updates page say July 24, 2026. The published artifacts (schema 1.7.0 /
   schema-v1.21.0 / TS SDK 1.4.0) shipped 2026-08-20.
6. **"Locked MCP 2026-07-28 release candidate".** ACP's docs describe the MCP
   target as a release candidate at `/specification/draft/...`; MCP's
   versioning page now lists 2026-07-28 as *current*, and its `schema.ts`
   elicitation types are identical to `schema/draft/schema.ts` in the parts
   quoted here (the two files differ elsewhere). I did not diff every field
   outside elicitation.
7. **`docs.rs` for `agent-client-protocol` 2.1.0** was not fetched directly
   (crates.io metadata and the GitHub source were used instead); the claim
   that elicitation types are ungated in the 2.1.0 SDK rests on the crate's
   feature list (no `unstable_elicitation`) and its exact dependency on
   `agent-client-protocol-schema =1.7.0`, whose source has no such gate.
8. **Zed or other client implementations** of the stabilized subset were not
   examined; nothing here speaks to what real clients actually render.
9. **Property display order.** Neither the protocol page, the RFD, nor
   `schema.json` says anything about the order in which properties should be
   rendered. The Rust reference type stores `properties` in a `BTreeMap`
   (alphabetical), even though the crate enables `serde_json`'s
   `preserve_order` feature for `Value` maps, so ordering intent is lost when
   either side uses the Rust SDK. The TypeScript SDK uses plain objects (insertion
   order preserved). I found no issue tracking this.

---

## Fit against schemaform (as of `8bb3401`, 2026-09-07)

Nothing in this repository or its history mentions ACP, MCP, or elicitation.
The table maps every construct in the ACP form-mode subset to what schemaform
does with it today. "Editing" means a generated control with the expected
operations; locations are `file:line` in this repository.

| ACP construct | schemaform today | Evidence |
| --- | --- | --- |
| Root without `$schema` | `FormDefinition::compile` → `QualificationError::MissingDialect`; compiles only through `FormDefinition::compiler(..).default_dialect(Dialect::Draft202012)` | `crates/schemaform/src/resources.rs:164-170`, `:1345-1349`; `crates/schemaform/tests/qualification_facade.rs:24-92, 210-226` |
| Root `type: "object"` + `properties` + `required` | Editing (fixed object). Properties are emitted in **alphabetical** order (`BTreeSet`), not authored order | `crates/schemaform/src/engine.rs:389-393` |
| Root `title` / `description` | Root label falls back to `"Form"`; no test asserts the root label | `crates/schemaform/src/engine.rs:127` |
| `string` + `minLength`/`maxLength`/`pattern` | Editing; validated; `pattern` runs on the linear-time `regex` engine with 10 MiB / 2 MiB size caps (satisfies ACP's "MUST bound evaluation"); ECMA-262-only syntax (backreferences, lookaround) fails qualification | `crates/schemaform/src/validation.rs:19-20, 605-613`; `crates/schemaform/tests/validator_configuration_target.rs:425-431, 465-475, 483-560` |
| `string` + `format` ∈ {email, uri, date, date-time} | Annotation only (`validate_formats: false`, no public knob); adapter always renders `type="text"` — never `email`/`url`/`date` | `crates/schemaform/src/validation.rs:32-37`; `crates/schemaform-dioxus/src/lib.rs:3182-3186`, `render.rs:2432-2439`; only `email` and an unknown format are tested |
| `string` + `enum: [..]` | Editing (choice control); labels are the raw values | `crates/schemaform/src/engine.rs:2582-2599`; `lib.rs:3266-3274`; `tests/generated_choice_facade.rs` |
| `string` + `oneOf: [{const, title}]` (titled single-select) | **Capability-blocking** `applicator.one-of`; property becomes `Unsupported`; strict compile fails. No titled-enum special case; `oneOf` branches are never inspected for `const` | `crates/schemaform/src/engine.rs:1451-1462, 1768-1794`; `tests/unsupported_one_of_facade.rs` |
| `number` / `integer` + `minimum`/`maximum` | Editing; exact arithmetic | `tests/generated_number_facade.rs`, `generated_integer_facade.rs` |
| `boolean` | Editing (checkbox / select) | `crates/schemaform-dioxus/src/lib.rs:3238-3282` |
| `default` on any property | Surfaced via `DataSchemaAnnotations::defaults()` and as `creation_seed()`; **never pre-populated** on `create_form`. ACP: clients SHOULD pre-populate | `crates/schemaform/src/engine.rs:1131, 1183-1200`; `lib.rs:2433-2439`; `tests/generated_annotation_facade.rs:89-157` |
| `array` + `items: {type: "string", enum}` + `minItems`/`maxItems`/`default` | Editing as a homogeneous array of single-select controls with append/remove/move affordances (bounds gate the affordances). Not a multi-select widget; duplicates allowed without `uniqueItems` | `crates/schemaform/src/engine.rs:712-935, 3334-3379`; `tests/scalar_array_facade.rs:630-733, 1406-1489` |
| `array` + `items: {anyOf: [{const, title}]}` (titled multi-select) | **Capability-blocking** `applicator.any-of` at `/properties/x/items/anyOf` | `crates/schemaform/src/engine.rs:721-726`; untested for this exact shape |
| Unknown property `type` (e.g. `"_vendor"`) | Whole schema rejected by Draft 2020-12 meta-validation (`InvalidSchema` at `/properties/x/type`), in both strict and lenient modes. ACP: preserve and don't render | `crates/schemaform/src/resources.rs:251-258`; `tests/qualification_facade.rs:94-131` |
| Optional non-required scalar left blank | Key omitted from the snapshot (matches ACP's `content`, which has no null value type) | `tests/generated_scalar_presence_facade.rs:8-34, 168-195` |
| Response `content` restricted to `string | integer | number | boolean | string[]` | Snapshot is arbitrary JSON; a host maps it. Trivial for this subset | — |

### Build-level hazard specific to Rust ACP clients

`agent-client-protocol-schema`'s `ElicitationPropertySchema` is
`#[serde(tag = "type")]` and `NumberPropertySchema` has `minimum`/`maximum`/
`default: Option<f64>`; `ElicitationContentValue` is `#[serde(untagged)]` with
`Number(f64)` (`agent-client-protocol-schema/src/v1/elicitation.rs:354-366,
966-988, 2070-2078`). These are exactly the buffered shapes the core README
names as broken under the build-wide `serde_json/arbitrary_precision`
(`crates/schemaform/README.md:204-220`). A Rust ACP client that links both
crates cannot decode an `elicitation/create` whose number property carries a
fractional or exponent-spelled `minimum`/`maximum`/`default` unless it
canonicalizes first. The generic shape is already pinned in
`crates/schemaform/tests/validator_configuration_target.rs:222-375`; nothing
pins it against the real ACP types.

### Where existing tests already cover the ACP shape

Flat string/integer/number/boolean/enum/const objects, bound findings, `default`
non-mutation, `format` inertness, `oneOf`-with-`type`-branches blocking, missing
`$schema`, and unknown `type` are all covered (`fixed_object_profile_target.rs`,
`generated_*_facade.rs`, `unsupported_one_of_facade.rs`,
`qualification_facade.rs`, `scalar_array_facade.rs`, `browser_csr.rs`). What no
test pins today: the titled-enum idioms (`oneOf`/`anyOf` of `{const, title}`),
`items` without `type`, a `_`-prefixed `type`, a `$schema`-less schema
exercising the full subset through `default_dialect`, multi-select with
`minItems`/`maxItems`/`default` together, `format` uri/date/date-time, root
`title` as the form label, and ACP wire types under `arbitrary_precision`.

### Fixture-corpus fit

The business-schema corpus is a poor home for ACP samples: fixtures must carry
`$schema` (`crates/schemaform/tests/business_schema_corpus.rs:119-122`) and the
product compiler sets no `default_dialect`
(`testing/fixtures/business-schemas/product_cases.rs:140-145`);
`profile_case_for_occurrence` (`business_schema_corpus.rs:1497-1554`) has no
mapping for `oneOf`, `minLength`, `maxLength`, `maxItems` and panics on `const`;
the fixture count is hard-coded to 20. A dedicated `*_target.rs` file is the
cheaper vehicle.

---

## Generic vs ACP-specific: what would have to change where

Constraint: schemaform should support ACP elicitation only through generic
JSON Schema behaviour; ACP-specific handling must live outside the core.

**Generic JSON Schema — the one core change actually required**

- Recognise `oneOf` / `anyOf` whose every branch is a const-only schema
  (`{const, title?, description?, …annotations}`) as a finite, labelled choice
  set instead of a blocking applicator. This is the ecosystem-standard way to
  label enum options and predates MCP/ACP: JSON Forms documents
  `oneOf: [{const, title}]` for single select and `type: array` +
  `uniqueItems` + `items.oneOf` for multi select
  ([JSON Forms → Multiple Choice](https://jsonforms.io/docs/multiple-choice));
  RJSF documents both `oneOf`/`const` and `anyOf`/`enum`-singleton forms
  ([RJSF → Single fields → Custom labels for enum](https://rjsf-team.github.io/react-jsonschema-form/docs/json-schema/single/#custom-labels-for-enum-fields)).
  Seam: `scalar_choices` (`crates/schemaform/src/engine.rs:2582-2712`) already
  intersects `enum`/`const`/`type` sources; const-only branches are a third
  source. `array_applicator_findings` (`engine.rs:1768-1794`) must skip the
  recognised keyword so `applicator.one-of` / `applicator.any-of` keep firing
  for everything else. `ScalarChoices` → `ChoiceOption` gains a per-value
  title/description (`crates/schemaform/src/lib.rs:2593-2613, 3266-3274`).
  The array item-template path calls the same function (`engine.rs:712-720`),
  so titled multi-select items follow for free. New profile ids
  (e.g. `applicator.one-of.const-choices`, `applicator.any-of.const-choices`,
  target `editing`) alongside the existing blocking ones.

**Generic and already present — host wiring only**

- `$schema`-less roots: `FormDefinition::compiler(..).default_dialect(Dialect::Draft202012)`.
- `pattern` bounded by the linear-time `regex` engine and size caps.
- `minLength`/`maxLength`/`minimum`/`maximum`/`minItems`/`maxItems`/`required`, root `title`/`description`, blank optional → key omitted, `_meta` and other unknown keywords ignored by the 2020-12 meta-schema.
- Gated submission = "validate before responding"; advisory submission if the host prefers agent-side validation.

**Generic presentation — optional, adapter-level, not needed for correctness**

- Multi-select widget for a homogeneous array of finite choices (JSON Forms keys it on `uniqueItems: true`); ACP hosts can add `uniqueItems: true` in pre-processing or accept the list rendering.
- `format` → HTML `type="email|url|date|datetime-local"`; validation stays annotation-only, which ACP also treats as annotations.

**ACP-specific — keep out of core; belongs in the host (or a separate glue crate)**

- Unknown `_`-prefixed property `type`: strip before compile, render a host placeholder, preserve the raw schema for proxying. Invalid JSON Schema; the meta-validation rejection is correct.
- `default` pre-population: schemaform's `metadata.default` ("without implicit materialization") is a deliberate design decision; the host seeds initial form data from `DefinitionNodeView::creation_seed()` before `create_form`.
- Response mapping: snapshot → `{action: "accept", content}`; decline/cancel are host UI.
- `arbitrary_precision` tripwire against `agent-client-protocol-schema` types: per `crates/schemaform/README.md:222-239` this belongs on the host's wire layer, not as a schemaform dev-dependency. At most, name ACP in the README as a known affected downstream.

Spec for the core change: [sagikazarmark/schemaform#35 — Constant choices](https://github.com/sagikazarmark/schemaform/issues/35).
