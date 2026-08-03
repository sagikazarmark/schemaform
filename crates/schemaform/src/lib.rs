//! A synchronous, framework-independent form engine for form shapes discovered
//! at runtime from application-trusted JSON Schema Draft 2020-12 data schemas.
//!
//! The crate compiles reusable [`FormDefinition`] values, owns canonical JSON
//! form data and edit state, validates accepted changes, and prepares immutable
//! [`SubmissionSnapshot`] values. Schema retrieval and submission transport
//! remain the application's responsibility.
#![deny(rustdoc::broken_intra_doc_links)]
#![forbid(unsafe_code)]

#[allow(dead_code)]
mod engine;
pub mod finding;
pub mod json;
mod limits;
mod resources;
pub mod ui;
mod validation;

pub mod qualification {
    use std::{error::Error, fmt};

    use crate::{JsonPointer, RetrievalUri};

    /// Identifies which input supplied a resource being schema-qualified.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum QualificationResource {
        /// The root data-schema document.
        Root,
        /// A caller-supplied resource, indexed in caller order.
        Caller(usize),
        /// A schema resource bundled with the crate.
        BuiltIn,
    }

    /// The provenance and in-document location of a qualification failure.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct QualificationLocation {
        resource: QualificationResource,
        retrieval_uri: RetrievalUri,
        pointer: JsonPointer,
    }

    impl QualificationLocation {
        pub(crate) fn new(
            resource: QualificationResource,
            retrieval_uri: RetrievalUri,
            pointer: JsonPointer,
        ) -> Self {
            Self {
                resource,
                retrieval_uri,
                pointer,
            }
        }

        /// Returns which input supplied the containing resource.
        pub fn resource(&self) -> QualificationResource {
            self.resource
        }

        /// Returns the containing resource's absolute retrieval URI.
        pub fn retrieval_uri(&self) -> &RetrievalUri {
            &self.retrieval_uri
        }

        /// Returns the location within the containing resource.
        pub fn pointer(&self) -> &JsonPointer {
            &self.pointer
        }
    }

    /// A schema-qualification failure with enough location data to diagnose the
    /// offending resource or reference.
    ///
    /// Qualification is strict: malformed schemas, unsupported required
    /// vocabularies or dialect switches, duplicate identities, and unresolved
    /// references prevent both compilation and analysis.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum QualificationError {
        InvalidSchema {
            location: QualificationLocation,
        },
        MissingDialect {
            location: QualificationLocation,
        },
        UnsupportedDialect {
            location: QualificationLocation,
            dialect: String,
        },
        NestedDialectSwitch {
            location: QualificationLocation,
            dialect: String,
        },
        UnsupportedRequiredVocabulary {
            location: QualificationLocation,
            vocabulary: String,
        },
        InvalidCanonicalIdentity {
            location: QualificationLocation,
            identity: String,
        },
        DuplicateRetrievalIdentity {
            identity: String,
            first_location: Box<QualificationLocation>,
            second_location: Box<QualificationLocation>,
        },
        DuplicateCanonicalIdentity {
            identity: String,
            first_location: Box<QualificationLocation>,
            second_location: Box<QualificationLocation>,
        },
        DuplicateAnchorIdentity {
            resource_uri: String,
            anchor: String,
            first_location: Box<QualificationLocation>,
            second_location: Box<QualificationLocation>,
        },
        UnresolvedReference {
            location: QualificationLocation,
            reference: String,
        },
    }

    impl fmt::Display for QualificationError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidSchema { location } => write!(
                    formatter,
                    "data schema at {}#{} is invalid against the Draft 2020-12 meta-schema",
                    location.retrieval_uri(),
                    location.pointer()
                ),
                Self::MissingDialect { location } => write!(
                    formatter,
                    "data-schema resource at {}#{} has no dialect",
                    location.retrieval_uri(),
                    location.pointer()
                ),
                Self::UnsupportedDialect { location, dialect } => write!(
                    formatter,
                    "data-schema resource at {}#{} declares unsupported dialect {dialect}",
                    location.retrieval_uri(),
                    location.pointer()
                ),
                Self::NestedDialectSwitch { location, dialect } => write!(
                    formatter,
                    "data schema at {}#{} switches to unsupported nested dialect {dialect}",
                    location.retrieval_uri(),
                    location.pointer()
                ),
                Self::UnsupportedRequiredVocabulary {
                    location,
                    vocabulary,
                } => write!(
                    formatter,
                    "data-schema meta-schema at {}#{} requires unsupported vocabulary {vocabulary}",
                    location.retrieval_uri(),
                    location.pointer()
                ),
                Self::InvalidCanonicalIdentity { location, identity } => write!(
                    formatter,
                    "data-schema resource at {}#{} has invalid canonical identity {identity}",
                    location.retrieval_uri(),
                    location.pointer()
                ),
                Self::DuplicateRetrievalIdentity {
                    identity,
                    first_location,
                    second_location,
                } => write!(
                    formatter,
                    "data-schema resources at {}#{} and {}#{} repeat retrieval identity {identity}",
                    first_location.retrieval_uri(),
                    first_location.pointer(),
                    second_location.retrieval_uri(),
                    second_location.pointer()
                ),
                Self::DuplicateCanonicalIdentity {
                    identity,
                    first_location,
                    second_location,
                } => write!(
                    formatter,
                    "data-schema resources at {}#{} and {}#{} repeat canonical identity {identity}",
                    first_location.retrieval_uri(),
                    first_location.pointer(),
                    second_location.retrieval_uri(),
                    second_location.pointer()
                ),
                Self::DuplicateAnchorIdentity {
                    resource_uri,
                    anchor,
                    first_location,
                    second_location,
                } => write!(
                    formatter,
                    "data-schema locations at {}#{} and {}#{} repeat anchor {anchor} in schema resource {resource_uri}",
                    first_location.retrieval_uri(),
                    first_location.pointer(),
                    second_location.retrieval_uri(),
                    second_location.pointer()
                ),
                Self::UnresolvedReference {
                    location,
                    reference,
                } => write!(
                    formatter,
                    "data-schema reference at {}#{} has unresolved target {reference}",
                    location.retrieval_uri(),
                    location.pointer()
                ),
            }
        }
    }

    impl Error for QualificationError {}
}

pub mod address {
    use std::{error::Error, fmt};

    use jsonptr::PointerBuf;
    use serde::{Deserialize, Deserializer, Serialize, de};

    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
    #[serde(transparent)]
    pub struct JsonPointer(String);

    impl JsonPointer {
        pub fn parse(value: impl Into<String>) -> Result<Self, AddressError> {
            let value = value.into();
            PointerBuf::parse(value.clone()).map_err(|_| AddressError::InvalidJsonPointer)?;
            Ok(Self(value))
        }

        pub fn as_str(&self) -> &str {
            &self.0
        }

        pub(crate) fn is_strict_descendant_of(&self, ancestor: &Self) -> bool {
            let pointer = jsonptr::Pointer::parse(&self.0)
                .expect("public JSON Pointers are validated during construction");
            let ancestor = jsonptr::Pointer::parse(&ancestor.0)
                .expect("public JSON Pointers are validated during construction");
            pointer != ancestor && pointer.starts_with(ancestor)
        }

        pub(crate) fn intersects(&self, other: &Self) -> bool {
            let pointer = jsonptr::Pointer::parse(&self.0)
                .expect("public JSON Pointers are validated during construction");
            let other = jsonptr::Pointer::parse(&other.0)
                .expect("public JSON Pointers are validated during construction");
            pointer.starts_with(other) || other.starts_with(pointer)
        }
    }

    impl fmt::Debug for JsonPointer {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.debug_tuple("JsonPointer").field(&self.0).finish()
        }
    }

    impl fmt::Display for JsonPointer {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(&self.0)
        }
    }

    impl<'de> Deserialize<'de> for JsonPointer {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            Self::parse(value).map_err(de::Error::custom)
        }
    }

    macro_rules! string_address {
        ($name:ident) => {
            #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
            #[serde(transparent)]
            pub struct $name(String);

            impl $name {
                pub fn as_str(&self) -> &str {
                    &self.0
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str(&self.0)
                }
            }
        };
    }

    string_address!(RetrievalUri);
    string_address!(ExtensionNamespace);
    string_address!(WidgetSymbol);

    impl<'de> Deserialize<'de> for RetrievalUri {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            Self::parse(value).map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for WidgetSymbol {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            Self::parse(value).map_err(de::Error::custom)
        }
    }

    impl RetrievalUri {
        pub fn parse(value: impl Into<String>) -> Result<Self, AddressError> {
            let value = value.into();
            let parsed = referencing::Uri::parse(value.clone())
                .map_err(|_| AddressError::UriMustBeAbsolute)?;
            if parsed.fragment().is_some() {
                return Err(AddressError::RetrievalUriHasFragment);
            }
            Ok(Self(value))
        }
    }

    impl ExtensionNamespace {
        pub fn parse(value: impl Into<String>) -> Result<Self, AddressError> {
            let value = value.into();
            referencing::Uri::parse(value.clone()).map_err(|_| AddressError::UriMustBeAbsolute)?;
            Ok(Self(value))
        }
    }

    impl<'de> Deserialize<'de> for ExtensionNamespace {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            Self::parse(value).map_err(de::Error::custom)
        }
    }

    impl WidgetSymbol {
        pub fn parse(value: impl Into<String>) -> Result<Self, AddressError> {
            let value = value.into();
            if value.is_empty() {
                return Err(AddressError::Empty);
            }
            Ok(Self(value))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SchemaLocation {
        resource: RetrievalUri,
        pointer: JsonPointer,
    }

    impl SchemaLocation {
        pub fn new(resource: RetrievalUri, pointer: JsonPointer) -> Self {
            Self { resource, pointer }
        }

        pub fn resource(&self) -> &RetrievalUri {
            &self.resource
        }

        pub fn pointer(&self) -> &JsonPointer {
            &self.pointer
        }
    }

    /// Failure to construct a validated public address value.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum AddressError {
        Empty,
        InvalidJsonPointer,
        UriMustBeAbsolute,
        RetrievalUriHasFragment,
    }

    impl fmt::Display for AddressError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "invalid public address: {self:?}")
        }
    }

    impl Error for AddressError {}
}

pub mod definition {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        error::Error,
        fmt,
        sync::Arc,
    };

    use serde_json::Value;
    use sha2::{Digest, Sha256};

    pub use crate::limits::{
        CompilationLimitDimension, CompilationLimitError, CompilationLimitPhase,
    };
    use crate::{
        address::{ExtensionNamespace, JsonPointer, RetrievalUri, SchemaLocation, WidgetSymbol},
        engine,
        form::{Form, FormBuildError, FormBuilder},
        qualification::QualificationError,
        ui,
    };

    pub use crate::engine::DataSchemaAnnotations;

    /// A reusable immutable form definition compiled from trusted data-schema resources.
    ///
    /// Definitions contain the framework-neutral definition tree, validator,
    /// capability findings, and opaque semantic fingerprint. They perform no I/O.
    #[derive(Clone)]
    pub struct FormDefinition {
        pub(crate) inner: Arc<DefinitionInner>,
    }

    pub(crate) struct DefinitionInner {
        pub(crate) engine: engine::FormDefinition,
        pub(crate) validator: crate::validation::Validator,
        nodes: Vec<DefinitionNode>,
        required_extensions: Vec<ExtensionNamespace>,
        annotation_scopes: Vec<AnnotationScope>,
        capability_report: CapabilityReport,
        fingerprint: DefinitionFingerprint,
    }

    struct AnnotationScope {
        binding: Option<JsonPointer>,
        annotations: DataSchemaAnnotations,
    }

    pub(crate) struct DefinitionNode {
        pub(crate) id: DefinitionNodeId,
        authored_id: Option<String>,
        kind: DefinitionNodeKind,
        semantic_kind: Option<SemanticKind>,
        binding: Option<JsonPointer>,
        widget: Option<WidgetSymbol>,
        extensions: BTreeMap<ExtensionNamespace, Value>,
        label: String,
        label_reference: Option<ui::v1::TextReference>,
        label_visible: bool,
        help: Option<String>,
        help_reference: Option<ui::v1::TextReference>,
        item_label_reference: Option<ui::v1::TextReference>,
        text: Option<ui::v1::TextReference>,
        data_schema_annotations: DataSchemaAnnotations,
        creation_seed: Option<Value>,
        required: bool,
        accepts_null: bool,
        choice_options: Vec<ChoiceOption>,
        choice_selectable: bool,
        owning_array: Option<DefinitionNodeId>,
        grid_spans: Option<GridSpans>,
        schema_locations: Vec<SchemaLocation>,
        children: Vec<DefinitionNodeId>,
    }

    #[derive(Clone)]
    struct GeneratedNode {
        binding: String,
        parent_binding: Option<String>,
        kind: DefinitionNodeKind,
        semantic_kind: Option<SemanticKind>,
        label: String,
        help: Option<String>,
        data_schema_annotations: DataSchemaAnnotations,
        creation_seed: Option<Value>,
        required: bool,
        accepts_null: bool,
        choice_options: Vec<ChoiceOption>,
        choice_selectable: bool,
        owning_array_binding: Option<String>,
        schema_locations: Vec<SchemaLocation>,
    }

    impl FormDefinition {
        /// Strictly compiles one self-contained Draft 2020-12 data schema.
        ///
        /// Use [`Self::compiler`] for an authored UI schema, caller-supplied
        /// resources, an explicit default dialect, or custom limits.
        pub fn compile(data_schema: Value) -> Result<Self, CompileError> {
            Self::compile_at(data_schema, crate::validation::default_root_uri())
        }

        fn compile_at(data_schema: Value, root_uri: RetrievalUri) -> Result<Self, CompileError> {
            let definition = Self::analyze_at(
                data_schema,
                root_uri,
                Vec::new(),
                None,
                CompilationProfile::default(),
                None,
            )?;
            Self::require_no_blocking_capabilities(definition)
        }

        fn require_no_blocking_capabilities(definition: Self) -> Result<Self, CompileError> {
            if definition.inner.capability_report.is_blocking() {
                return Err(CompileError::Capability(
                    definition.inner.capability_report.clone(),
                ));
            }
            Ok(definition)
        }

        fn analyze_at(
            data_schema: Value,
            root_uri: RetrievalUri,
            resources: Vec<SchemaResource>,
            ui_schema: Option<ui::v1::UiSchema>,
            profile: CompilationProfile,
            default_dialect: Option<Dialect>,
        ) -> Result<Self, CompileError> {
            crate::limits::check_compilation_inputs(&root_uri, &data_schema, &resources, &profile)
                .map_err(|error| CompileError::Resource(ResourceError::Limit(error)))?;
            let graph = crate::resources::ResourceGraph::prepare_with_default_dialect(
                root_uri.clone(),
                data_schema,
                resources
                    .into_iter()
                    .map(|resource| (resource.uri, resource.document))
                    .collect(),
                default_dialect.map(|_| "https://json-schema.org/draft/2020-12/schema"),
            )
            .map_err(|error| match error {
                crate::resources::PrepareError::Qualification(error) => {
                    CompileError::Qualification(error)
                }
                crate::resources::PrepareError::InvalidGraph => {
                    CompileError::Resource(ResourceError::InvalidResourceGraph)
                }
            })?;
            crate::limits::check_qualified_graph(graph.limit_metrics(), &profile)
                .map_err(|error| CompileError::Resource(ResourceError::Limit(error)))?;
            let canonical_root_uri = RetrievalUri::parse(graph.root_resource())
                .expect("prepared resource identities are absolute and fragment-free");
            let engine = engine::FormDefinition::compile_graph(
                &graph,
                profile.data_schema_limits().traversal,
            )
            .map_err(|error| CompileError::engine(error, &canonical_root_uri))?;
            let mut generated_nodes = engine
                .objects()
                .map(|object| GeneratedNode {
                    binding: object.binding().to_owned(),
                    parent_binding: object.parent_binding().map(str::to_owned),
                    kind: DefinitionNodeKind::AutoGeneratedLayout,
                    semantic_kind: Some(SemanticKind::FixedObject),
                    label: object.label().to_owned(),
                    help: object.help().map(str::to_owned),
                    data_schema_annotations: object.data_schema_annotations().clone(),
                    creation_seed: Some(object.creation_seed().clone()),
                    required: object.is_required(),
                    accepts_null: false,
                    choice_options: Vec::new(),
                    choice_selectable: false,
                    owning_array_binding: None,
                    schema_locations: collect_schema_locations(object.schema_locations()),
                })
                .collect::<Vec<_>>();
            generated_nodes.extend(
                engine
                    .controls()
                    .map(|control| {
                        let semantic_kind = control_semantic_kind(control);
                        GeneratedNode {
                            binding: control.binding().to_owned(),
                            parent_binding: control.parent_binding().map(str::to_owned),
                            kind: DefinitionNodeKind::Control,
                            semantic_kind: Some(semantic_kind),
                            label: control.label().to_owned(),
                            help: control.help().map(str::to_owned),
                            data_schema_annotations: control.data_schema_annotations().clone(),
                            creation_seed: control.creation_seed().cloned(),
                            required: control.is_required(),
                            accepts_null: control.accepts_null(),
                            choice_options: control
                                .choices()
                                .cloned()
                                .map(|value| ChoiceOption {
                                    label: scalar_choice_label(&value),
                                    value,
                                })
                                .collect(),
                            choice_selectable: control.is_choice(),
                            owning_array_binding: None,
                            schema_locations: collect_schema_locations(control.schema_locations()),
                        }
                    })
                    .collect::<Vec<_>>(),
            );
            for array in engine.arrays() {
                let array_binding = array.binding().to_owned();
                generated_nodes.push(GeneratedNode {
                    binding: array_binding.clone(),
                    parent_binding: array.parent_binding().map(str::to_owned),
                    kind: DefinitionNodeKind::Control,
                    semantic_kind: Some(SemanticKind::HomogeneousArray),
                    label: array.label().to_owned(),
                    help: array.help().map(str::to_owned),
                    data_schema_annotations: array.data_schema_annotations().clone(),
                    creation_seed: Some(array.creation_seed().clone()),
                    required: array.is_required(),
                    accepts_null: false,
                    choice_options: Vec::new(),
                    choice_selectable: false,
                    owning_array_binding: None,
                    schema_locations: collect_schema_locations(array.schema_locations()),
                });
                generated_nodes.extend(array.item_objects().map(|object| GeneratedNode {
                    binding: object.binding().to_owned(),
                    parent_binding: object.parent_binding().map(str::to_owned),
                    kind: DefinitionNodeKind::AutoGeneratedLayout,
                    semantic_kind: Some(SemanticKind::FixedObject),
                    label: object.label().to_owned(),
                    help: object.help().map(str::to_owned),
                    data_schema_annotations: object.data_schema_annotations().clone(),
                    creation_seed: Some(object.creation_seed().clone()),
                    required: object.is_required(),
                    accepts_null: false,
                    choice_options: Vec::new(),
                    choice_selectable: false,
                    owning_array_binding: Some(array_binding.clone()),
                    schema_locations: collect_schema_locations(object.schema_locations()),
                }));
                generated_nodes.extend(array.item_controls().map(|item| {
                    GeneratedNode {
                        binding: item.binding().to_owned(),
                        parent_binding: item.parent_binding().map(str::to_owned),
                        kind: DefinitionNodeKind::Control,
                        semantic_kind: Some(control_semantic_kind(item)),
                        label: item.label().to_owned(),
                        help: item.help().map(str::to_owned),
                        data_schema_annotations: item.data_schema_annotations().clone(),
                        creation_seed: item.creation_seed().cloned(),
                        required: item.is_required(),
                        accepts_null: item.accepts_null(),
                        choice_options: item
                            .choices()
                            .cloned()
                            .map(|value| ChoiceOption {
                                label: scalar_choice_label(&value),
                                value,
                            })
                            .collect(),
                        choice_selectable: item.is_choice(),
                        owning_array_binding: Some(array_binding.clone()),
                        schema_locations: collect_schema_locations(item.schema_locations()),
                    }
                }));
            }
            let capability_report = CapabilityReport {
                findings: engine
                    .capability_findings()
                    .map(|finding| CapabilityFinding {
                        code: finding.code(),
                        instance_location: JsonPointer::parse(finding.binding())
                            .expect("compiled engine bindings are valid JSON Pointers"),
                        keyword_location: SchemaLocation::new(
                            RetrievalUri::parse(finding.resource())
                                .expect("compiled engine resources are absolute URIs"),
                            JsonPointer::parse(finding.keyword_location()).expect(
                                "compiled engine keyword locations are valid JSON Pointers",
                            ),
                        ),
                        parameters: finding.parameters().clone(),
                        severity: if finding.is_blocking() {
                            CapabilitySeverity::Blocking
                        } else {
                            CapabilitySeverity::Warning
                        },
                    })
                    .collect(),
            };
            generated_nodes.extend(engine.unsupported_regions().map(|region| GeneratedNode {
                binding: region.binding().to_owned(),
                parent_binding: region.parent_binding().map(str::to_owned),
                kind: DefinitionNodeKind::Unsupported,
                semantic_kind: None,
                label: region.label().to_owned(),
                help: region.help().map(str::to_owned),
                data_schema_annotations: region.data_schema_annotations().clone(),
                creation_seed: None,
                required: region.is_required(),
                accepts_null: false,
                choice_options: Vec::new(),
                choice_selectable: false,
                owning_array_binding: None,
                schema_locations: collect_schema_locations(region.schema_locations()),
            }));
            generated_nodes.sort_by(|left, right| {
                (&left.owning_array_binding, &left.binding)
                    .cmp(&(&right.owning_array_binding, &right.binding))
            });

            let annotation_scopes = std::iter::once(AnnotationScope {
                binding: None,
                annotations: engine.root_annotations().clone(),
            })
            .chain(
                generated_nodes
                    .iter()
                    .filter(|node| node.owning_array_binding.is_none())
                    .map(|node| AnnotationScope {
                        binding: Some(
                            JsonPointer::parse(node.binding.clone())
                                .expect("compiled engine bindings are valid JSON Pointers"),
                        ),
                        annotations: node.data_schema_annotations.clone(),
                    }),
            )
            .collect::<Vec<_>>();

            let (nodes, required_extensions) = if let Some(ui_schema) = ui_schema {
                let required_extensions = ui_schema
                    .required_extensions()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let nodes = compile_authored_nodes(
                    &ui_schema,
                    &generated_nodes,
                    engine.root_annotations(),
                    collect_schema_locations(engine.root_schema_locations()),
                    &profile,
                )?;
                (nodes, required_extensions)
            } else {
                let roots = root_auto_roots(
                    &generated_nodes,
                    direct_generated_properties(
                        &generated_nodes,
                        AuthoredBindingContext::root(),
                        "",
                    )
                    .into_values()
                    .collect(),
                );
                let mut nodes = vec![DefinitionNode {
                    id: DefinitionNodeId(0),
                    authored_id: None,
                    kind: DefinitionNodeKind::AutoGeneratedLayout,
                    semantic_kind: None,
                    binding: None,
                    widget: None,
                    extensions: BTreeMap::new(),
                    label: String::new(),
                    label_reference: None,
                    label_visible: true,
                    help: None,
                    help_reference: None,
                    item_label_reference: None,
                    text: None,
                    data_schema_annotations: engine.root_annotations().clone(),
                    creation_seed: None,
                    required: false,
                    accepts_null: false,
                    choice_options: Vec::new(),
                    choice_selectable: false,
                    owning_array: None,
                    grid_spans: None,
                    schema_locations: collect_schema_locations(engine.root_schema_locations()),
                    children: Vec::new(),
                }];
                nodes[0].children = materialize_generated_region(
                    &generated_nodes,
                    None,
                    &roots,
                    AuthoredBindingContext::root(),
                    "",
                    &mut HashSet::new(),
                    &mut nodes,
                )
                .expect("the complete generated projection has unique bindings");
                (nodes, Vec::new())
            };

            crate::limits::check_compilation_outputs(
                nodes.len(),
                nodes
                    .iter()
                    .filter(|node| node.kind == DefinitionNodeKind::Control)
                    .count(),
                capability_report.findings.len(),
                &profile,
            )
            .map_err(|error| CompileError::Resource(ResourceError::Limit(error)))?;

            let validator = crate::validation::Validator::compile(&graph)
                .map_err(CompileError::Qualification)?;

            let fingerprint = fingerprint(
                &nodes,
                &required_extensions,
                &capability_report,
                &engine,
                &root_uri,
                &profile,
            );
            Ok(Self {
                inner: Arc::new(DefinitionInner {
                    engine,
                    validator,
                    nodes,
                    required_extensions,
                    annotation_scopes,
                    capability_report,
                    fingerprint,
                }),
            })
        }

        /// Returns the opaque semantic fingerprint of this compiled definition.
        ///
        /// Equality means the compilation inputs that affect observable form
        /// behavior were equivalent for this crate version. The byte format is
        /// intentionally unavailable and must not be treated as a stable digest.
        pub fn fingerprint(&self) -> DefinitionFingerprint {
            self.inner.fingerprint
        }

        /// Starts a configurable compilation for `data_schema`.
        ///
        /// The returned builder uses release-candidate limits and an
        /// implementation-provided root URI until those settings are overridden.
        pub fn compiler(data_schema: Value) -> FormCompiler {
            FormCompiler::new(data_schema)
        }

        /// Returns the root of the immutable definition tree.
        ///
        /// The identifier is valid only with this definition and definitions
        /// compiled from semantically identical inputs.
        pub fn root(&self) -> DefinitionNodeId {
            DefinitionNodeId(0)
        }

        /// Looks up a definition node by its opaque identifier.
        ///
        /// Returns `None` when no node has this numeric identifier. Identifiers
        /// carry no definition provenance and must not be mixed across unrelated
        /// definitions. Item-template nodes are definition nodes; their runtime
        /// instances are exposed by [`crate::NodeView::children`].
        pub fn node(&self, id: DefinitionNodeId) -> Option<DefinitionNodeView<'_>> {
            self.inner
                .nodes
                .get(usize::try_from(id.0).ok()?)
                .filter(|node| node.id == id)
                .map(|node| DefinitionNodeView { node })
        }

        /// Creates independent owned form state over canonical JSON form data.
        ///
        /// Schema-invalid but structurally permitted data remains constructible
        /// so that a user can inspect and repair it.
        pub fn create_form(&self, form_data: Value) -> Result<Form, FormBuildError> {
            Form::new(
                self.clone(),
                form_data,
                crate::json::FormDataLimits::default(),
                crate::form::FindingVisibilityPolicy::default(),
                crate::form::ExternalFindingLimits::default(),
            )
        }

        /// Starts a form builder over owned canonical JSON data.
        ///
        /// Use this instead of [`Self::create_form`] to customize runtime input
        /// limits, finding visibility, or external-finding limits.
        pub fn form(&self, form_data: Value) -> FormBuilder<'_> {
            FormBuilder::new(self, form_data)
        }

        /// Iterates the extension namespaces the authored UI schema requires.
        ///
        /// The compiler validates declarations and returns each namespace once in
        /// deterministic order; callers remain responsible for providing them.
        pub fn required_extensions(&self) -> impl Iterator<Item = &ExtensionNamespace> {
            self.inner.required_extensions.iter()
        }

        /// Iterates all capability findings retained during compilation.
        ///
        /// A definition produced by strict [`Self::compile`] has no blocking
        /// findings. Definitions obtained through analysis may contain them.
        pub fn capability_findings(&self) -> impl Iterator<Item = &CapabilityFinding> {
            self.inner.capability_report.findings()
        }

        pub(crate) fn array_template(&self, array: DefinitionNodeId) -> Option<DefinitionNodeId> {
            self.inner
                .nodes
                .get(usize::try_from(array.0).ok()?)?
                .children
                .iter()
                .copied()
                .find(|child| {
                    self.inner
                        .nodes
                        .get(child.0 as usize)
                        .is_some_and(|node| node.owning_array == Some(array))
                })
        }

        pub(crate) fn array_for_template(
            &self,
            template: DefinitionNodeId,
        ) -> Option<DefinitionNodeId> {
            self.inner
                .nodes
                .get(usize::try_from(template.0).ok()?)?
                .owning_array
        }

        pub(crate) fn initial_tree_metrics(&self, form_data: &Value) -> (usize, usize) {
            let mut form_tree_nodes = self
                .inner
                .nodes
                .iter()
                .filter(|node| node.owning_array.is_none())
                .count();
            let mut repeated_items = 0usize;
            for array in self.inner.nodes.iter().filter(|node| {
                node.owning_array.is_none()
                    && node.semantic_kind == Some(SemanticKind::HomogeneousArray)
            }) {
                let item_count = array
                    .binding
                    .as_ref()
                    .and_then(|binding| form_data.pointer(binding.as_str()))
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                repeated_items = repeated_items.saturating_add(item_count);
                let template_nodes = self
                    .inner
                    .nodes
                    .iter()
                    .filter(|node| node.owning_array == Some(array.id))
                    .count();
                form_tree_nodes =
                    form_tree_nodes.saturating_add(item_count.saturating_mul(template_nodes));
            }
            (form_tree_nodes, repeated_items)
        }

        pub(crate) fn binding_is_read_only(&self, binding: &JsonPointer) -> bool {
            self.binding_has_annotation(binding, DataSchemaAnnotations::is_read_only)
        }

        pub(crate) fn binding_is_write_only(&self, binding: &JsonPointer) -> bool {
            self.binding_has_annotation(binding, DataSchemaAnnotations::is_write_only)
        }

        fn binding_has_annotation(
            &self,
            binding: &JsonPointer,
            has_annotation: fn(&DataSchemaAnnotations) -> bool,
        ) -> bool {
            let binding = jsonptr::Pointer::parse(binding.as_str())
                .expect("definition bindings are valid JSON Pointers");
            self.inner.annotation_scopes.iter().any(|scope| {
                if !has_annotation(&scope.annotations) {
                    return false;
                }
                scope.binding.as_ref().is_none_or(|ancestor| {
                    let ancestor = jsonptr::Pointer::parse(ancestor.as_str())
                        .expect("definition bindings are valid JSON Pointers");
                    binding.starts_with(ancestor)
                })
            })
        }

        pub(crate) fn template_has_annotation(
            &self,
            node: DefinitionNodeId,
            has_annotation: fn(&DataSchemaAnnotations) -> bool,
        ) -> bool {
            let Some(target) = self.inner.nodes.get(node.0 as usize) else {
                return false;
            };
            let Some(owner) = target.owning_array else {
                return false;
            };
            let Some(target_binding) = target
                .binding
                .as_ref()
                .and_then(|binding| jsonptr::Pointer::parse(binding.as_str()).ok())
            else {
                return false;
            };
            self.inner.nodes.iter().any(|candidate| {
                if candidate.owning_array != Some(owner)
                    || !has_annotation(&candidate.data_schema_annotations)
                {
                    return false;
                }
                candidate.binding.as_ref().is_some_and(|binding| {
                    jsonptr::Pointer::parse(binding.as_str())
                        .is_ok_and(|ancestor| target_binding.starts_with(ancestor))
                })
            })
        }
    }

    fn compile_authored_nodes(
        ui_schema: &ui::v1::UiSchema,
        generated_nodes: &[GeneratedNode],
        root_annotations: &DataSchemaAnnotations,
        root_schema_locations: Vec<SchemaLocation>,
        profile: &CompilationProfile,
    ) -> Result<Vec<DefinitionNode>, CompileError> {
        ui_schema
            .validate_limits(profile)
            .map_err(|error| invalid_ui_schema(error.location, error.kind))?;
        let mut nodes = vec![DefinitionNode {
            id: DefinitionNodeId(0),
            authored_id: None,
            kind: DefinitionNodeKind::AutoGeneratedLayout,
            semantic_kind: None,
            binding: None,
            widget: None,
            extensions: BTreeMap::new(),
            label: String::new(),
            label_reference: None,
            label_visible: true,
            help: None,
            help_reference: None,
            item_label_reference: None,
            text: None,
            data_schema_annotations: root_annotations.clone(),
            creation_seed: None,
            required: false,
            accepts_null: false,
            choice_options: Vec::new(),
            choice_selectable: false,
            owning_array: None,
            grid_spans: None,
            schema_locations: root_schema_locations,
            children: Vec::new(),
        }];
        let mut bindings = HashSet::new();
        let mut authored_ids = HashSet::new();
        let root = compile_authored_element(
            ui_schema.root(),
            "/root",
            generated_nodes,
            AuthoredBindingContext::root(),
            &mut bindings,
            &mut authored_ids,
            &mut nodes,
        )?;
        nodes[0].children.extend(root);
        Ok(nodes)
    }

    #[derive(Clone, Copy)]
    struct AuthoredBindingContext<'a> {
        origin: ui::v1::BindingOrigin,
        owning_array_binding: Option<&'a JsonPointer>,
        owning_array: Option<DefinitionNodeId>,
    }

    impl AuthoredBindingContext<'_> {
        fn root() -> Self {
            Self {
                origin: ui::v1::BindingOrigin::Root,
                owning_array_binding: None,
                owning_array: None,
            }
        }
    }

    fn compile_authored_element(
        element: &ui::v1::Element,
        location: &str,
        generated_nodes: &[GeneratedNode],
        binding_context: AuthoredBindingContext<'_>,
        bindings: &mut HashSet<String>,
        authored_ids: &mut HashSet<String>,
        nodes: &mut Vec<DefinitionNode>,
    ) -> Result<Vec<DefinitionNodeId>, CompileError> {
        let value_location = format!("{location}/value");
        match element {
            ui::v1::Element::Control(control) => {
                let authored_id =
                    register_authored_id(control.meta_value(), &value_location, authored_ids)?;
                if control.binding().origin() != binding_context.origin {
                    return Err(invalid_ui_schema(
                        format!("{value_location}/binding/origin"),
                        UiSchemaInputErrorKind::InvalidBindingOrigin,
                    ));
                }
                let binding = control.binding().pointer().as_str();
                let Some(generated) = generated_nodes.iter().find(|node| {
                    node.owning_array_binding.as_deref()
                        == binding_context
                            .owning_array_binding
                            .map(JsonPointer::as_str)
                        && node.binding == binding
                        && node.kind == DefinitionNodeKind::Control
                        && (node.semantic_kind != Some(SemanticKind::HomogeneousArray)
                            || (binding_context.origin == ui::v1::BindingOrigin::Root
                                && control.item_template_value().is_some()))
                }) else {
                    return Err(invalid_ui_schema(
                        format!("{value_location}/binding/pointer"),
                        UiSchemaInputErrorKind::UnknownBinding,
                    ));
                };
                if !bindings.insert(binding.to_owned()) {
                    return Err(invalid_ui_schema(
                        format!("{value_location}/binding/pointer"),
                        UiSchemaInputErrorKind::DuplicateBinding,
                    ));
                }
                let (label, label_reference, label_visible) = match control.label_setting() {
                    ui::v1::TextSetting::Inherit => (generated.label.clone(), None, true),
                    ui::v1::TextSetting::Suppress => (generated.label.clone(), None, false),
                    ui::v1::TextSetting::Value(reference) => (
                        reference.fallback().to_owned(),
                        Some(reference.clone()),
                        true,
                    ),
                };
                let (help, help_reference) = match control.help_setting() {
                    ui::v1::TextSetting::Inherit => (generated.help.clone(), None),
                    ui::v1::TextSetting::Suppress => (None, None),
                    ui::v1::TextSetting::Value(reference) => (
                        Some(reference.fallback().to_owned()),
                        Some(reference.clone()),
                    ),
                };
                if generated.semantic_kind != Some(SemanticKind::HomogeneousArray)
                    && control.item_template_value().is_some()
                {
                    return Err(invalid_ui_schema(
                        format!("{value_location}/item_template"),
                        UiSchemaInputErrorKind::UnsupportedField,
                    ));
                }
                if generated.semantic_kind != Some(SemanticKind::HomogeneousArray)
                    && control.item_label_value().is_some()
                {
                    return Err(invalid_ui_schema(
                        format!("{value_location}/item_label"),
                        UiSchemaInputErrorKind::UnsupportedField,
                    ));
                }
                let id = push_definition_node(
                    nodes,
                    DefinitionNode {
                        id: DefinitionNodeId(0),
                        authored_id,
                        kind: DefinitionNodeKind::Control,
                        semantic_kind: generated.semantic_kind,
                        binding: Some(control.binding().pointer().clone()),
                        widget: control.widget_value().cloned(),
                        extensions: control.meta_value().extensions_value().clone(),
                        label,
                        label_reference,
                        label_visible,
                        help,
                        help_reference,
                        item_label_reference: control.item_label_value().cloned(),
                        text: None,
                        data_schema_annotations: generated.data_schema_annotations.clone(),
                        creation_seed: generated.creation_seed.clone(),
                        required: generated.required,
                        accepts_null: generated.accepts_null,
                        choice_options: generated.choice_options.clone(),
                        choice_selectable: generated.choice_selectable,
                        owning_array: binding_context.owning_array,
                        grid_spans: None,
                        schema_locations: generated.schema_locations.clone(),
                        children: Vec::new(),
                    },
                );
                if let Some(template) = control.item_template_value() {
                    let mut template_bindings = HashSet::new();
                    let authored_children = compile_authored_element(
                        template,
                        &format!("{value_location}/item_template"),
                        generated_nodes,
                        AuthoredBindingContext {
                            origin: ui::v1::BindingOrigin::ItemTemplate,
                            owning_array_binding: Some(control.binding().pointer()),
                            owning_array: Some(id),
                        },
                        &mut template_bindings,
                        authored_ids,
                        nodes,
                    )?;
                    let children = generated_nodes
                        .iter()
                        .find(|node| {
                            node.owning_array_binding.as_deref() == Some(binding)
                                && node.binding.is_empty()
                                && node.kind == DefinitionNodeKind::AutoGeneratedLayout
                                && node.semantic_kind == Some(SemanticKind::FixedObject)
                        })
                        .filter(|_| {
                            !matches!(
                                authored_children.as_slice(),
                                [authored_root]
                                    if nodes[authored_root.0 as usize].semantic_kind
                                        == Some(SemanticKind::FixedObject)
                                        && nodes[authored_root.0 as usize]
                                            .binding
                                            .as_ref()
                                            .is_some_and(|binding| binding.as_str().is_empty())
                            )
                        })
                        .map(|item_object| {
                            vec![push_definition_node(
                                nodes,
                                DefinitionNode {
                                    id: DefinitionNodeId(0),
                                    authored_id: None,
                                    kind: item_object.kind,
                                    semantic_kind: item_object.semantic_kind,
                                    binding: Some(
                                        JsonPointer::parse(item_object.binding.clone()).expect(
                                            "compiled engine bindings are valid JSON Pointers",
                                        ),
                                    ),
                                    widget: None,
                                    extensions: BTreeMap::new(),
                                    label: item_object.label.clone(),
                                    label_reference: None,
                                    label_visible: true,
                                    help: item_object.help.clone(),
                                    help_reference: None,
                                    item_label_reference: None,
                                    text: None,
                                    data_schema_annotations: item_object
                                        .data_schema_annotations
                                        .clone(),
                                    creation_seed: item_object.creation_seed.clone(),
                                    required: item_object.required,
                                    accepts_null: item_object.accepts_null,
                                    choice_options: item_object.choice_options.clone(),
                                    choice_selectable: item_object.choice_selectable,
                                    owning_array: Some(id),
                                    grid_spans: None,
                                    schema_locations: item_object.schema_locations.clone(),
                                    children: authored_children.clone(),
                                },
                            )]
                        })
                        .unwrap_or(authored_children);
                    nodes[id.0 as usize].children = children;
                }
                Ok(vec![id])
            }
            ui::v1::Element::Stack(stack) => {
                let authored_id =
                    register_authored_id(stack.meta_value(), &value_location, authored_ids)?;
                let children = stack
                    .children()
                    .iter()
                    .enumerate()
                    .map(|(index, child)| {
                        compile_authored_element(
                            child,
                            &format!("{value_location}/children/{index}"),
                            generated_nodes,
                            binding_context,
                            bindings,
                            authored_ids,
                            nodes,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .flatten()
                    .collect();
                let mut node = presentation_node(DefinitionNodeKind::Stack, children);
                node.authored_id = authored_id;
                node.extensions = stack.meta_value().extensions_value().clone();
                node.owning_array = binding_context.owning_array;
                Ok(vec![push_definition_node(nodes, node)])
            }
            ui::v1::Element::Grid(grid) => {
                let authored_id =
                    register_authored_id(grid.meta_value(), &value_location, authored_ids)?;
                let cells = grid
                    .cells()
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| {
                        let children = compile_authored_element(
                            cell.child(),
                            &format!("{value_location}/cells/{index}/child"),
                            generated_nodes,
                            binding_context,
                            bindings,
                            authored_ids,
                            nodes,
                        )?;
                        let mut node = presentation_node(DefinitionNodeKind::GridCell, children);
                        node.owning_array = binding_context.owning_array;
                        node.grid_spans = Some(GridSpans {
                            compact: cell.compact_span().get(),
                            wide: cell.effective_wide_span().get(),
                        });
                        Ok(push_definition_node(nodes, node))
                    })
                    .collect::<Result<Vec<_>, CompileError>>()?;
                let mut node = presentation_node(DefinitionNodeKind::Grid, cells);
                node.authored_id = authored_id;
                node.extensions = grid.meta_value().extensions_value().clone();
                node.owning_array = binding_context.owning_array;
                Ok(vec![push_definition_node(nodes, node)])
            }
            ui::v1::Element::Group(group) => {
                let authored_id =
                    register_authored_id(group.meta_value(), &value_location, authored_ids)?;
                let children = compile_authored_element(
                    group.child(),
                    &format!("{value_location}/child"),
                    generated_nodes,
                    binding_context,
                    bindings,
                    authored_ids,
                    nodes,
                )?;
                let mut node = presentation_node(DefinitionNodeKind::Group, children);
                node.authored_id = authored_id;
                node.extensions = group.meta_value().extensions_value().clone();
                node.label = group.title().fallback().to_owned();
                node.label_reference = Some(group.title().clone());
                node.owning_array = binding_context.owning_array;
                Ok(vec![push_definition_node(nodes, node)])
            }
            ui::v1::Element::Tabs(tabs) => {
                let authored_id =
                    register_authored_id(tabs.meta_value(), &value_location, authored_ids)?;
                let panels = tabs
                    .panels()
                    .iter()
                    .enumerate()
                    .map(|(index, panel)| {
                        let children = compile_authored_element(
                            panel.child(),
                            &format!("{value_location}/panels/{index}/child"),
                            generated_nodes,
                            binding_context,
                            bindings,
                            authored_ids,
                            nodes,
                        )?;
                        let mut node = presentation_node(DefinitionNodeKind::TabPanel, children);
                        node.label = panel.title().fallback().to_owned();
                        node.label_reference = Some(panel.title().clone());
                        node.owning_array = binding_context.owning_array;
                        Ok(push_definition_node(nodes, node))
                    })
                    .collect::<Result<Vec<_>, CompileError>>()?;
                let mut node = presentation_node(DefinitionNodeKind::Tabs, panels);
                node.authored_id = authored_id;
                node.extensions = tabs.meta_value().extensions_value().clone();
                node.owning_array = binding_context.owning_array;
                Ok(vec![push_definition_node(nodes, node)])
            }
            ui::v1::Element::Text(text) => {
                let authored_id =
                    register_authored_id(text.meta_value(), &value_location, authored_ids)?;
                let mut node = presentation_node(DefinitionNodeKind::Text, Vec::new());
                node.authored_id = authored_id;
                node.extensions = text.meta_value().extensions_value().clone();
                node.text = Some(text.content().clone());
                node.owning_array = binding_context.owning_array;
                Ok(vec![push_definition_node(nodes, node)])
            }
            ui::v1::Element::Auto(auto) => {
                let authored_id =
                    register_authored_id(auto.meta_value(), &value_location, authored_ids)?;
                if auto.binding().origin() != binding_context.origin {
                    return Err(invalid_ui_schema(
                        format!("{value_location}/binding/origin"),
                        UiSchemaInputErrorKind::InvalidBindingOrigin,
                    ));
                }
                let binding = auto.binding().pointer().as_str();
                let owning_array_binding = binding_context
                    .owning_array_binding
                    .map(JsonPointer::as_str);
                let (explicitly_ordered, remaining_seen) =
                    validate_auto_property_order(auto.properties_value(), &value_location)?;
                let generated_parent = if binding.is_empty() && owning_array_binding.is_none() {
                    None
                } else {
                    generated_nodes.iter().position(|node| {
                        node.owning_array_binding.as_deref() == owning_array_binding
                            && node.binding == binding
                            && node.kind == DefinitionNodeKind::AutoGeneratedLayout
                            && node.semantic_kind == Some(SemanticKind::FixedObject)
                    })
                };
                if !(binding.is_empty() && owning_array_binding.is_none())
                    && generated_parent.is_none()
                {
                    if let Some(unsupported) = generated_nodes.iter().position(|node| {
                        node.owning_array_binding.as_deref() == owning_array_binding
                            && node.binding == binding
                            && node.kind == DefinitionNodeKind::Unsupported
                    }) {
                        let children = materialize_generated_region(
                            generated_nodes,
                            None,
                            &[unsupported],
                            binding_context,
                            &format!("{value_location}/binding/pointer"),
                            bindings,
                            nodes,
                        )?;
                        return Ok(wrap_auto_meta(
                            auto,
                            authored_id,
                            binding_context,
                            children,
                            nodes,
                        ));
                    }
                    return Err(invalid_ui_schema(
                        format!("{value_location}/binding/pointer"),
                        UiSchemaInputErrorKind::UnknownBinding,
                    ));
                }
                let direct_properties =
                    direct_generated_properties(generated_nodes, binding_context, binding);
                let selected = select_auto_properties(
                    auto.properties_value(),
                    &direct_properties,
                    &value_location,
                    &explicitly_ordered,
                    remaining_seen,
                )?;
                let selected = if binding.is_empty() && owning_array_binding.is_none() {
                    root_auto_roots(generated_nodes, selected)
                } else {
                    selected
                };
                let children = materialize_generated_region(
                    generated_nodes,
                    generated_parent,
                    &selected,
                    binding_context,
                    &format!("{value_location}/binding/pointer"),
                    bindings,
                    nodes,
                )?;
                Ok(wrap_auto_meta(
                    auto,
                    authored_id,
                    binding_context,
                    children,
                    nodes,
                ))
            }
        }
    }

    fn wrap_auto_meta(
        auto: &ui::v1::Auto,
        authored_id: Option<String>,
        binding_context: AuthoredBindingContext<'_>,
        children: Vec<DefinitionNodeId>,
        nodes: &mut Vec<DefinitionNode>,
    ) -> Vec<DefinitionNodeId> {
        if authored_id.is_none() && auto.meta_value().extensions_value().is_empty() {
            return children;
        }
        let mut node = presentation_node(DefinitionNodeKind::AutoGeneratedLayout, children);
        node.authored_id = authored_id;
        node.owning_array = binding_context.owning_array;
        node.extensions = auto.meta_value().extensions_value().clone();
        vec![push_definition_node(nodes, node)]
    }

    fn direct_generated_properties(
        generated_nodes: &[GeneratedNode],
        binding_context: AuthoredBindingContext<'_>,
        binding: &str,
    ) -> BTreeMap<String, usize> {
        let owning_array_binding = binding_context
            .owning_array_binding
            .map(JsonPointer::as_str);
        generated_nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.owning_array_binding.as_deref() == owning_array_binding
                    && node.binding != binding
                    && if binding.is_empty() && owning_array_binding.is_none() {
                        node.parent_binding.is_none()
                    } else {
                        node.parent_binding.as_deref() == Some(binding)
                    }
            })
            .map(|(index, node)| (generated_property_name(node), index))
            .collect()
    }

    fn root_auto_roots(
        generated_nodes: &[GeneratedNode],
        mut properties: Vec<usize>,
    ) -> Vec<usize> {
        if let Some(root_unsupported) = generated_nodes.iter().position(|node| {
            node.owning_array_binding.is_none()
                && node.binding.is_empty()
                && node.kind == DefinitionNodeKind::Unsupported
        }) {
            properties.insert(0, root_unsupported);
        }
        properties
    }

    fn validate_auto_property_order(
        selection: &ui::v1::PropertySelection,
        value_location: &str,
    ) -> Result<(BTreeSet<String>, bool), CompileError> {
        let mut explicitly_ordered = BTreeSet::new();
        let mut remaining_seen = false;
        for (index, position) in selection.order_value().iter().enumerate() {
            match position {
                ui::v1::PropertyPosition::Property(property) => {
                    if !explicitly_ordered.insert(property.clone()) {
                        return Err(invalid_ui_schema(
                            format!("{value_location}/properties/order/{index}/property"),
                            UiSchemaInputErrorKind::InvalidPropertySelection,
                        ));
                    }
                }
                ui::v1::PropertyPosition::Remaining => {
                    if remaining_seen {
                        return Err(invalid_ui_schema(
                            format!("{value_location}/properties/order/{index}"),
                            UiSchemaInputErrorKind::InvalidPropertySelection,
                        ));
                    }
                    remaining_seen = true;
                }
            }
        }
        Ok((explicitly_ordered, remaining_seen))
    }

    fn select_auto_properties(
        selection: &ui::v1::PropertySelection,
        direct_properties: &BTreeMap<String, usize>,
        value_location: &str,
        explicitly_ordered: &BTreeSet<String>,
        remaining_seen: bool,
    ) -> Result<Vec<usize>, CompileError> {
        let mut selected = if selection.include_value().is_empty() {
            direct_properties.keys().cloned().collect::<BTreeSet<_>>()
        } else {
            let mut selected = BTreeSet::new();
            for (index, property) in selection.include_value().iter().enumerate() {
                if !direct_properties.contains_key(property) {
                    return Err(invalid_ui_schema(
                        format!("{value_location}/properties/include/{index}"),
                        UiSchemaInputErrorKind::UnknownBinding,
                    ));
                }
                selected.insert(property.clone());
            }
            selected
        };
        for (index, property) in selection.exclude_value().iter().enumerate() {
            if !direct_properties.contains_key(property) {
                return Err(invalid_ui_schema(
                    format!("{value_location}/properties/exclude/{index}"),
                    UiSchemaInputErrorKind::UnknownBinding,
                ));
            }
            selected.remove(property);
        }

        for (index, position) in selection.order_value().iter().enumerate() {
            if let ui::v1::PropertyPosition::Property(property) = position
                && !direct_properties.contains_key(property)
            {
                return Err(invalid_ui_schema(
                    format!("{value_location}/properties/order/{index}/property"),
                    UiSchemaInputErrorKind::UnknownBinding,
                ));
            }
        }

        let remaining = selected
            .iter()
            .filter(|property| !explicitly_ordered.contains(*property))
            .map(|property| direct_properties[property])
            .collect::<Vec<_>>();
        let mut ordered = Vec::new();
        for position in selection.order_value() {
            match position {
                ui::v1::PropertyPosition::Property(property) => {
                    if selected.contains(property) {
                        ordered.push(direct_properties[property]);
                    }
                }
                ui::v1::PropertyPosition::Remaining => {
                    ordered.extend(remaining.iter().copied());
                }
            }
        }
        if !remaining_seen {
            ordered.extend(remaining);
        }
        Ok(ordered)
    }

    fn materialize_generated_region(
        generated_nodes: &[GeneratedNode],
        generated_parent: Option<usize>,
        roots: &[usize],
        binding_context: AuthoredBindingContext<'_>,
        error_location: &str,
        bindings: &mut HashSet<String>,
        nodes: &mut Vec<DefinitionNode>,
    ) -> Result<Vec<DefinitionNodeId>, CompileError> {
        let mut included = vec![false; generated_nodes.len()];
        if let Some(generated_parent) = generated_parent {
            included[generated_parent] = true;
        }
        for root in roots {
            include_generated_subtree(*root, generated_nodes, &mut included);
        }
        let mut ids = HashMap::new();
        for (index, is_included) in included.iter().copied().enumerate() {
            if is_included {
                ids.insert(index, DefinitionNodeId((nodes.len() + ids.len()) as u64));
            }
        }
        for (index, generated) in generated_nodes.iter().enumerate() {
            if !included[index] {
                continue;
            }
            let owns_root_binding = generated.owning_array_binding.as_deref()
                == binding_context
                    .owning_array_binding
                    .map(JsonPointer::as_str)
                && (generated.kind == DefinitionNodeKind::Control
                    || (generated.kind == DefinitionNodeKind::AutoGeneratedLayout
                        && generated.semantic_kind == Some(SemanticKind::FixedObject)));
            if owns_root_binding && !bindings.insert(generated.binding.clone()) {
                return Err(invalid_ui_schema(
                    error_location,
                    UiSchemaInputErrorKind::DuplicateBinding,
                ));
            }
            let child_indices = if Some(index) == generated_parent {
                roots.to_vec()
            } else {
                generated_child_indices(index, generated_nodes)
            };
            let children = child_indices
                .into_iter()
                .filter(|child| included[*child])
                .map(|child| ids[&child])
                .collect();
            let owning_array = generated.owning_array_binding.as_ref().and_then(|owner| {
                if let Some(owning_array) = binding_context.owning_array {
                    return Some(owning_array);
                }
                generated_nodes
                    .iter()
                    .enumerate()
                    .find(|(_, candidate)| {
                        candidate.owning_array_binding.is_none()
                            && candidate.binding == *owner
                            && candidate.semantic_kind == Some(SemanticKind::HomogeneousArray)
                    })
                    .and_then(|(owner_index, _)| ids.get(&owner_index))
                    .copied()
            });
            nodes.push(DefinitionNode {
                id: ids[&index],
                authored_id: None,
                kind: generated.kind,
                semantic_kind: generated.semantic_kind,
                binding: Some(
                    JsonPointer::parse(generated.binding.clone())
                        .expect("compiled engine bindings are valid JSON Pointers"),
                ),
                widget: None,
                extensions: BTreeMap::new(),
                label: generated.label.clone(),
                label_reference: None,
                label_visible: true,
                help: generated.help.clone(),
                help_reference: None,
                item_label_reference: None,
                text: None,
                data_schema_annotations: generated.data_schema_annotations.clone(),
                creation_seed: generated.creation_seed.clone(),
                required: generated.required,
                accepts_null: generated.accepts_null,
                choice_options: generated.choice_options.clone(),
                choice_selectable: generated.choice_selectable,
                owning_array,
                grid_spans: None,
                schema_locations: generated.schema_locations.clone(),
                children,
            });
        }
        Ok(if let Some(generated_parent) = generated_parent {
            vec![ids[&generated_parent]]
        } else {
            roots.iter().map(|root| ids[root]).collect()
        })
    }

    fn include_generated_subtree(
        index: usize,
        generated_nodes: &[GeneratedNode],
        included: &mut [bool],
    ) {
        if included[index] {
            return;
        }
        included[index] = true;
        for child in generated_child_indices(index, generated_nodes) {
            include_generated_subtree(child, generated_nodes, included);
        }
    }

    fn generated_child_indices(index: usize, generated_nodes: &[GeneratedNode]) -> Vec<usize> {
        let parent = &generated_nodes[index];
        let mut children = generated_nodes
            .iter()
            .enumerate()
            .filter_map(|(child_index, child)| {
                let ordinary_child = child.owning_array_binding == parent.owning_array_binding
                    && child.parent_binding.as_deref() == Some(parent.binding.as_str());
                let item_template = parent.owning_array_binding.is_none()
                    && parent.semantic_kind == Some(SemanticKind::HomogeneousArray)
                    && child.owning_array_binding.as_deref() == Some(parent.binding.as_str())
                    && child.parent_binding.is_none();
                (ordinary_child || item_template).then_some(child_index)
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            generated_property_name(&generated_nodes[*left])
                .cmp(&generated_property_name(&generated_nodes[*right]))
        });
        children
    }

    fn generated_property_name(node: &GeneratedNode) -> String {
        jsonptr::Pointer::parse(&node.binding)
            .expect("compiled engine bindings are valid JSON Pointers")
            .back()
            .map(|token| token.decoded().into_owned())
            .unwrap_or_default()
    }

    fn push_definition_node(
        nodes: &mut Vec<DefinitionNode>,
        mut node: DefinitionNode,
    ) -> DefinitionNodeId {
        let id = DefinitionNodeId(nodes.len() as u64);
        node.id = id;
        nodes.push(node);
        id
    }

    fn presentation_node(
        kind: DefinitionNodeKind,
        children: Vec<DefinitionNodeId>,
    ) -> DefinitionNode {
        DefinitionNode {
            id: DefinitionNodeId(0),
            authored_id: None,
            kind,
            semantic_kind: None,
            binding: None,
            widget: None,
            extensions: BTreeMap::new(),
            label: String::new(),
            label_reference: None,
            label_visible: true,
            help: None,
            help_reference: None,
            item_label_reference: None,
            text: None,
            data_schema_annotations: DataSchemaAnnotations::default(),
            creation_seed: None,
            required: false,
            accepts_null: false,
            choice_options: Vec::new(),
            choice_selectable: false,
            owning_array: None,
            grid_spans: None,
            schema_locations: Vec::new(),
            children,
        }
    }

    fn register_authored_id(
        meta: &ui::v1::ElementMeta,
        value_location: &str,
        authored_ids: &mut HashSet<String>,
    ) -> Result<Option<String>, CompileError> {
        let Some(authored_id) = meta.id_value() else {
            return Ok(None);
        };
        if !authored_ids.insert(authored_id.to_owned()) {
            return Err(invalid_ui_schema(
                format!("{value_location}/id"),
                UiSchemaInputErrorKind::DuplicateElementId,
            ));
        }
        Ok(Some(authored_id.to_owned()))
    }

    fn invalid_ui_schema(
        location: impl Into<String>,
        kind: UiSchemaInputErrorKind,
    ) -> CompileError {
        CompileError::Input(InputError::InvalidUiSchema(UiSchemaInputError::new(
            location, kind,
        )))
    }

    /// Configures qualification, resource resolution, projection, and limits for
    /// one immutable [`FormDefinition`].
    ///
    /// Builder calls perform no compilation or I/O. All schemas and resources are
    /// consumed and checked only by [`Self::compile`] or [`Self::analyze`].
    #[derive(Clone)]
    pub struct FormCompiler {
        data_schema: Value,
        root_uri: Option<RetrievalUri>,
        resources: Vec<SchemaResource>,
        ui_schema: Option<ui::v1::UiSchema>,
        profile: CompilationProfile,
        default_dialect: Option<Dialect>,
    }

    impl FormCompiler {
        /// Creates a compiler for a root data schema using release defaults.
        ///
        /// In the absence of [`Self::root_uri`], an implementation-defined
        /// absolute URI identifies the root resource.
        pub fn new(data_schema: Value) -> Self {
            Self {
                data_schema,
                root_uri: None,
                resources: Vec::new(),
                ui_schema: None,
                profile: CompilationProfile::default(),
                default_dialect: None,
            }
        }

        /// Sets the absolute, fragment-free retrieval URI of the root resource.
        ///
        /// Canonical identities declared by the schema may still determine the
        /// resource identity used during reference resolution.
        pub fn root_uri(mut self, uri: RetrievalUri) -> Self {
            self.root_uri = Some(uri);
            self
        }

        /// Adds one caller-supplied schema resource to the resolution graph.
        ///
        /// Resources are never fetched. Duplicate identities, unresolved
        /// references, and configured resource limits fail compilation or analysis.
        pub fn resource(mut self, resource: SchemaResource) -> Self {
            self.resources.push(resource);
            self
        }

        /// Uses an authored UI schema instead of the generated projection.
        ///
        /// The UI schema is validated against compiled bindings, extension
        /// declarations, and the active [`CompilationProfile`].
        pub fn ui_schema(mut self, ui_schema: ui::v1::UiSchema) -> Self {
            self.ui_schema = Some(ui_schema);
            self
        }

        /// Supplies a dialect for the root and caller-supplied schema resources
        /// that omit `$schema`.
        ///
        /// An explicit resource declaration still takes precedence. Only
        /// dialects represented by [`Dialect`] are accepted.
        pub fn default_dialect(mut self, dialect: Dialect) -> Self {
            self.default_dialect = Some(dialect);
            self
        }

        /// Replaces all compilation resource limits with `profile`.
        pub fn profile(mut self, profile: CompilationProfile) -> Self {
            self.profile = profile;
            self
        }

        fn analyze_definition(self) -> Result<FormDefinition, CompileError> {
            FormDefinition::analyze_at(
                self.data_schema,
                self.root_uri
                    .unwrap_or_else(crate::validation::default_root_uri),
                self.resources,
                self.ui_schema,
                self.profile,
                self.default_dialect,
            )
        }

        /// Strictly compiles the configured inputs into a reusable definition.
        ///
        /// Unlike [`Self::analyze`], this returns [`CompileError::Capability`] if
        /// the resulting report contains any blocking finding. Warnings remain on
        /// the returned definition. Compilation performs no network or other I/O.
        pub fn compile(self) -> Result<FormDefinition, CompileError> {
            FormDefinition::require_no_blocking_capabilities(self.analyze_definition()?)
        }

        /// Leniently analyzes the configured inputs and returns the definition and
        /// its complete capability report when projection can proceed.
        ///
        /// Blocking findings retained in a successful [`FormAnalysis`] are data,
        /// not errors. Invalid inputs, resource/qualification failures, and
        /// capability failures that prevent any definition still return
        /// [`AnalysisError`].
        pub fn analyze(self) -> Result<FormAnalysis, AnalysisError> {
            let definition = self.analyze_definition().map_err(|error| match error {
                CompileError::Input(error) => AnalysisError::Input(error),
                CompileError::Resource(error) => AnalysisError::Resource(error),
                CompileError::Qualification(error) => AnalysisError::Qualification(error),
                CompileError::Capability(report) => AnalysisError::Capability(report),
            })?;
            let report = definition.inner.capability_report.clone();
            Ok(FormAnalysis { definition, report })
        }
    }

    /// One caller-supplied schema document and its absolute retrieval identity.
    ///
    /// Resources are resolved entirely in memory and are never fetched.
    #[derive(Debug, Clone)]
    pub struct SchemaResource {
        uri: RetrievalUri,
        document: Value,
    }

    impl SchemaResource {
        /// Associates a parsed schema document with its retrieval URI.
        pub fn new(uri: RetrievalUri, document: Value) -> Self {
            Self { uri, document }
        }

        /// Returns the absolute, fragment-free retrieval URI.
        pub fn uri(&self) -> &RetrievalUri {
            &self.uri
        }

        /// Borrows the parsed JSON schema document.
        pub fn document(&self) -> &Value {
            &self.document
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum Dialect {
        Draft202012,
    }

    /// Resource limits for bounded parsing and compilation of data-schema and
    /// UI-schema inputs.
    ///
    /// These limits do not make an untrusted data schema safe for evaluation.
    /// Each builder replaces one inclusive maximum; an observed value greater
    /// than the maximum fails. Source-byte and token limits apply in the bounded
    /// parsing APIs. A maximum of zero disables any nonempty input in that
    /// dimension rather than disabling the check.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CompilationProfile {
        max_data_schema_bytes: usize,
        max_data_schema_tokens: usize,
        max_data_schema_depth: usize,
        max_data_schema_nodes: usize,
        max_data_schema_members: usize,
        max_data_schema_scalar_bytes: usize,
        max_data_schema_resources: usize,
        max_data_schema_references: usize,
        max_data_schema_traversal: usize,
        max_definition_nodes: usize,
        max_controls: usize,
        max_uri_bytes: usize,
        max_pointer_bytes: usize,
        max_capability_findings: usize,
        max_ui_schema_bytes: usize,
        max_ui_schema_tokens: usize,
        max_ui_schema_depth: usize,
        max_ui_schema_nodes: usize,
        max_ui_schema_members: usize,
        max_ui_schema_collection_length: usize,
        max_ui_schema_scalar_bytes: usize,
        max_extension_namespaces: usize,
        max_extension_occurrences: usize,
        max_extension_namespace_bytes: usize,
        max_extension_value_depth: usize,
        max_extension_value_nodes: usize,
        max_extension_value_bytes: usize,
    }

    impl CompilationProfile {
        /// Returns the standard bounded profile for this crate version.
        pub fn standard() -> Self {
            Self::default()
        }

        /// Sets the maximum source bytes accepted when parsing a data schema.
        pub fn max_data_schema_bytes(mut self, maximum: usize) -> Self {
            self.max_data_schema_bytes = maximum;
            self
        }

        /// Sets the maximum token count accepted when parsing a data schema.
        pub fn max_data_schema_tokens(mut self, maximum: usize) -> Self {
            self.max_data_schema_tokens = maximum;
            self
        }

        /// Sets the maximum nesting depth of a data-schema input.
        pub fn max_data_schema_depth(mut self, maximum: usize) -> Self {
            self.max_data_schema_depth = maximum;
            self
        }

        /// Sets the maximum aggregate JSON node count across schema resources.
        pub fn max_data_schema_nodes(mut self, maximum: usize) -> Self {
            self.max_data_schema_nodes = maximum;
            self
        }

        /// Sets the maximum length of one data-schema object or array.
        pub fn max_data_schema_members(mut self, maximum: usize) -> Self {
            self.max_data_schema_members = maximum;
            self
        }

        /// Sets the maximum encoded bytes of one data-schema scalar.
        pub fn max_data_schema_scalar_bytes(mut self, maximum: usize) -> Self {
            self.max_data_schema_scalar_bytes = maximum;
            self
        }

        /// Sets the maximum number of root, embedded, and supplied resources.
        pub fn max_data_schema_resources(mut self, maximum: usize) -> Self {
            self.max_data_schema_resources = maximum;
            self
        }

        /// Sets the maximum number of references in the qualified schema graph.
        pub fn max_data_schema_references(mut self, maximum: usize) -> Self {
            self.max_data_schema_references = maximum;
            self
        }

        /// Sets the maximum visits permitted in each bounded graph or projection traversal.
        pub fn max_data_schema_traversal(mut self, maximum: usize) -> Self {
            self.max_data_schema_traversal = maximum;
            self
        }

        /// Sets the maximum number of nodes in the compiled definition tree.
        pub fn max_definition_nodes(mut self, maximum: usize) -> Self {
            self.max_definition_nodes = maximum;
            self
        }

        /// Sets the maximum number of controls in the compiled definition.
        pub fn max_controls(mut self, maximum: usize) -> Self {
            self.max_controls = maximum;
            self
        }

        /// Sets the maximum encoded bytes of one schema URI.
        pub fn max_uri_bytes(mut self, maximum: usize) -> Self {
            self.max_uri_bytes = maximum;
            self
        }

        /// Sets the maximum encoded bytes of one schema or instance pointer.
        pub fn max_pointer_bytes(mut self, maximum: usize) -> Self {
            self.max_pointer_bytes = maximum;
            self
        }

        /// Sets the maximum number of retained capability findings.
        pub fn max_capability_findings(mut self, maximum: usize) -> Self {
            self.max_capability_findings = maximum;
            self
        }

        /// Sets the maximum encoded bytes of the authored UI schema.
        pub fn max_ui_schema_bytes(mut self, maximum: usize) -> Self {
            self.max_ui_schema_bytes = maximum;
            self
        }

        /// Sets the maximum token count in the authored UI schema.
        pub fn max_ui_schema_tokens(mut self, maximum: usize) -> Self {
            self.max_ui_schema_tokens = maximum;
            self
        }

        /// Sets the maximum nesting depth of the authored UI schema.
        pub fn max_ui_schema_depth(mut self, maximum: usize) -> Self {
            self.max_ui_schema_depth = maximum;
            self
        }

        /// Sets the maximum aggregate JSON node count in the UI schema.
        pub fn max_ui_schema_nodes(mut self, maximum: usize) -> Self {
            self.max_ui_schema_nodes = maximum;
            self
        }

        /// Sets the maximum aggregate number of object members in the UI schema.
        pub fn max_ui_schema_members(mut self, maximum: usize) -> Self {
            self.max_ui_schema_members = maximum;
            self
        }

        /// Sets the maximum length of one UI-schema array or object.
        pub fn max_ui_schema_collection_length(mut self, maximum: usize) -> Self {
            self.max_ui_schema_collection_length = maximum;
            self
        }

        /// Sets the maximum encoded bytes of one UI-schema scalar.
        pub fn max_ui_schema_scalar_bytes(mut self, maximum: usize) -> Self {
            self.max_ui_schema_scalar_bytes = maximum;
            self
        }

        /// Sets the maximum number of distinct declared extension namespaces.
        pub fn max_extension_namespaces(mut self, maximum: usize) -> Self {
            self.max_extension_namespaces = maximum;
            self
        }

        /// Sets the maximum number of extension values across the UI schema.
        pub fn max_extension_occurrences(mut self, maximum: usize) -> Self {
            self.max_extension_occurrences = maximum;
            self
        }

        /// Sets the maximum encoded bytes of one extension namespace URI.
        pub fn max_extension_namespace_bytes(mut self, maximum: usize) -> Self {
            self.max_extension_namespace_bytes = maximum;
            self
        }

        /// Sets the maximum nesting depth of one extension value.
        pub fn max_extension_value_depth(mut self, maximum: usize) -> Self {
            self.max_extension_value_depth = maximum;
            self
        }

        /// Sets the maximum number of JSON values in one extension value.
        pub fn max_extension_value_nodes(mut self, maximum: usize) -> Self {
            self.max_extension_value_nodes = maximum;
            self
        }

        /// Sets the maximum encoded bytes of one extension value.
        pub fn max_extension_value_bytes(mut self, maximum: usize) -> Self {
            self.max_extension_value_bytes = maximum;
            self
        }

        pub(crate) fn extension_limits(&self) -> ExtensionLimits {
            ExtensionLimits {
                namespaces: self.max_extension_namespaces,
                occurrences: self.max_extension_occurrences,
                namespace_bytes: self.max_extension_namespace_bytes,
                value_depth: self.max_extension_value_depth,
                value_nodes: self.max_extension_value_nodes,
                value_bytes: self.max_extension_value_bytes,
            }
        }

        pub(crate) fn ui_schema_limits(&self) -> crate::limits::InputLimits {
            crate::limits::InputLimits {
                bytes: self.max_ui_schema_bytes,
                tokens: self.max_ui_schema_tokens,
                depth: self.max_ui_schema_depth,
                nodes: self.max_ui_schema_nodes,
                members: self.max_ui_schema_members,
                collection_length: self.max_ui_schema_collection_length,
                scalar_bytes: self.max_ui_schema_scalar_bytes,
            }
        }

        pub(crate) fn data_schema_limits(&self) -> crate::limits::DataSchemaLimits {
            crate::limits::DataSchemaLimits {
                bytes: self.max_data_schema_bytes,
                tokens: self.max_data_schema_tokens,
                depth: self.max_data_schema_depth,
                nodes: self.max_data_schema_nodes,
                members: self.max_data_schema_members,
                scalar_bytes: self.max_data_schema_scalar_bytes,
                resources: self.max_data_schema_resources,
                references: self.max_data_schema_references,
                traversal: self.max_data_schema_traversal,
                definition_nodes: self.max_definition_nodes,
                controls: self.max_controls,
                uri_bytes: self.max_uri_bytes,
                pointer_bytes: self.max_pointer_bytes,
                capability_findings: self.max_capability_findings,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct ExtensionLimits {
        pub(crate) namespaces: usize,
        pub(crate) occurrences: usize,
        pub(crate) namespace_bytes: usize,
        pub(crate) value_depth: usize,
        pub(crate) value_nodes: usize,
        pub(crate) value_bytes: usize,
    }

    impl ExtensionLimits {
        fn values(self) -> [usize; 6] {
            [
                self.namespaces,
                self.occurrences,
                self.namespace_bytes,
                self.value_depth,
                self.value_nodes,
                self.value_bytes,
            ]
        }
    }

    impl Default for CompilationProfile {
        fn default() -> Self {
            Self {
                max_data_schema_bytes: 2_097_152,
                max_data_schema_tokens: 262_144,
                max_data_schema_depth: 128,
                max_data_schema_nodes: 131_072,
                max_data_schema_members: 16_384,
                max_data_schema_scalar_bytes: 1_048_576,
                max_data_schema_resources: 256,
                max_data_schema_references: 16_384,
                max_data_schema_traversal: 131_072,
                max_definition_nodes: 16_384,
                max_controls: 8_192,
                max_uri_bytes: 8_192,
                max_pointer_bytes: 8_192,
                max_capability_findings: 1_024,
                max_ui_schema_bytes: 2_097_152,
                max_ui_schema_tokens: 262_144,
                max_ui_schema_depth: 128,
                max_ui_schema_nodes: 131_072,
                max_ui_schema_members: 65_536,
                max_ui_schema_collection_length: 16_384,
                max_ui_schema_scalar_bytes: 1_048_576,
                max_extension_namespaces: 32,
                max_extension_occurrences: 4_096,
                max_extension_namespace_bytes: 2_048,
                max_extension_value_depth: 32,
                max_extension_value_nodes: 16_384,
                max_extension_value_bytes: 1_048_576,
            }
        }
    }

    /// The lenient result of compiling a definition together with its capability
    /// diagnostics.
    ///
    /// Unlike strict compilation, this result may contain blocking findings. The
    /// definition remains useful for inspection but submission will remain blocked.
    pub struct FormAnalysis {
        definition: FormDefinition,
        report: CapabilityReport,
    }

    impl FormAnalysis {
        /// Borrows the analyzed definition, including unsupported placeholder nodes.
        pub fn definition(&self) -> &FormDefinition {
            &self.definition
        }

        /// Borrows the complete capability report for the analyzed definition.
        pub fn capability_report(&self) -> &CapabilityReport {
            &self.report
        }

        /// Consumes the analysis and returns its definition and report.
        pub fn into_parts(self) -> (FormDefinition, CapabilityReport) {
            (self.definition, self.report)
        }
    }

    /// Opaque identifier for one node in a compiled definition tree.
    ///
    /// It has meaning only with the definition that produced it; use
    /// [`FormDefinition::node`] rather than assuming an encoding.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct DefinitionNodeId(pub(crate) u64);

    /// A borrowed inspection view of one node in an immutable definition tree.
    ///
    /// This describes compiled structure and presentation, not the current state
    /// of any [`Form`] instance.
    #[derive(Clone, Copy)]
    pub struct DefinitionNodeView<'a> {
        node: &'a DefinitionNode,
    }

    impl<'a> DefinitionNodeView<'a> {
        /// Returns this node's opaque, definition-scoped identifier.
        pub fn id(&self) -> DefinitionNodeId {
            self.node.id
        }

        /// Returns the explicit UI-schema element ID, if one was authored.
        pub fn authored_id(&self) -> Option<&'a str> {
            self.node.authored_id.as_deref()
        }

        /// Returns this node's layout or control role.
        ///
        /// This is distinct from [`Self::semantic_kind`], which describes the
        /// bound data shape rather than the node's place in the definition tree.
        pub fn kind(&self) -> DefinitionNodeKind {
            self.node.kind
        }

        /// Returns the compiled data semantics, when this node represents data.
        pub fn semantic_kind(&self) -> Option<SemanticKind> {
            self.node.semantic_kind
        }

        /// Returns the node's compiled control binding, if it binds form data.
        ///
        /// Bindings are root-relative except inside an array's inline item
        /// template, where they are relative to the current item. The control for
        /// a scalar item therefore has the empty pointer.
        pub fn binding(&self) -> Option<&'a JsonPointer> {
            self.node.binding.as_ref()
        }

        /// Returns the compiled literal label fallback.
        ///
        /// This may come from data-schema `title` or an authored text reference;
        /// consult [`Self::label_reference`] to preserve localization metadata.
        pub fn label(&self) -> &'a str {
            &self.node.label
        }

        /// Returns the authored label reference, including its localization key.
        ///
        /// Inherited data-schema titles have no structured UI-schema reference.
        pub fn label_reference(&self) -> Option<&'a ui::v1::TextReference> {
            self.node.label_reference.as_ref()
        }

        /// Returns whether a renderer should present the compiled label.
        pub fn is_label_visible(&self) -> bool {
            self.node.label_visible
        }

        /// Returns the compiled literal help fallback, if any.
        ///
        /// This may come from data-schema `description` or an authored text
        /// reference; consult [`Self::help_reference`] for localization metadata.
        pub fn help(&self) -> Option<&'a str> {
            self.node.help.as_deref()
        }

        /// Returns the authored help reference, including its localization key.
        pub fn help_reference(&self) -> Option<&'a ui::v1::TextReference> {
            self.node.help_reference.as_ref()
        }

        /// Returns the authored reference used to label each array item.
        pub fn item_label_reference(&self) -> Option<&'a ui::v1::TextReference> {
            self.node.item_label_reference.as_ref()
        }

        /// Returns the authored plain-text reference for a [`DefinitionNodeKind::Text`] node.
        pub fn text(&self) -> Option<&'a ui::v1::TextReference> {
            self.node.text.as_ref()
        }

        /// Returns annotations collected from the applicable data schemas.
        ///
        /// Annotations remain descriptive except where the form runtime explicitly
        /// gives them authority, such as `readOnly` and `writeOnly`.
        pub fn data_schema_annotations(&self) -> &'a DataSchemaAnnotations {
            &self.node.data_schema_annotations
        }

        /// Returns the compiled value used to create this absent value or array item.
        ///
        /// Seeds are applied only by explicit creation operations; they do not
        /// mutate initial form data and are still validated normally after use.
        pub fn creation_seed(&self) -> Option<&'a Value> {
            self.node.creation_seed.as_ref()
        }

        /// Returns whether the bound object member is required by the data schema.
        ///
        /// Requiredness prevents removal; it does not imply that current form data
        /// contains the member or that the value cannot be null.
        pub fn is_required(&self) -> bool {
            self.node.required
        }

        /// Returns whether the bound scalar accepts JSON null.
        ///
        /// Null acceptance is independent of [`Self::is_required`], which concerns
        /// whether an object member may be absent.
        pub fn accepts_null(&self) -> bool {
            self.node.accepts_null
        }

        /// Iterates the compiled scalar choices in deterministic display order.
        pub fn choice_options(&self) -> impl Iterator<Item = ChoiceOptionView<'a>> + 'a {
            self.node
                .choice_options
                .iter()
                .map(|option| ChoiceOptionView { option })
        }

        /// Returns whether the choices represent a user-selectable set.
        ///
        /// A fixed scalar constant can expose one option for display while not
        /// allowing selection.
        pub fn is_choice_selectable(&self) -> bool {
            self.node.choice_selectable
        }

        /// Iterates child definition IDs in presentation order.
        ///
        /// An array control exposes its inline item template here once; runtime
        /// form-tree traversal instantiates that template once per current item.
        pub fn children(&self) -> impl Iterator<Item = DefinitionNodeId> + 'a {
            self.node.children.iter().copied()
        }

        /// Returns responsive column spans for a grid-cell node.
        pub fn grid_spans(&self) -> Option<GridSpans> {
            self.node.grid_spans
        }

        /// Iterates the data-schema locations that contributed this node's semantics.
        ///
        /// These resource-and-pointer pairs record schema provenance and are not
        /// form-data bindings.
        pub fn schema_locations(&self) -> impl Iterator<Item = &'a SchemaLocation> {
            self.node.schema_locations.iter()
        }

        pub(crate) fn is_item_template(&self) -> bool {
            self.node.owning_array.is_some()
        }

        /// Returns the exact widget symbol requested by the authored UI schema.
        ///
        /// The core preserves the request but does not resolve it to a renderer.
        pub fn widget(&self) -> Option<&'a WidgetSymbol> {
            self.node.widget.as_ref()
        }

        /// Iterates opaque UI-schema extension values by exact namespace URI.
        ///
        /// Core preserves these bounded values in deterministic namespace order;
        /// extensions do not alter core binding, validation, or form behavior.
        pub fn extensions(
            &self,
        ) -> impl ExactSizeIterator<Item = (&'a ExtensionNamespace, &'a Value)> {
            self.node.extensions.iter()
        }
    }

    /// A definition node's layout or control role.
    ///
    /// This classifies definition-tree structure and is deliberately separate
    /// from [`SemanticKind`], which classifies represented data.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum DefinitionNodeKind {
        /// A form-data control.
        Control,
        /// A structural layout synthesized from the data schema.
        AutoGeneratedLayout,
        /// An authored ordered vertical layout.
        Stack,
        /// An authored responsive grid.
        Grid,
        /// A cell within a responsive grid.
        GridCell,
        /// An authored titled grouping.
        Group,
        /// An authored tab container.
        Tabs,
        /// One authored titled panel within tabs.
        TabPanel,
        /// Authored escaped plain text, not HTML or Markdown.
        Text,
        /// A visible placeholder for a region the compiler cannot represent.
        Unsupported,
    }

    /// Effective compact and wide spans for a cell in a 12-column grid.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GridSpans {
        compact: u8,
        wide: u8,
    }

    impl GridSpans {
        /// Returns the compact-layout span in columns.
        pub fn compact(self) -> u8 {
            self.compact
        }

        /// Returns the wide-layout span in columns.
        pub fn wide(self) -> u8 {
            self.wide
        }
    }

    /// The compiled data shape represented by a definition node.
    ///
    /// Unlike [`DefinitionNodeKind`], this does not describe layout or whether the
    /// node was authored or generated.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum SemanticKind {
        /// A JSON string scalar.
        String,
        /// A JSON number scalar.
        Number,
        /// A mathematically integral JSON number scalar.
        Integer,
        /// A JSON boolean scalar.
        Boolean,
        /// The JSON null value.
        Null,
        /// A finite set of compiled JSON scalar choices.
        Choice,
        /// An object with a finite, statically named editable projection.
        FixedObject,
        /// An array whose items share one inline definition template.
        HomogeneousArray,
    }

    /// One owned compiled scalar choice and its display label.
    ///
    /// Definition inspection exposes borrowed [`ChoiceOptionView`] values.
    #[derive(Debug, Clone, PartialEq)]
    pub struct ChoiceOption {
        value: Value,
        label: String,
    }

    /// A borrowed view of one compiled scalar choice.
    #[derive(Clone, Copy)]
    pub struct ChoiceOptionView<'a> {
        option: &'a ChoiceOption,
    }

    impl<'a> ChoiceOptionView<'a> {
        /// Returns the exact JSON scalar written when this option is selected.
        pub fn value(self) -> &'a Value {
            &self.option.value
        }

        /// Returns the option's compiled plain-text display label.
        pub fn label(self) -> &'a str {
            &self.option.label
        }
    }

    /// Opaque, versioned identity of a definition's observable semantics.
    ///
    /// It is suitable for equality checks and in-process cache keys. No stability
    /// is promised across crate versions, so it is not a persistent content ID.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    pub struct DefinitionFingerprint([u8; 32]);

    impl fmt::Debug for DefinitionFingerprint {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("DefinitionFingerprint(..)")
        }
    }

    /// Failure to strictly compile a form definition.
    ///
    /// Input, resource, and qualification errors mean the source cannot be
    /// compiled. [`Self::Capability`] contains a report whose blocking findings
    /// made an otherwise analyzed definition unacceptable to strict compilation.
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum CompileError {
        Input(InputError),
        Resource(ResourceError),
        Qualification(QualificationError),
        Capability(CapabilityReport),
    }

    impl CompileError {
        fn engine(error: engine::CompileError, root_uri: &RetrievalUri) -> Self {
            match error {
                engine::CompileError::UnsupportedReference(reference) => {
                    Self::Capability(CapabilityReport::blocking(
                        "unsupported-reference",
                        JsonPointer::parse("").expect("the root pointer is valid"),
                        SchemaLocation::new(
                            root_uri.clone(),
                            JsonPointer::parse("/$ref")
                                .expect("the root reference pointer is valid"),
                        ),
                        serde_json::json!({ "reference": reference }),
                    ))
                }
                engine::CompileError::ResourceLimit(error) => {
                    Self::Resource(ResourceError::Limit(error))
                }
                engine::CompileError::MissingProperties => {
                    Self::Input(InputError::InvalidDataSchema)
                }
            }
        }
    }

    impl fmt::Display for CompileError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Input(error) => error.fmt(formatter),
                Self::Resource(error) => error.fmt(formatter),
                Self::Qualification(error) => error.fmt(formatter),
                Self::Capability(report) => {
                    if let Some(finding) =
                        report.findings.iter().find(|finding| finding.is_blocking())
                    {
                        write!(
                            formatter,
                            "data schema cannot be represented: {}",
                            finding.code
                        )
                    } else {
                        formatter.write_str("data schema cannot be represented")
                    }
                }
            }
        }
    }

    impl Error for CompileError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::Input(error) => Some(error),
                Self::Resource(error) => Some(error),
                Self::Qualification(error) => Some(error),
                Self::Capability(_) => None,
            }
        }
    }

    /// Invalid or unsupported caller input detected before capability analysis.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum InputError {
        InvalidDataSchema,
        InvalidUiSchema(UiSchemaInputError),
    }

    impl fmt::Display for InputError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidDataSchema => formatter.write_str("invalid data schema"),
                Self::InvalidUiSchema(error) => write!(
                    formatter,
                    "invalid or unsupported UI schema at {}: {:?}",
                    error.location, error.kind
                ),
            }
        }
    }

    impl Error for InputError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::InvalidUiSchema(error) => Some(error),
                Self::InvalidDataSchema => None,
            }
        }
    }

    /// A UI-schema input failure paired with its JSON location and stable category.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UiSchemaInputError {
        location: JsonPointer,
        kind: UiSchemaInputErrorKind,
    }

    impl UiSchemaInputError {
        fn new(location: impl Into<String>, kind: UiSchemaInputErrorKind) -> Self {
            Self {
                location: JsonPointer::parse(location.into())
                    .expect("compiler-owned UI-schema locations are valid JSON Pointers"),
                kind,
            }
        }

        /// Returns the location in the authored UI schema.
        pub fn location(&self) -> &JsonPointer {
            &self.location
        }

        /// Returns the machine-readable category of the failure.
        pub fn kind(&self) -> UiSchemaInputErrorKind {
            self.kind
        }
    }

    impl fmt::Display for UiSchemaInputError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "UI schema {:?} at {}", self.kind, self.location)
        }
    }

    impl Error for UiSchemaInputError {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum UiSchemaInputErrorKind {
        UnsupportedField,
        InvalidBindingOrigin,
        UnknownBinding,
        DuplicateBinding,
        InvalidPropertySelection,
        DuplicateElementId,
        DuplicateRequiredExtension,
        MissingRequiredExtension,
        ResourceLimit,
    }

    impl fmt::Display for UiSchemaInputErrorKind {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(match self {
                Self::UnsupportedField => "unsupported field",
                Self::InvalidBindingOrigin => "invalid binding origin",
                Self::UnknownBinding => "unknown binding",
                Self::DuplicateBinding => "duplicate binding",
                Self::InvalidPropertySelection => "invalid property selection",
                Self::DuplicateElementId => "duplicate element ID",
                Self::DuplicateRequiredExtension => "duplicate required extension",
                Self::MissingRequiredExtension => "missing required extension",
                Self::ResourceLimit => "resource limit",
            })
        }
    }

    /// Failure to produce even a lenient [`FormAnalysis`].
    ///
    /// A capability report appears here only when the compiler cannot construct a
    /// usable definition; ordinary blocking findings are returned successfully by
    /// [`FormCompiler::analyze`].
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum AnalysisError {
        Input(InputError),
        Resource(ResourceError),
        Qualification(QualificationError),
        Capability(CapabilityReport),
    }

    /// Failure to prepare or bound the caller-supplied schema resource graph.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ResourceError {
        InvalidResourceGraph,
        Limit(CompilationLimitError),
    }

    impl fmt::Display for AnalysisError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Input(error) => error.fmt(formatter),
                Self::Resource(error) => error.fmt(formatter),
                Self::Qualification(error) => error.fmt(formatter),
                Self::Capability(_) => formatter.write_str("data schema cannot be analyzed"),
            }
        }
    }

    impl fmt::Display for ResourceError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidResourceGraph => {
                    formatter.write_str("invalid or incomplete caller-supplied resource graph")
                }
                Self::Limit(error) => error.fmt(formatter),
            }
        }
    }

    impl Error for AnalysisError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::Input(error) => Some(error),
                Self::Resource(error) => Some(error),
                Self::Qualification(error) => Some(error),
                Self::Capability(_) => None,
            }
        }
    }

    impl Error for ResourceError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::Limit(error) => Some(error),
                Self::InvalidResourceGraph => None,
            }
        }
    }

    /// Stage at which a runtime form-state or untrusted-input resource limit was enforced.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ResourceLimitPhase {
        Construction,
        Operation,
    }

    /// A deterministic resource-bound violation with its phase and location.
    ///
    /// `observed` is the rejected value and is always greater than `maximum`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ResourceLimitError {
        phase: ResourceLimitPhase,
        dimension: &'static str,
        maximum: usize,
        observed: usize,
        path: JsonPointer,
    }

    impl ResourceLimitError {
        pub(crate) fn new(
            phase: ResourceLimitPhase,
            dimension: &'static str,
            maximum: usize,
            observed: usize,
            path: JsonPointer,
        ) -> Self {
            Self {
                phase,
                dimension,
                maximum,
                observed,
                path,
            }
        }

        /// Returns whether the limit failed during construction or an operation.
        pub fn phase(&self) -> ResourceLimitPhase {
            self.phase
        }

        /// Returns the stable machine-readable name of the bounded dimension.
        ///
        /// Current values are:
        ///
        /// - `bytes`, `tokens`, `depth`, `nodes`, `members`, `collection_length`,
        ///   and `scalar_bytes` for bounded JSON or operation inputs;
        /// - `extension_namespaces`, `extension_occurrences`,
        ///   `extension_namespace_bytes`, `extension_value_depth`,
        ///   `extension_value_nodes`, and `extension_value_bytes` for authored
        ///   UI-schema extensions;
        /// - `repeated_items` and `form_tree_nodes` for instantiated form state;
        /// - `incoming_external_findings`, `incoming_external_finding_bytes`,
        ///   `active_external_findings`, `active_external_finding_bytes`,
        ///   `external_finding_parameter_depth`, `external_finding_parameter_nodes`,
        ///   `external_finding_parameter_collection_length`, and
        ///   `external_finding_parameter_scalar_bytes` for host findings;
        /// - `edit_buffer_bytes`, `active_edit_buffers`, and
        ///   `total_edit_buffer_bytes` for textual edit state; and
        /// - `host_operations_per_transaction` for host transactions.
        ///
        /// Existing names retain their meaning within a compatible crate release.
        /// Future compatible releases may add names, so consumers must handle
        /// unknown values.
        pub fn dimension(&self) -> &'static str {
            self.dimension
        }

        /// Returns the configured inclusive maximum.
        pub fn maximum(&self) -> usize {
            self.maximum
        }

        /// Returns the value observed when the operation was rejected.
        pub fn observed(&self) -> usize {
            self.observed
        }

        /// Returns the nearest relevant form-data, JSON-input, or UI-schema location.
        pub fn path(&self) -> &JsonPointer {
            &self.path
        }
    }

    impl fmt::Display for ResourceLimitError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "observed {} exceeds {} limit {} at {} during {:?}",
                self.observed, self.dimension, self.maximum, self.path, self.phase
            )
        }
    }

    impl Error for ResourceLimitError {}

    /// Deterministically ordered diagnostics about semantics the runtime cannot
    /// represent completely.
    ///
    /// Warnings preserve operation; blocking findings make strict compilation or
    /// submission unavailable.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CapabilityReport {
        findings: Vec<CapabilityFinding>,
    }

    impl CapabilityReport {
        fn blocking(
            code: &'static str,
            instance_location: JsonPointer,
            keyword_location: SchemaLocation,
            parameters: Value,
        ) -> Self {
            Self {
                findings: vec![CapabilityFinding {
                    code,
                    instance_location,
                    keyword_location,
                    parameters,
                    severity: CapabilitySeverity::Blocking,
                }],
            }
        }

        /// Iterates all findings in compiler-defined deterministic order.
        pub fn findings(&self) -> impl Iterator<Item = &CapabilityFinding> {
            self.findings.iter()
        }

        /// Returns whether any finding prevents strict use of the definition.
        pub fn is_blocking(&self) -> bool {
            self.findings.iter().any(CapabilityFinding::is_blocking)
        }
    }

    /// One structured capability diagnostic tied to instance and schema locations.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CapabilityFinding {
        code: &'static str,
        instance_location: JsonPointer,
        keyword_location: SchemaLocation,
        parameters: Value,
        severity: CapabilitySeverity,
    }

    /// Whether a capability finding is advisory or prevents strict operation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum CapabilitySeverity {
        Warning,
        Blocking,
    }

    impl CapabilityFinding {
        /// Returns the stable machine-readable diagnostic code.
        ///
        /// The current code and [`Self::parameters`] contracts are:
        ///
        /// - `annotation.conflict`: `{"keyword":"title"|"description","values":[string,...]}`;
        /// - `applicator.additional-properties.dynamic-map` and
        ///   `applicator.additional-properties.open`: `{"implicit":boolean}`;
        /// - `applicator.all-of.ambiguous`:
        ///   `{"branchCount":integer,"reason":"incompatible-kind"}`;
        /// - `applicator.any-of` and `applicator.one-of`: `{"branchCount":integer}`;
        /// - `applicator.prefix-items`: `{"itemCount":integer}`;
        /// - `applicator.properties.conditional`: `{"branch":"then"|"else"}`;
        /// - `structure.array.homogeneous-scalar`: `{}` or
        ///   `{"reason":"missing-items"}`;
        /// - `unsupported-reference`: `{"reference":string}`;
        /// - all other current codes have `{}` parameters.
        ///
        /// The other current codes are `applicator.additional-properties.schema-projection`,
        /// `applicator.all-of.conditional`, `applicator.dependent-schemas.structural`,
        /// `applicator.else.structural`, `applicator.if.structural`,
        /// `applicator.not.shape`, `applicator.pattern-properties.fixed-projection`,
        /// `applicator.pattern-properties.shape`, `applicator.then.structural`,
        /// `core.boolean.unconstrained`, `core.dynamic-reference.shape`,
        /// `structure.array.nested`, `structure.recursive.projection`,
        /// `structure.root.array`, `structure.root.scalar`, `unevaluated.items.shape`,
        /// `unevaluated.properties.shape`, `validation.const.conflicting`,
        /// `validation.const.incompatible`, `validation.const.structured`,
        /// `validation.enum.conditional`, `validation.enum.incompatible`,
        /// `validation.enum.structured`, `validation.type.ambiguous`, and
        /// `validation.type.unconstrained`.
        ///
        /// Existing codes and parameter fields retain their meaning within a
        /// compatible crate release. Future compatible releases may add codes or
        /// fields, so consumers must handle unknown values and ignore unknown fields.
        pub fn code(&self) -> &str {
            self.code
        }

        /// Returns the form-data location affected by the unsupported semantics.
        pub fn instance_location(&self) -> &JsonPointer {
            &self.instance_location
        }

        /// Returns the schema resource and keyword location that caused the finding.
        pub fn keyword_location(&self) -> &SchemaLocation {
            &self.keyword_location
        }

        /// Returns code-specific structured diagnostic parameters.
        pub fn parameters(&self) -> &Value {
            &self.parameters
        }

        /// Returns the finding's effect on strict compilation and submission.
        pub fn severity(&self) -> CapabilitySeverity {
            self.severity
        }

        /// Returns whether this finding prevents strict use of the definition.
        pub fn is_blocking(&self) -> bool {
            self.severity == CapabilitySeverity::Blocking
        }
    }

    fn fingerprint(
        nodes: &[DefinitionNode],
        required_extensions: &[ExtensionNamespace],
        capability_report: &CapabilityReport,
        engine: &engine::FormDefinition,
        root_uri: &RetrievalUri,
        profile: &CompilationProfile,
    ) -> DefinitionFingerprint {
        let mut hasher = Sha256::new();
        hasher.update(b"schemaform-definition-v18\0");
        hash_bytes(&mut hasher, engine.fingerprint_bytes());
        hash_bytes(&mut hasher, root_uri.as_str().as_bytes());
        for maximum in profile.ui_schema_limits().values() {
            hasher.update((maximum as u64).to_be_bytes());
        }
        for maximum in profile.data_schema_limits().values() {
            hasher.update((maximum as u64).to_be_bytes());
        }
        for maximum in profile.extension_limits().values() {
            hasher.update((maximum as u64).to_be_bytes());
        }
        hasher.update((required_extensions.len() as u64).to_be_bytes());
        for namespace in required_extensions {
            hash_bytes(&mut hasher, namespace.as_str().as_bytes());
        }
        for node in nodes {
            hasher.update(node.id.0.to_be_bytes());
            if let Some(authored_id) = &node.authored_id {
                hasher.update([1]);
                hash_bytes(&mut hasher, authored_id.as_bytes());
            } else {
                hasher.update([0]);
            }
            hasher.update([match node.kind {
                DefinitionNodeKind::Control => 0,
                DefinitionNodeKind::AutoGeneratedLayout => 1,
                DefinitionNodeKind::Stack => 2,
                DefinitionNodeKind::Grid => 3,
                DefinitionNodeKind::GridCell => 4,
                DefinitionNodeKind::Group => 5,
                DefinitionNodeKind::Tabs => 6,
                DefinitionNodeKind::TabPanel => 7,
                DefinitionNodeKind::Text => 8,
                DefinitionNodeKind::Unsupported => 9,
            }]);
            if let Some(binding) = &node.binding {
                hash_bytes(&mut hasher, binding.as_str().as_bytes());
            } else {
                hash_bytes(&mut hasher, &[]);
            }
            if let Some(widget) = &node.widget {
                hasher.update([1]);
                hash_bytes(&mut hasher, widget.as_str().as_bytes());
            } else {
                hasher.update([0]);
            }
            hasher.update((node.extensions.len() as u64).to_be_bytes());
            for (namespace, value) in &node.extensions {
                hash_bytes(&mut hasher, namespace.as_str().as_bytes());
                hash_bytes(&mut hasher, &engine::semantic_json_fingerprint(value));
            }
            hash_bytes(&mut hasher, node.label.as_bytes());
            hasher.update([node.label_visible as u8]);
            hash_text_reference(&mut hasher, node.label_reference.as_ref());
            if let Some(help) = &node.help {
                hasher.update([1]);
                hash_bytes(&mut hasher, help.as_bytes());
            } else {
                hasher.update([0]);
            }
            hash_text_reference(&mut hasher, node.help_reference.as_ref());
            hash_text_reference(&mut hasher, node.item_label_reference.as_ref());
            hash_text_reference(&mut hasher, node.text.as_ref());
            hash_string_annotations(&mut hasher, node.data_schema_annotations.formats());
            hash_value_annotations(&mut hasher, node.data_schema_annotations.defaults());
            hasher.update([node.data_schema_annotations.is_deprecated() as u8]);
            hasher.update([node.data_schema_annotations.is_read_only() as u8]);
            hasher.update([node.data_schema_annotations.is_write_only() as u8]);
            hash_value_annotations(&mut hasher, node.data_schema_annotations.examples());
            hash_string_annotations(
                &mut hasher,
                node.data_schema_annotations.content_encodings(),
            );
            hash_string_annotations(
                &mut hasher,
                node.data_schema_annotations.content_media_types(),
            );
            hash_value_annotations(&mut hasher, node.data_schema_annotations.content_schemas());
            if let Some(seed) = &node.creation_seed {
                hasher.update([1]);
                hasher.update(engine::semantic_json_fingerprint(seed));
            } else {
                hasher.update([0]);
            }
            hasher.update([node.required as u8]);
            hasher.update([node.choice_selectable as u8]);
            if let Some(owner) = node.owning_array {
                hasher.update([1]);
                hasher.update(owner.0.to_be_bytes());
            } else {
                hasher.update([0]);
            }
            if let Some(spans) = node.grid_spans {
                hasher.update([1, spans.compact, spans.wide]);
            } else {
                hasher.update([0]);
            }
            hasher.update((node.choice_options.len() as u64).to_be_bytes());
            for option in &node.choice_options {
                hash_bytes(&mut hasher, option.label.as_bytes());
                hash_bytes(&mut hasher, option.value.to_string().as_bytes());
            }
            hasher.update([match node.semantic_kind {
                None => 0,
                Some(SemanticKind::String) => 1,
                Some(SemanticKind::Integer) => 2,
                Some(SemanticKind::Number) => 3,
                Some(SemanticKind::Boolean) => 4,
                Some(SemanticKind::Null) => 5,
                Some(SemanticKind::Choice) => 6,
                Some(SemanticKind::FixedObject) => 7,
                Some(SemanticKind::HomogeneousArray) => 8,
            }]);
            hasher.update((node.schema_locations.len() as u64).to_be_bytes());
            for location in &node.schema_locations {
                hash_bytes(&mut hasher, location.resource().as_str().as_bytes());
                hash_bytes(&mut hasher, location.pointer().as_str().as_bytes());
            }
            hasher.update((node.children.len() as u64).to_be_bytes());
            for child in &node.children {
                hasher.update(child.0.to_be_bytes());
            }
        }
        for finding in capability_report.findings() {
            hash_bytes(&mut hasher, finding.code().as_bytes());
            hash_bytes(&mut hasher, finding.instance_location().as_str().as_bytes());
            hash_bytes(
                &mut hasher,
                finding.keyword_location().resource().as_str().as_bytes(),
            );
            hash_bytes(
                &mut hasher,
                finding.keyword_location().pointer().as_str().as_bytes(),
            );
            hash_bytes(&mut hasher, finding.parameters().to_string().as_bytes());
            hasher.update([finding.is_blocking() as u8]);
        }
        DefinitionFingerprint(hasher.finalize().into())
    }

    fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }

    fn hash_text_reference(hasher: &mut Sha256, reference: Option<&ui::v1::TextReference>) {
        let Some(reference) = reference else {
            hasher.update([0]);
            return;
        };
        hasher.update([1]);
        hash_bytes(hasher, reference.fallback().as_bytes());
        if let Some(key) = reference.key() {
            hasher.update([1]);
            hash_bytes(hasher, key.as_bytes());
        } else {
            hasher.update([0]);
        }
    }

    fn hash_string_annotations<'a>(hasher: &mut Sha256, values: impl Iterator<Item = &'a str>) {
        let values = values.collect::<Vec<_>>();
        hasher.update((values.len() as u64).to_be_bytes());
        for value in values {
            hash_bytes(hasher, value.as_bytes());
        }
    }

    fn hash_value_annotations<'a>(hasher: &mut Sha256, values: impl Iterator<Item = &'a Value>) {
        let values = values.collect::<Vec<_>>();
        hasher.update((values.len() as u64).to_be_bytes());
        for value in values {
            hasher.update(engine::semantic_json_fingerprint(value));
        }
    }

    fn scalar_choice_label(value: &Value) -> String {
        match value {
            Value::String(value) => value.clone(),
            Value::Null | Value::Bool(_) | Value::Number(_) => value.to_string(),
            Value::Array(_) | Value::Object(_) => {
                unreachable!("compiled choices contain only JSON scalars")
            }
        }
    }

    fn control_semantic_kind(control: engine::ControlDefinitionView<'_>) -> SemanticKind {
        if control.is_string() {
            SemanticKind::String
        } else if control.is_number() {
            SemanticKind::Number
        } else if control.is_boolean() {
            SemanticKind::Boolean
        } else if control.is_null() {
            SemanticKind::Null
        } else if control.is_constant() || control.is_choice() {
            SemanticKind::Choice
        } else {
            SemanticKind::Integer
        }
    }

    fn collect_schema_locations<'a>(
        locations: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Vec<SchemaLocation> {
        locations
            .map(|(resource, pointer)| {
                SchemaLocation::new(
                    RetrievalUri::parse(resource)
                        .expect("compiled engine resources are absolute URIs"),
                    JsonPointer::parse(pointer)
                        .expect("compiled engine schema locations are JSON Pointers"),
                )
            })
            .collect()
    }
}

pub mod form {
    use std::{
        collections::{HashMap, HashSet},
        convert::Infallible,
        error::Error,
        fmt,
        num::NonZeroUsize,
        sync::atomic::{AtomicU64, Ordering},
    };

    use jsonptr::PointerBuf;
    use serde_json::Value;

    use crate::{
        address::JsonPointer,
        definition::{
            DataSchemaAnnotations, DefinitionFingerprint, DefinitionNodeId, DefinitionNodeView,
            FormDefinition, ResourceLimitError, ResourceLimitPhase, SemanticKind,
        },
        engine,
        finding::{ExternalFinding, ExternalFindingBatch, FindingView, ValidationFinding},
        json::FormDataLimits,
        validation,
    };

    static NEXT_FORM_ID: AtomicU64 = AtomicU64::new(1);
    const DEFAULT_MAX_ACTIVE_EXTERNAL_FINDINGS: usize = 1024;
    const DEFAULT_MAX_ACTIVE_EXTERNAL_FINDING_BYTES: usize = 1024 * 1024;
    const DEFAULT_INCOMING_EXTERNAL_FINDING_MULTIPLIER: usize = 4;
    const DEFAULT_MAX_EXTERNAL_PARAMETER_DEPTH: usize = 32;
    const DEFAULT_MAX_EXTERNAL_PARAMETER_NODES: usize = 4096;
    const DEFAULT_MAX_EXTERNAL_PARAMETER_COLLECTION_LENGTH: usize = 1024;
    const DEFAULT_MAX_EXTERNAL_PARAMETER_SCALAR_BYTES: usize = 64 * 1024;

    /// Owned synchronous form data, interaction state, findings, and revisions.
    ///
    /// Mutating methods return a [`Transition`] describing observable changes.
    /// User and host operations enforce configured limits before committing; a
    /// returned error leaves form-owned data and interaction state unchanged.
    pub struct Form {
        definition: FormDefinition,
        engine: engine::Form,
        validation: validation::Outcome,
        external_finding_batches: Vec<ExternalFindingBatch>,
        external_finding_limits: ExternalFindingLimits,
        limits: FormDataLimits,
        finding_visibility: FindingVisibilityPolicy,
        id: u64,
    }

    #[derive(Clone, PartialEq, Eq)]
    struct NodeObservation {
        binding: Option<JsonPointer>,
        children: Vec<InstanceIdentity>,
        current_data: Option<Value>,
        edit_buffer: Option<String>,
        allowed_operations: AllowedOperations,
        value_state: Option<ScalarValueState>,
        touched: bool,
        dirty: bool,
        findings: Vec<FindingObservation>,
    }

    #[derive(Clone, PartialEq, Eq)]
    enum FindingObservation {
        Validation(ValidationFinding),
        ValidationFindingsTruncated(usize),
        Indeterminate(IndeterminateReason),
        Capability(crate::definition::CapabilityFinding),
        External(String, ExternalFinding),
        Parse(ParseBlockerKind),
    }

    impl Form {
        pub(crate) fn new(
            definition: FormDefinition,
            form_data: Value,
            limits: FormDataLimits,
            finding_visibility: FindingVisibilityPolicy,
            external_finding_limits: ExternalFindingLimits,
        ) -> Result<Self, FormBuildError> {
            if !form_data.is_object() {
                return Err(FormBuildError::FormDataMustBeObject);
            }
            crate::limits::check_input_value(&form_data, limits.input_limits()).map_err(
                |error| {
                    FormBuildError::ResourceLimit(ResourceLimitError::new(
                        ResourceLimitPhase::Construction,
                        error.dimension,
                        error.maximum,
                        error.observed,
                        JsonPointer::parse(error.pointer)
                            .expect("input limit scans produce valid JSON Pointers"),
                    ))
                },
            )?;
            check_form_tree(
                &definition,
                &form_data,
                limits,
                ResourceLimitPhase::Construction,
            )
            .map_err(FormBuildError::ResourceLimit)?;
            let engine = definition
                .inner
                .engine
                .create_form(form_data)
                .map_err(|_| FormBuildError::FormDataMustBeObject)?;
            let validation = definition.inner.validator.validate_with_limits(
                engine.form_data(),
                limits.retained_validation_findings(),
                limits.validation_parameter_bytes(),
            );
            let id = NEXT_FORM_ID.fetch_add(1, Ordering::Relaxed);
            Ok(Self {
                definition,
                engine,
                validation,
                external_finding_batches: Vec::new(),
                external_finding_limits,
                limits,
                finding_visibility,
                id,
            })
        }

        /// Borrows the immutable definition shared by this form.
        pub fn definition(&self) -> &FormDefinition {
            &self.definition
        }

        /// Returns a copyable read-only view of current form-wide state.
        pub fn view(&self) -> FormView<'_> {
            FormView { form: self }
        }

        /// Resolves a form-scoped instance identity to its current node.
        ///
        /// Returns `None` for identities from another form, removed array items,
        /// definition-only templates, or otherwise unknown instances.
        pub fn node(&self, identity: InstanceIdentity) -> Option<NodeView<'_>> {
            if identity.form != self.id {
                return None;
            }
            let definition_id = DefinitionNodeId(identity.node);
            let definition = self.definition.node(definition_id)?;
            match identity.item {
                None if definition.is_item_template() => return None,
                Some(item) => {
                    if item.form != self.id {
                        return None;
                    }
                    if !definition.is_item_template() {
                        return None;
                    }
                    let array = self.definition.array_for_template(definition_id)?;
                    let binding = self.definition.node(array)?.binding()?;
                    self.engine
                        .array_item_binding(binding.as_str(), item.local)?;
                }
                None => {}
            }
            Some(NodeView {
                form: self,
                identity,
            })
        }

        /// Borrows committed canonical form data.
        ///
        /// Unparseable numeric edit buffers are interaction state and do not
        /// replace this value until they become parseable.
        pub fn form_data(&self) -> &Value {
            self.engine.form_data()
        }

        /// Starts a facade for schema-aware end-user operations.
        ///
        /// The mutable borrow prevents interleaving user and host mutations.
        pub fn user(&mut self) -> UserActions<'_> {
            UserActions { form: self }
        }

        /// Applies a batch of host-authored JSON and array operations atomically.
        ///
        /// The closure edits an isolated candidate. If it returns `Err`, queues an
        /// invalid operation, exceeds limits, or produces non-object form data,
        /// nothing is committed. A successful data change revalidates and clears
        /// all external finding batches. Schema-invalid data is permitted provided
        /// structural and resource invariants hold.
        pub fn try_transact<E, F>(
            &mut self,
            transaction: F,
        ) -> Result<Transition, TransactionError<E>>
        where
            F: FnOnce(&mut HostTransaction<'_>) -> Result<(), E>,
        {
            let before = self.revisions();
            let identities_before = self.instance_identities();
            let findings_before = self.visible_validation_findings_by_node();
            let mut candidate = self.form_data().clone();
            let mut valid = true;
            let mut writes = Vec::new();
            let mut item_writes = Vec::new();
            let mut array_topologies = self.host_array_topologies();
            let array_topologies_before = array_topologies.clone();
            let mut operation_count = 0;
            let mut commit_error = None;
            transaction(&mut HostTransaction {
                form_id: self.id,
                candidate: &mut candidate,
                valid: &mut valid,
                writes: &mut writes,
                item_writes: &mut item_writes,
                array_topologies: &mut array_topologies,
                operation_count: &mut operation_count,
                maximum_operations: self.limits.host_operations_per_transaction(),
                commit_error: &mut commit_error,
            })
            .map_err(TransactionError::Closure)?;
            if let Some(error) = commit_error {
                return Err(TransactionError::Commit(error));
            }
            if !valid || !candidate.is_object() {
                return Err(TransactionError::Commit(HostCommitError::InvalidOperation));
            }
            check_runtime_form_data(&self.definition, &candidate, self.limits)
                .map_err(|error| TransactionError::Commit(HostCommitError::ResourceLimit(error)))?;
            engine::preserve_semantically_equal_values(self.form_data(), &mut candidate);
            let data_changed = !engine::json_values_equal(self.form_data(), &candidate);
            let mut directly_changed = self.host_directly_changed_nodes(&candidate, &writes);
            directly_changed.extend(self.host_structurally_changed_nodes(&array_topologies));
            let validation = data_changed.then(|| {
                self.definition.inner.validator.validate_with_limits(
                    &candidate,
                    self.limits.retained_validation_findings(),
                    self.limits.validation_parameter_bytes(),
                )
            });
            let engine_writes = writes
                .iter()
                .map(|write| {
                    PointerBuf::parse(write.as_str().to_owned())
                        .expect("public JSON Pointers are validated during construction")
                })
                .collect::<Vec<_>>();
            let array_changes = array_topologies
                .iter()
                .map(|topology| {
                    if topology.authoritative_replacement {
                        engine::HostArrayChange::Replace
                    } else {
                        engine::HostArrayChange::Preserve(topology.items.clone())
                    }
                })
                .collect::<Vec<_>>();
            self.engine
                .apply_host_transaction(candidate, &engine_writes, &array_changes, &item_writes)
                .map_err(|_| TransactionError::Commit(HostCommitError::InvalidOperation))?;
            if !data_changed {
                self.rebase_external_findings_for_topologies(
                    &array_topologies_before,
                    &array_topologies,
                    UnmatchedFinding::Remove,
                );
            }
            if let Some(validation) = validation {
                self.validation = validation;
                self.external_finding_batches.clear();
            }
            let after = self.revisions();
            let changed =
                self.host_changed_nodes(&directly_changed, before, findings_before, after);
            Ok(self.topology_transition(before, after, identities_before, changed))
        }

        /// Applies an infallible host closure with the same atomic commit rules as
        /// [`Self::try_transact`].
        ///
        /// Operations on [`HostTransaction`] return `()`; any invalid queued
        /// operation is reported here as [`HostCommitError`] and rolls back the
        /// complete transaction.
        pub fn transact<F>(&mut self, transaction: F) -> Result<Transition, HostCommitError>
        where
            F: FnOnce(&mut HostTransaction<'_>),
        {
            self.try_transact(|draft| {
                transaction(draft);
                Ok::<_, Infallible>(())
            })
            .map_err(|error| match error {
                TransactionError::Closure(error) => match error {},
                TransactionError::Commit(error) => error,
            })
        }

        /// Atomically replaces one source's external findings for the current data
        /// revision.
        ///
        /// The batch must name this form's exact current [`DataRevision`]. An empty
        /// batch removes that source. The raw batch and each parameter are bounded
        /// before findings are sorted and deduplicated; aggregate active-count and
        /// byte limits are then checked before mutation. Form data is never changed.
        /// Reapplying an identical batch is a no-op transition.
        pub fn apply_external_findings(
            &mut self,
            mut batch: ExternalFindingBatch,
        ) -> Result<Transition, ExternalFindingError> {
            let before = self.revisions();
            if batch.revision.form != self.id {
                return Err(ExternalFindingError::StaleRevision {
                    current: before.0,
                    supplied: batch.revision,
                });
            }
            if batch.revision != before.0 {
                return Err(ExternalFindingError::StaleRevision {
                    current: before.0,
                    supplied: batch.revision,
                });
            }
            let incoming_count = batch.findings.len();
            if incoming_count > self.external_finding_limits.max_incoming_findings {
                return Err(ExternalFindingError::ResourceLimit(
                    ResourceLimitError::new(
                        ResourceLimitPhase::Operation,
                        "incoming_external_findings",
                        self.external_finding_limits.max_incoming_findings,
                        incoming_count,
                        JsonPointer::parse("").expect("the root JSON Pointer is valid"),
                    ),
                ));
            }
            let parameter_limits = crate::limits::InputLimits {
                bytes: self.external_finding_limits.max_incoming_bytes,
                tokens: self.external_finding_limits.max_parameter_nodes,
                depth: self.external_finding_limits.max_parameter_depth,
                nodes: self.external_finding_limits.max_parameter_nodes,
                members: self.external_finding_limits.max_parameter_nodes,
                collection_length: self.external_finding_limits.max_parameter_collection_length,
                scalar_bytes: self.external_finding_limits.max_parameter_scalar_bytes,
            };
            for finding in &batch.findings {
                if let Err(error) =
                    crate::limits::check_input_value(&finding.parameters, parameter_limits)
                {
                    let dimension = match error.dimension {
                        "depth" => "external_finding_parameter_depth",
                        "nodes" => "external_finding_parameter_nodes",
                        "members" => "external_finding_parameter_nodes",
                        "collection_length" => "external_finding_parameter_collection_length",
                        "scalar_bytes" => "external_finding_parameter_scalar_bytes",
                        other => other,
                    };
                    return Err(ExternalFindingError::ResourceLimit(
                        ResourceLimitError::new(
                            ResourceLimitPhase::Operation,
                            dimension,
                            error.maximum,
                            error.observed,
                            finding.instance_location.clone(),
                        ),
                    ));
                }
            }
            if let Err(observed_bytes) = incoming_external_finding_batch_bytes(
                &batch,
                self.external_finding_limits.max_incoming_bytes,
            ) {
                return Err(ExternalFindingError::ResourceLimit(
                    ResourceLimitError::new(
                        ResourceLimitPhase::Operation,
                        "incoming_external_finding_bytes",
                        self.external_finding_limits.max_incoming_bytes,
                        observed_bytes,
                        JsonPointer::parse("").expect("the root JSON Pointer is valid"),
                    ),
                ));
            }
            sort_external_findings(&mut batch.findings);
            batch.findings.dedup();
            let findings_before = self.visible_external_findings_by_node();
            let existing_index = self
                .external_finding_batches
                .iter()
                .position(|existing| existing.source == batch.source);
            let retained_count = self
                .external_finding_batches
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != existing_index)
                .fold(0usize, |count, (_, batch)| {
                    count.saturating_add(batch.findings.len())
                });
            let observed_count = retained_count.saturating_add(batch.findings.len());
            if observed_count > self.external_finding_limits.max_active_findings {
                return Err(ExternalFindingError::ResourceLimit(
                    ResourceLimitError::new(
                        ResourceLimitPhase::Operation,
                        "active_external_findings",
                        self.external_finding_limits.max_active_findings,
                        observed_count,
                        JsonPointer::parse("").expect("the root JSON Pointer is valid"),
                    ),
                ));
            }
            let incoming_bytes = match active_external_finding_batch_bytes(
                &batch,
                self.external_finding_limits.max_active_bytes,
            ) {
                Ok(bytes) => bytes,
                Err(observed_bytes) => {
                    return Err(ExternalFindingError::ResourceLimit(
                        ResourceLimitError::new(
                            ResourceLimitPhase::Operation,
                            "active_external_finding_bytes",
                            self.external_finding_limits.max_active_bytes,
                            observed_bytes,
                            JsonPointer::parse("").expect("the root JSON Pointer is valid"),
                        ),
                    ));
                }
            };
            let retained_bytes = self
                .external_finding_batches
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != existing_index)
                .map(|(_, batch)| {
                    active_external_finding_batch_bytes(
                        batch,
                        self.external_finding_limits.max_active_bytes,
                    )
                    .expect("stored external finding batches satisfy the active byte limit")
                })
                .fold(0usize, usize::saturating_add);
            let observed_bytes = retained_bytes.saturating_add(incoming_bytes);
            if observed_bytes > self.external_finding_limits.max_active_bytes {
                return Err(ExternalFindingError::ResourceLimit(
                    ResourceLimitError::new(
                        ResourceLimitPhase::Operation,
                        "active_external_finding_bytes",
                        self.external_finding_limits.max_active_bytes,
                        observed_bytes,
                        JsonPointer::parse("").expect("the root JSON Pointer is valid"),
                    ),
                ));
            }
            let changed = if batch.findings.is_empty() {
                existing_index
                    .map(|index| self.external_finding_batches.remove(index))
                    .is_some()
            } else if let Some(index) = existing_index {
                if self.external_finding_batches[index] == batch {
                    false
                } else {
                    self.external_finding_batches[index] = batch;
                    true
                }
            } else {
                self.external_finding_batches.push(batch);
                self.external_finding_batches
                    .sort_by(|left, right| left.source.cmp(&right.source));
                true
            };
            if changed {
                self.engine.mark_state_changed();
            }
            let after = self.revisions();
            let findings_after = self.visible_external_findings_by_node();
            let changed = findings_before
                .into_iter()
                .zip(findings_after)
                .filter_map(
                    |((identity, before_findings), (after_identity, after_findings))| {
                        debug_assert_eq!(identity, after_identity);
                        (before_findings != after_findings).then_some(identity)
                    },
                )
                .collect();
            Ok(self.transition(before, after, changed))
        }

        /// Restores the form's baseline data and interaction state.
        ///
        /// Validation is refreshed when data changes. External findings are
        /// cleared on a data change and otherwise rebased across restored array
        /// topology where identities can be preserved.
        pub fn reset(&mut self) -> Transition {
            let before = self.revisions();
            let identities_before = self.instance_identities();
            let observations_before = self.node_observations();
            let array_topologies_before = self.host_array_topologies();
            self.engine.reset();
            self.revalidate_if_data_changed(before.0);
            let after = self.revisions();
            if before.0 != after.0 {
                self.external_finding_batches.clear();
            } else {
                let array_topologies_after = self.host_array_topologies();
                self.rebase_external_findings_for_topologies(
                    &array_topologies_before,
                    &array_topologies_after,
                    UnmatchedFinding::Preserve,
                );
            }
            let changed = self.changed_nodes(observations_before);
            self.topology_transition(before, after, identities_before, changed)
        }

        /// Changes validation and external-finding presentation policy.
        ///
        /// This may advance only the state revision and report nodes whose visible
        /// findings changed; it does not revalidate or mutate form data.
        pub fn set_finding_visibility(&mut self, policy: FindingVisibilityPolicy) -> Transition {
            let before = self.revisions();
            let observations_before = self.node_observations();
            if self.finding_visibility != policy {
                self.finding_visibility = policy;
                self.engine.mark_state_changed();
            }
            let after = self.revisions();
            self.transition(before, after, self.changed_nodes(observations_before))
        }

        /// Replaces the form's baseline and current data with a new owned object.
        ///
        /// Construction-style shape and runtime resource limits are checked before
        /// mutation. Success resets interaction state, revalidates, clears all
        /// external findings, and may replace array-item identities; failure is
        /// atomic.
        pub fn reinitialize(&mut self, form_data: Value) -> Result<Transition, ReinitializeError> {
            if !form_data.is_object() {
                return Err(ReinitializeError::InvalidFormData);
            }
            check_runtime_form_data(&self.definition, &form_data, self.limits)
                .map_err(ReinitializeError::ResourceLimit)?;
            let before = self.revisions();
            let identities_before = self.instance_identities();
            let observations_before = self.node_observations();
            self.engine
                .reinitialize(form_data)
                .map_err(|_| ReinitializeError::InvalidFormData)?;
            self.external_finding_batches.clear();
            self.revalidate();
            let after = self.revisions();
            let changed = self.changed_nodes(observations_before);
            Ok(self.topology_transition(before, after, identities_before, changed))
        }

        /// Finalizes parseable buffers and returns every blocker or one snapshot.
        ///
        /// Blocked submission is an ordinary [`SubmissionOutcome`], not an
        /// operation error. Preparation marks submission as attempted and can
        /// commit parseable edit buffers, so callers must process its [`Transition`]
        /// even when blocked. This method performs no serialization or transport.
        pub fn prepare_submission(&mut self) -> SubmissionPreparation {
            let before = self.revisions();
            let observations_before = self.node_observations();
            let (snapshot, mut blockers) = match self.engine.prepare_submission() {
                Ok(snapshot) => (Some(snapshot), Vec::new()),
                Err(failure) => {
                    let blockers = failure
                        .parse_blockers()
                        .filter_map(|blocker| {
                            self.identity_for_binding(blocker.binding()).map(|target| {
                                SubmissionBlocker::Parse {
                                    target,
                                    kind: match blocker.reason() {
                                        engine::ParseBlocker::InvalidNumber => {
                                            ParseBlockerKind::InvalidNumber
                                        }
                                        engine::ParseBlocker::InvalidInteger => {
                                            ParseBlockerKind::InvalidInteger
                                        }
                                        engine::ParseBlocker::ResourceLimitExceeded => {
                                            ParseBlockerKind::ResourceLimitExceeded
                                        }
                                    },
                                }
                            })
                        })
                        .collect::<Vec<_>>();
                    (None, blockers)
                }
            };
            match &self.validation {
                validation::Outcome::Valid => {}
                validation::Outcome::Invalid {
                    findings,
                    truncated,
                } => {
                    blockers.extend(findings.iter().cloned().map(SubmissionBlocker::Validation));
                    if *truncated {
                        blockers.push(SubmissionBlocker::ValidationFindingsTruncated {
                            retained: findings.len(),
                        });
                    }
                }
                validation::Outcome::Indeterminate(reason) => {
                    blockers.push(SubmissionBlocker::Indeterminate(reason.clone()))
                }
            }
            blockers.extend(
                self.definition
                    .capability_findings()
                    .filter(|finding| finding.is_blocking())
                    .cloned()
                    .map(SubmissionBlocker::Capability),
            );
            blockers.extend(self.external_finding_batches.iter().flat_map(|batch| {
                batch
                    .findings
                    .iter()
                    .filter(|finding| finding.is_blocking())
                    .cloned()
                    .map(|finding| SubmissionBlocker::External {
                        source: batch.source.clone(),
                        finding,
                    })
            }));
            let outcome = if blockers.is_empty() {
                let snapshot = snapshot.expect("a blocker-free engine preparation has a snapshot");
                SubmissionOutcome::Ready(SubmissionSnapshot {
                    form_data: snapshot.form_data().clone(),
                    data_revision: DataRevision {
                        form: self.id,
                        revision: snapshot.data_revision(),
                    },
                    definition_fingerprint: self.definition.fingerprint(),
                })
            } else {
                SubmissionOutcome::Blocked(SubmissionBlockers { blockers })
            };
            let after = self.revisions();
            SubmissionPreparation {
                transition: self.transition(before, after, self.changed_nodes(observations_before)),
                outcome,
            }
        }

        fn revalidate(&mut self) {
            self.validation = self.definition.inner.validator.validate_with_limits(
                self.engine.form_data(),
                self.limits.retained_validation_findings(),
                self.limits.validation_parameter_bytes(),
            );
        }

        fn revalidate_if_data_changed(&mut self, before: DataRevision) {
            if self.engine.data_revision() != before.revision {
                self.revalidate();
            }
        }

        fn validation_findings(&self) -> &[ValidationFinding] {
            match &self.validation {
                validation::Outcome::Invalid { findings, .. } => findings,
                validation::Outcome::Valid | validation::Outcome::Indeterminate(_) => &[],
            }
        }

        fn visible_validation_findings_by_node(
            &self,
        ) -> Vec<(InstanceIdentity, Vec<ValidationFinding>)> {
            self.instance_identities()
                .into_iter()
                .map(|identity| {
                    let findings = self
                        .node(identity)
                        .expect("definition nodes always have form instances")
                        .validation_findings()
                        .cloned()
                        .collect();
                    (identity, findings)
                })
                .collect()
        }

        fn validation_finding_visible(&self, finding: &ValidationFinding) -> bool {
            match self.finding_visibility.validation {
                FindingVisibility::Immediate => true,
                FindingVisibility::SubmissionOnly => self.engine.submission_attempted(),
                FindingVisibility::TouchedOrSubmission => {
                    self.engine.submission_attempted()
                        || self
                            .engine
                            .control(finding.instance_location().as_str())
                            .is_some_and(|control| control.is_touched())
                }
            }
        }

        fn aggregate_validation_outcome_visible(&self) -> bool {
            match self.finding_visibility.validation {
                FindingVisibility::Immediate => true,
                FindingVisibility::TouchedOrSubmission | FindingVisibility::SubmissionOnly => {
                    self.engine.submission_attempted()
                }
            }
        }

        fn external_finding_visible(&self, finding: &ExternalFinding) -> bool {
            match self.finding_visibility.external {
                FindingVisibility::Immediate => true,
                FindingVisibility::SubmissionOnly => self.engine.submission_attempted(),
                FindingVisibility::TouchedOrSubmission => {
                    self.engine.submission_attempted()
                        || self
                            .engine
                            .control(finding.instance_location().as_str())
                            .is_some_and(|control| control.is_touched())
                }
            }
        }

        fn revisions(&self) -> (DataRevision, StateRevision) {
            (
                DataRevision {
                    form: self.id,
                    revision: self.engine.data_revision(),
                },
                StateRevision {
                    form: self.id,
                    revision: self.engine.state_revision(),
                },
            )
        }

        fn identity(&self, node: DefinitionNodeId) -> InstanceIdentity {
            InstanceIdentity {
                form: self.id,
                node: node.0,
                item: None,
            }
        }

        fn item_identity(&self, node: DefinitionNodeId, item: ItemIdentity) -> InstanceIdentity {
            debug_assert_eq!(item.form, self.id);
            InstanceIdentity {
                form: self.id,
                node: node.0,
                item: Some(item),
            }
        }

        fn item(&self, local: u64) -> ItemIdentity {
            ItemIdentity {
                form: self.id,
                local,
            }
        }

        fn item_subtree_identities(
            &self,
            template: DefinitionNodeId,
            item: ItemIdentity,
        ) -> Vec<InstanceIdentity> {
            let mut identities = Vec::new();
            let mut pending = vec![template];
            while let Some(node) = pending.pop() {
                identities.push(self.item_identity(node, item));
                let mut children = self
                    .definition
                    .node(node)
                    .map(|node| node.children().collect::<Vec<_>>())
                    .unwrap_or_default();
                children.reverse();
                pending.extend(children);
            }
            identities
        }

        fn identity_for_binding(&self, binding: &str) -> Option<InstanceIdentity> {
            self.instance_identities().into_iter().find(|identity| {
                self.node(*identity)
                    .and_then(|node| node.binding())
                    .is_some_and(|current| current.pointer().as_str() == binding)
            })
        }

        fn instance_identities(&self) -> Vec<InstanceIdentity> {
            let root = self.identity(self.definition.root());
            let mut identities = Vec::new();
            let mut pending = vec![root];
            while let Some(identity) = pending.pop() {
                identities.push(identity);
                let mut children = self
                    .node(identity)
                    .map(|node| node.children().collect::<Vec<_>>())
                    .unwrap_or_default();
                children.reverse();
                pending.extend(children);
            }
            identities
        }

        fn identity_for_finding_location(&self, location: &JsonPointer) -> InstanceIdentity {
            self.identity_for_binding(location.as_str())
                .unwrap_or_else(|| self.identity(self.definition.root()))
        }

        fn transition(
            &self,
            before: (DataRevision, StateRevision),
            after: (DataRevision, StateRevision),
            changed: Vec<InstanceIdentity>,
        ) -> Transition {
            Transition {
                before_data: before.0,
                after_data: after.0,
                before_state: before.1,
                after_state: after.1,
                changed,
                removed: Vec::new(),
            }
        }

        fn topology_transition(
            &self,
            before: (DataRevision, StateRevision),
            after: (DataRevision, StateRevision),
            identities_before: Vec<InstanceIdentity>,
            mut changed: Vec<InstanceIdentity>,
        ) -> Transition {
            let identities_after = self.instance_identities();
            let after_set = identities_after.iter().copied().collect::<HashSet<_>>();
            let before_set = identities_before.iter().copied().collect::<HashSet<_>>();
            changed.extend(
                identities_after
                    .into_iter()
                    .filter(|identity| !before_set.contains(identity)),
            );
            let mut seen = HashSet::new();
            changed.retain(|identity| after_set.contains(identity) && seen.insert(*identity));
            Transition {
                before_data: before.0,
                after_data: after.0,
                before_state: before.1,
                after_state: after.1,
                changed,
                removed: identities_before
                    .into_iter()
                    .filter(|identity| !after_set.contains(identity))
                    .collect(),
            }
        }

        fn user_transition(
            &self,
            target: InstanceIdentity,
            before: (DataRevision, StateRevision),
            findings_before: Vec<(InstanceIdentity, Vec<ValidationFinding>)>,
            external_targets_before: Vec<InstanceIdentity>,
            after: (DataRevision, StateRevision),
        ) -> Transition {
            let mut affected = vec![target];
            if let Some(target_binding) = self
                .node(target)
                .and_then(|node| node.binding().map(|binding| binding.pointer().clone()))
            {
                affected.extend(self.instance_identities().into_iter().filter(|identity| {
                    self.node(*identity)
                        .and_then(|node| node.binding())
                        .is_some_and(|binding| {
                            target_binding.is_strict_descendant_of(binding.pointer())
                        })
                }));
            }
            let mut pending = vec![target];
            while let Some(identity) = pending.pop() {
                let children = self
                    .node(identity)
                    .map(|node| node.children().collect::<Vec<_>>())
                    .unwrap_or_default();
                affected.extend(children.iter().copied());
                pending.extend(children);
            }
            let findings_after = self.visible_validation_findings_by_node();
            let data_changed = before.0 != after.0;
            if data_changed {
                affected.extend(external_targets_before);
            }
            let changed = findings_before
                .into_iter()
                .zip(findings_after)
                .filter_map(
                    |((identity, before_findings), (after_identity, after_findings))| {
                        debug_assert_eq!(identity, after_identity);
                        ((identity == target && before != after)
                            || (data_changed && affected.contains(&identity))
                            || before_findings != after_findings)
                            .then_some(identity)
                    },
                )
                .collect();
            self.transition(before, after, changed)
        }

        fn changed_nodes(
            &self,
            before: Vec<(InstanceIdentity, NodeObservation)>,
        ) -> Vec<InstanceIdentity> {
            let before = before.into_iter().collect::<HashMap<_, _>>();
            self.instance_identities()
                .into_iter()
                .filter(|identity| {
                    self.node_observation(*identity)
                        .is_some_and(|after| before.get(identity) != Some(&after))
                })
                .collect()
        }

        fn node_observations(&self) -> Vec<(InstanceIdentity, NodeObservation)> {
            self.instance_identities()
                .into_iter()
                .filter_map(|identity| {
                    self.node_observation(identity)
                        .map(|observation| (identity, observation))
                })
                .collect()
        }

        fn node_observation(&self, identity: InstanceIdentity) -> Option<NodeObservation> {
            let node = self.node(identity)?;
            let findings = node
                .visible_findings()
                .map(|finding| match finding {
                    FindingView::Validation { finding, .. } => {
                        FindingObservation::Validation(finding.clone())
                    }
                    FindingView::ValidationFindingsTruncated { retained, .. } => {
                        FindingObservation::ValidationFindingsTruncated(retained)
                    }
                    FindingView::Indeterminate { reason, .. } => {
                        FindingObservation::Indeterminate(reason.clone())
                    }
                    FindingView::Capability { finding, .. } => {
                        FindingObservation::Capability(finding.clone())
                    }
                    FindingView::External {
                        source, finding, ..
                    } => FindingObservation::External(source.to_owned(), finding.clone()),
                    FindingView::Parse { kind, .. } => FindingObservation::Parse(kind),
                })
                .collect();
            Some(NodeObservation {
                binding: node.binding().map(|binding| binding.pointer().clone()),
                children: node.children().collect(),
                current_data: node.current_data().cloned(),
                edit_buffer: node.edit_buffer().map(str::to_owned),
                allowed_operations: node.allowed_operations(),
                value_state: node.value_state(),
                touched: node.is_touched(),
                dirty: node.is_dirty(),
                findings,
            })
        }

        fn host_changed_nodes(
            &self,
            directly_changed: &[InstanceIdentity],
            before: (DataRevision, StateRevision),
            findings_before: Vec<(InstanceIdentity, Vec<ValidationFinding>)>,
            after: (DataRevision, StateRevision),
        ) -> Vec<InstanceIdentity> {
            if before == after {
                return Vec::new();
            }
            let findings_before = findings_before.into_iter().collect::<HashMap<_, _>>();
            self.visible_validation_findings_by_node()
                .into_iter()
                .filter_map(|(identity, after_findings)| {
                    (directly_changed.contains(&identity)
                        || findings_before.get(&identity) != Some(&after_findings))
                    .then_some(identity)
                })
                .collect()
        }

        fn host_directly_changed_nodes(
            &self,
            candidate: &Value,
            writes: &[JsonPointer],
        ) -> Vec<InstanceIdentity> {
            let data_changed = !engine::json_values_equal(self.form_data(), candidate);
            let cleared_external_locations = if data_changed {
                self.external_finding_batches
                    .iter()
                    .flat_map(|batch| &batch.findings)
                    .map(|finding| finding.instance_location.as_str().to_owned())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            self.instance_identities()
                .into_iter()
                .filter_map(|identity| {
                    let binding = self.node(identity).and_then(|node| node.binding())?;
                    let pointer = PointerBuf::parse(binding.as_str().to_owned())
                        .expect("current bindings are valid JSON Pointers");
                    let data_changed = match (
                        pointer.resolve(self.form_data()),
                        pointer.resolve(candidate),
                    ) {
                        (Ok(current), Ok(candidate)) => {
                            !engine::json_values_equal(current, candidate)
                        }
                        (Err(_), Err(_)) => false,
                        _ => true,
                    };
                    let parent_object_changed = pointer.split_back().is_some_and(|(parent, _)| {
                        parent
                            .resolve(self.form_data())
                            .ok()
                            .is_some_and(Value::is_object)
                            != parent.resolve(candidate).ok().is_some_and(Value::is_object)
                    });
                    let edit_state_cleared = writes
                        .iter()
                        .any(|write| write.intersects(binding.pointer()))
                        && self
                            .engine
                            .control(binding.as_str())
                            .is_some_and(|control| {
                                control.edit_buffer().is_some() || control.parse_blocker().is_some()
                            });
                    let external_findings_cleared = cleared_external_locations
                        .iter()
                        .any(|location| location == binding.as_str());
                    (data_changed
                        || parent_object_changed
                        || edit_state_cleared
                        || external_findings_cleared)
                        .then_some(identity)
                })
                .collect()
        }

        fn host_structurally_changed_nodes(
            &self,
            array_topologies: &[HostArrayTopology],
        ) -> Vec<InstanceIdentity> {
            let mut changed = Vec::new();
            for (array, topology) in self.definition.inner.engine.arrays().zip(array_topologies) {
                let before = self
                    .engine
                    .array_item_identities(array.binding())
                    .unwrap_or_default();
                if before
                    .iter()
                    .copied()
                    .map(engine::HostArrayItem::Existing)
                    .eq(topology.items.iter().copied())
                {
                    continue;
                }
                let Some(array_identity) = self.identity_for_binding(array.binding()) else {
                    continue;
                };
                changed.push(array_identity);
                let Some(template) = self
                    .definition
                    .array_template(DefinitionNodeId(array_identity.node))
                else {
                    continue;
                };
                for (new_index, item) in topology.items.iter().enumerate() {
                    let engine::HostArrayItem::Existing(identity) = item else {
                        continue;
                    };
                    if before.iter().position(|before| before == identity) != Some(new_index) {
                        changed
                            .extend(self.item_subtree_identities(template, self.item(*identity)));
                    }
                }
            }
            changed
        }

        fn host_array_topologies(&self) -> Vec<HostArrayTopology> {
            self.definition
                .inner
                .engine
                .arrays()
                .map(|array| {
                    let binding = PointerBuf::parse(array.binding().to_owned())
                        .expect("compiled array bindings are valid JSON Pointers");
                    let items = self
                        .engine
                        .array_item_identities(array.binding())
                        .unwrap_or_default()
                        .into_iter()
                        .map(engine::HostArrayItem::Existing)
                        .collect::<Vec<_>>();
                    HostArrayTopology {
                        original: binding.resolve(self.form_data()).ok().cloned(),
                        original_items: items.clone(),
                        items,
                        binding,
                        authoritative_replacement: false,
                    }
                })
                .collect()
        }

        fn visible_external_finding_targets(&self) -> Vec<InstanceIdentity> {
            self.instance_identities()
                .into_iter()
                .filter(|identity| *identity != self.identity(self.definition.root()))
                .filter(|identity| {
                    self.node(*identity)
                        .is_some_and(|node| node.external_findings().next().is_some())
                })
                .collect()
        }

        fn move_external_findings(&mut self, array: &str, from: usize, to: usize) {
            let from = JsonPointer::parse(format!("{array}/{from}"))
                .expect("an array binding with an index is a valid JSON Pointer");
            let to = JsonPointer::parse(format!("{array}/{to}"))
                .expect("an array binding with an index is a valid JSON Pointer");
            for batch in &mut self.external_finding_batches {
                for finding in &mut batch.findings {
                    if let Some(rebased) = rebase_pointer(&finding.instance_location, &from, &to) {
                        finding.instance_location = rebased;
                    } else if let Some(rebased) =
                        rebase_pointer(&finding.instance_location, &to, &from)
                    {
                        finding.instance_location = rebased;
                    }
                }
                sort_external_findings(&mut batch.findings);
            }
        }

        fn rebase_external_findings_for_topologies(
            &mut self,
            before: &[HostArrayTopology],
            after: &[HostArrayTopology],
            unmatched: UnmatchedFinding,
        ) {
            for batch in &mut self.external_finding_batches {
                batch.findings.retain_mut(|finding| {
                    for (before, after) in before.iter().zip(after) {
                        let array = before.binding.as_str();
                        for (old_index, item) in before.items.iter().enumerate() {
                            let from = JsonPointer::parse(format!("{array}/{old_index}"))
                                .expect("an array binding with an index is a valid JSON Pointer");
                            let Some(relative) = rebase_pointer(
                                &finding.instance_location,
                                &from,
                                &JsonPointer::parse("")
                                    .expect("the root JSON Pointer is always valid"),
                            ) else {
                                continue;
                            };
                            let Some(new_index) =
                                after.items.iter().position(|candidate| candidate == item)
                            else {
                                return unmatched == UnmatchedFinding::Preserve;
                            };
                            finding.instance_location = JsonPointer::parse(format!(
                                "{array}/{new_index}{}",
                                relative.as_str()
                            ))
                            .expect("rebasing an array finding produces a valid JSON Pointer");
                            break;
                        }
                    }
                    true
                });
                sort_external_findings(&mut batch.findings);
            }
            self.external_finding_batches
                .retain(|batch| !batch.findings.is_empty());
        }

        fn visible_external_findings_by_node(
            &self,
        ) -> Vec<(InstanceIdentity, Vec<(String, ExternalFinding)>)> {
            self.instance_identities()
                .into_iter()
                .map(|identity| {
                    let findings = self
                        .node(identity)
                        .expect("definition nodes always have form instances")
                        .external_findings()
                        .map(|(source, finding)| (source.to_owned(), finding.clone()))
                        .collect();
                    (identity, findings)
                })
                .collect()
        }
    }

    fn check_form_tree(
        definition: &FormDefinition,
        form_data: &Value,
        limits: FormDataLimits,
        phase: ResourceLimitPhase,
    ) -> Result<(), ResourceLimitError> {
        let (form_tree_nodes, repeated_items) = definition.initial_tree_metrics(form_data);
        if repeated_items > limits.repeated_items() {
            return Err(ResourceLimitError::new(
                phase,
                "repeated_items",
                limits.repeated_items(),
                repeated_items,
                JsonPointer::parse("").expect("the root JSON Pointer is valid"),
            ));
        }
        if form_tree_nodes > limits.form_tree_nodes() {
            return Err(ResourceLimitError::new(
                phase,
                "form_tree_nodes",
                limits.form_tree_nodes(),
                form_tree_nodes,
                JsonPointer::parse("").expect("the root JSON Pointer is valid"),
            ));
        }
        Ok(())
    }

    fn check_runtime_form_data(
        definition: &FormDefinition,
        form_data: &Value,
        limits: FormDataLimits,
    ) -> Result<(), ResourceLimitError> {
        crate::limits::check_input_value(form_data, limits.input_limits()).map_err(|error| {
            ResourceLimitError::new(
                ResourceLimitPhase::Operation,
                error.dimension,
                error.maximum,
                error.observed,
                JsonPointer::parse(error.pointer)
                    .expect("input limit scans produce valid JSON Pointers"),
            )
        })?;
        check_form_tree(definition, form_data, limits, ResourceLimitPhase::Operation)
    }

    fn check_runtime_input_value(
        value: &Value,
        binding: &str,
        limits: FormDataLimits,
    ) -> Result<(), ResourceLimitError> {
        crate::limits::check_input_value(value, limits.input_limits()).map_err(|error| {
            ResourceLimitError::new(
                ResourceLimitPhase::Operation,
                error.dimension,
                error.maximum,
                error.observed,
                JsonPointer::parse(format!("{binding}{}", error.pointer))
                    .expect("a control binding joined with a relative scan pointer is valid"),
            )
        })
    }

    fn rebase_pointer(
        location: &JsonPointer,
        from: &JsonPointer,
        to: &JsonPointer,
    ) -> Option<JsonPointer> {
        let location_pointer = jsonptr::Pointer::parse(location.as_str()).ok()?;
        let from_pointer = jsonptr::Pointer::parse(from.as_str()).ok()?;
        location_pointer.starts_with(from_pointer).then(|| {
            JsonPointer::parse(format!(
                "{}{}",
                to.as_str(),
                &location.as_str()[from.as_str().len()..]
            ))
            .expect("rebasing valid JSON Pointer prefixes produces a valid JSON Pointer")
        })
    }

    fn sort_external_findings(findings: &mut [ExternalFinding]) {
        findings.sort_by(|left, right| {
            left.instance_location
                .cmp(&right.instance_location)
                .then_with(|| left.code.cmp(&right.code))
                .then_with(|| left.blocking.cmp(&right.blocking))
                .then_with(|| {
                    left.parameters
                        .to_string()
                        .cmp(&right.parameters.to_string())
                })
        });
    }

    fn incoming_external_finding_batch_bytes(
        batch: &ExternalFindingBatch,
        maximum: usize,
    ) -> Result<usize, usize> {
        let mut bytes = 0usize;
        add_external_finding_bytes(&mut bytes, batch.source.len(), maximum)?;
        add_external_findings_bytes(batch, &mut bytes, maximum)?;
        Ok(bytes)
    }

    fn active_external_finding_batch_bytes(
        batch: &ExternalFindingBatch,
        maximum: usize,
    ) -> Result<usize, usize> {
        if batch.findings.is_empty() {
            return Ok(0);
        }
        let mut bytes = 0usize;
        add_external_finding_bytes(&mut bytes, batch.source.len(), maximum)?;
        add_external_findings_bytes(batch, &mut bytes, maximum)?;
        Ok(bytes)
    }

    fn add_external_findings_bytes(
        batch: &ExternalFindingBatch,
        bytes: &mut usize,
        maximum: usize,
    ) -> Result<(), usize> {
        for finding in &batch.findings {
            add_external_finding_bytes(bytes, finding.code.len(), maximum)?;
            add_external_finding_bytes(bytes, finding.instance_location.as_str().len(), maximum)?;
            add_json_encoded_len(&finding.parameters, bytes, maximum)?;
        }
        Ok(())
    }

    fn add_external_finding_bytes(
        bytes: &mut usize,
        additional: usize,
        maximum: usize,
    ) -> Result<(), usize> {
        *bytes = bytes.saturating_add(additional);
        if *bytes > maximum {
            Err(*bytes)
        } else {
            Ok(())
        }
    }

    fn add_json_encoded_len(value: &Value, bytes: &mut usize, maximum: usize) -> Result<(), usize> {
        enum Frame<'a> {
            Value(&'a Value),
            Array(std::slice::Iter<'a, Value>),
            Object(serde_json::map::Iter<'a>),
        }

        let mut pending = vec![Frame::Value(value)];
        while let Some(frame) = pending.pop() {
            match frame {
                Frame::Value(Value::Null) => {
                    add_external_finding_bytes(bytes, 4, maximum)?;
                }
                Frame::Value(Value::Bool(true)) => {
                    add_external_finding_bytes(bytes, 4, maximum)?;
                }
                Frame::Value(Value::Bool(false)) => {
                    add_external_finding_bytes(bytes, 5, maximum)?;
                }
                Frame::Value(Value::Number(number)) => {
                    add_external_finding_bytes(bytes, number.as_str().len(), maximum)?;
                }
                Frame::Value(Value::String(value)) => {
                    add_json_string_encoded_len(value, bytes, maximum)?;
                }
                Frame::Value(Value::Array(values)) => {
                    add_external_finding_bytes(
                        bytes,
                        2usize.saturating_add(values.len().saturating_sub(1)),
                        maximum,
                    )?;
                    pending.push(Frame::Array(values.iter()));
                }
                Frame::Value(Value::Object(values)) => {
                    add_external_finding_bytes(
                        bytes,
                        2usize
                            .saturating_add(values.len().saturating_sub(1))
                            .saturating_add(values.len()),
                        maximum,
                    )?;
                    pending.push(Frame::Object(values.iter()));
                }
                Frame::Array(mut values) => {
                    if let Some(value) = values.next() {
                        pending.push(Frame::Array(values));
                        pending.push(Frame::Value(value));
                    }
                }
                Frame::Object(mut values) => {
                    if let Some((key, value)) = values.next() {
                        pending.push(Frame::Object(values));
                        pending.push(Frame::Value(value));
                        add_json_string_encoded_len(key, bytes, maximum)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn add_json_string_encoded_len(
        value: &str,
        bytes: &mut usize,
        maximum: usize,
    ) -> Result<(), usize> {
        add_external_finding_bytes(bytes, 2, maximum)?;
        for character in value.chars() {
            add_external_finding_bytes(
                bytes,
                match character {
                    '\"' | '\\' | '\u{0008}' | '\u{0009}' | '\n' | '\u{000c}' | '\r' => 2,
                    '\u{0000}'..='\u{001f}' => 6,
                    character => character.len_utf8(),
                },
                maximum,
            )?;
        }
        Ok(())
    }

    /// Configures construction of independent runtime state for a definition.
    ///
    /// The builder owns its input data but borrows the definition. Configuration
    /// is checked only by [`Self::build`].
    pub struct FormBuilder<'a> {
        definition: &'a FormDefinition,
        form_data: Value,
        visibility: FindingVisibilityPolicy,
        external_finding_limits: ExternalFindingLimits,
        limits: FormDataLimits,
    }

    impl<'a> FormBuilder<'a> {
        pub(crate) fn new(definition: &'a FormDefinition, form_data: Value) -> Self {
            Self {
                definition,
                form_data,
                visibility: FindingVisibilityPolicy::default(),
                external_finding_limits: ExternalFindingLimits::default(),
                limits: FormDataLimits::default(),
            }
        }

        /// Sets when validation and external findings become visible.
        ///
        /// Visibility affects presentation and transitions, not validation or
        /// whether findings block submission.
        pub fn finding_visibility(mut self, policy: FindingVisibilityPolicy) -> Self {
            self.visibility = policy;
            self
        }

        /// Sets aggregate and per-parameter limits for external findings.
        pub fn external_finding_limits(mut self, limits: ExternalFindingLimits) -> Self {
            self.external_finding_limits = limits;
            self
        }

        /// Replaces limits for baseline data and subsequent runtime operations.
        pub fn limits(mut self, limits: FormDataLimits) -> Self {
            self.limits = limits;
            self
        }

        /// Validates the owned input and creates independent form state.
        ///
        /// The root must be a JSON object and all construction limits must hold.
        /// Data-schema validation failures are retained as findings rather than
        /// failing construction.
        pub fn build(self) -> Result<Form, FormBuildError> {
            Form::new(
                self.definition.clone(),
                self.form_data,
                self.limits,
                self.visibility,
                self.external_finding_limits,
            )
        }
    }

    /// A borrowed, read-only view of form-wide revisions, validation, and findings.
    #[derive(Clone, Copy)]
    pub struct FormView<'a> {
        form: &'a Form,
    }

    impl FormView<'_> {
        /// Returns the current root instance identity for this form.
        pub fn root(&self) -> InstanceIdentity {
            self.form.identity(self.form.definition.root())
        }

        /// Returns the revision of canonical form data.
        ///
        /// Interaction-only changes, including invalid edit buffers and finding
        /// visibility, do not advance this revision.
        pub fn data_revision(&self) -> DataRevision {
            DataRevision {
                form: self.form.id,
                revision: self.form.engine.data_revision(),
            }
        }

        /// Returns the revision of all observable form state.
        pub fn state_revision(&self) -> StateRevision {
            StateRevision {
                form: self.form.id,
                revision: self.form.engine.state_revision(),
            }
        }

        /// Returns whether submission preparation has been attempted since the
        /// current lifecycle state was initialized or reset.
        pub fn submission_attempted(&self) -> bool {
            self.form.engine.submission_attempted()
        }

        /// Returns schema-validation status independent of visibility policy.
        ///
        /// An invalid result may be truncated at the configured retention limit;
        /// indeterminate means validation could not complete reliably.
        pub fn validation_outcome(&self) -> ValidationOutcomeView<'_> {
            match &self.form.validation {
                validation::Outcome::Valid => ValidationOutcomeView::Valid,
                validation::Outcome::Invalid {
                    findings,
                    truncated,
                } => ValidationOutcomeView::Invalid {
                    findings,
                    truncated: *truncated,
                },
                validation::Outcome::Indeterminate(reason) => {
                    ValidationOutcomeView::Indeterminate(reason)
                }
            }
        }

        /// Iterates currently visible validation, capability, external, and parse
        /// findings in deterministic category order.
        ///
        /// Validation and external findings obey [`FindingVisibilityPolicy`];
        /// capability and parse findings are always visible.
        pub fn visible_findings(&self) -> impl Iterator<Item = FindingView<'_>> {
            let validation = self
                .form
                .validation_findings()
                .iter()
                .filter(|finding| self.form.validation_finding_visible(finding))
                .map(|finding| FindingView::Validation {
                    target: self
                        .form
                        .identity_for_finding_location(finding.instance_location()),
                    finding,
                });
            let validation_truncated = match &self.form.validation {
                validation::Outcome::Invalid {
                    findings,
                    truncated: true,
                } if self.form.aggregate_validation_outcome_visible() => {
                    Some(FindingView::ValidationFindingsTruncated {
                        target: self.root(),
                        retained: findings.len(),
                    })
                }
                _ => None,
            };
            let indeterminate = match &self.form.validation {
                validation::Outcome::Indeterminate(reason)
                    if self.form.aggregate_validation_outcome_visible() =>
                {
                    Some(FindingView::Indeterminate {
                        target: self.root(),
                        reason,
                    })
                }
                _ => None,
            };
            let capability =
                self.form
                    .definition
                    .capability_findings()
                    .map(|finding| FindingView::Capability {
                        target: self
                            .form
                            .identity_for_finding_location(finding.instance_location()),
                        finding,
                    });
            let external = self.form.external_finding_batches.iter().flat_map(|batch| {
                batch.findings.iter().filter_map(|finding| {
                    self.form
                        .external_finding_visible(finding)
                        .then_some(FindingView::External {
                            target: self
                                .form
                                .identity_for_finding_location(finding.instance_location()),
                            source: batch.source.as_str(),
                            finding,
                        })
                })
            });
            let parse = self
                .form
                .instance_identities()
                .into_iter()
                .filter_map(|target| {
                    self.form
                        .node(target)
                        .and_then(|node| node.parse_blocker())
                        .map(|kind| FindingView::Parse { target, kind })
                });
            validation
                .chain(validation_truncated)
                .chain(indeterminate)
                .chain(capability)
                .chain(external)
                .chain(parse)
        }
    }

    /// A borrowed view of one current definition-node instance.
    ///
    /// Array item instances carry a stable [`ItemIdentity`] while their JSON
    /// pointer may change as neighboring items are inserted, removed, or moved.
    #[derive(Clone, Copy)]
    pub struct NodeView<'a> {
        form: &'a Form,
        identity: InstanceIdentity,
    }

    impl<'a> NodeView<'a> {
        /// Returns this node's form-scoped runtime identity.
        pub fn identity(&self) -> InstanceIdentity {
            self.identity
        }

        /// Returns the stable item identity for an array-template instance.
        pub fn item_identity(&self) -> Option<ItemIdentity> {
            self.identity.item
        }

        /// Returns the immutable definition node instantiated here.
        pub fn definition(&self) -> DefinitionNodeView<'a> {
            self.form
                .definition
                .node(DefinitionNodeId(self.identity.node))
                .expect("form nodes always reference their definition")
        }

        /// Returns the node's current absolute binding, if it binds form data.
        ///
        /// For array item instances this resolves the stable item identity to its
        /// current index; presentation-only nodes have no binding.
        pub fn binding(&self) -> Option<CurrentBinding> {
            if let Some(item) = self.identity.item {
                let template = DefinitionNodeId(self.identity.node);
                let array = self.form.definition.array_for_template(template)?;
                let array_binding = self.form.definition.node(array)?.binding()?;
                let pointer = self
                    .form
                    .engine
                    .array_item_binding(array_binding.as_str(), item.local)?;
                let relative = self.definition().binding()?;
                return Some(CurrentBinding {
                    pointer: JsonPointer::parse(format!("{pointer}{}", relative.as_str()))
                        .expect("engine array bindings are valid JSON Pointers"),
                    item: Some(item),
                });
            }
            self.definition()
                .binding()
                .cloned()
                .map(|pointer| CurrentBinding {
                    pointer,
                    item: None,
                })
        }

        /// Computes the user operations currently permitted by schema semantics,
        /// data shape, annotations, and array bounds.
        ///
        /// This is advisory for rendering; [`UserActions`] rechecks permission and
        /// resource limits atomically when an operation is attempted.
        pub fn allowed_operations(&self) -> AllowedOperations {
            if self.is_read_only() {
                return AllowedOperations::default();
            }
            let definition = self.definition();
            if definition.semantic_kind() == Some(SemanticKind::HomogeneousArray) {
                let mut operations = AllowedOperations::default();
                let binding = definition
                    .binding()
                    .expect("array definitions have root-origin bindings");
                if self.form.engine.array_can_append(binding.as_str()) {
                    operations |= AllowedOperations::APPEND_ITEM;
                }
                if self.form.engine.array_can_remove(binding.as_str()) {
                    operations |= AllowedOperations::REMOVE_ITEM;
                }
                if self.form.engine.array_can_move(binding.as_str()) {
                    operations |= AllowedOperations::MOVE_ITEM;
                }
                match self.current_data() {
                    Some(value) if !value.is_array() => {
                        operations |= AllowedOperations::REPLACE_VALUE;
                        if !definition.is_required() {
                            operations |= AllowedOperations::REMOVE_VALUE;
                        }
                    }
                    None if self.parent_is_object() => {
                        operations |= AllowedOperations::MATERIALIZE;
                    }
                    Some(_) if !definition.is_required() => {
                        operations |= AllowedOperations::REMOVE_VALUE;
                    }
                    _ => {}
                }
                return operations;
            }
            if definition.semantic_kind() == Some(SemanticKind::FixedObject) {
                let mut operations = AllowedOperations::default();
                let current = self.current_data();
                if current.is_some_and(|value| !value.is_object()) {
                    operations |= AllowedOperations::REPLACE_VALUE;
                    if !definition.is_required() {
                        operations |= AllowedOperations::REMOVE_VALUE;
                    }
                } else if current.is_none() && self.parent_is_object() {
                    operations |= AllowedOperations::MATERIALIZE;
                } else if current.is_some() && !definition.is_required() {
                    operations |= AllowedOperations::REMOVE_VALUE;
                }
                return operations;
            }
            let Some(state) = self.value_state() else {
                return AllowedOperations::default();
            };
            let can_write_missing = state != ScalarValueState::Missing || self.parent_is_object();
            let has_concrete_value = definition_has_concrete_value(definition);
            let mut operations = AllowedOperations::default();

            if matches!(
                definition.semantic_kind(),
                Some(SemanticKind::String | SemanticKind::Number | SemanticKind::Integer)
            ) && can_write_missing
                && (matches!(
                    state,
                    ScalarValueState::Missing
                        | ScalarValueState::Empty
                        | ScalarValueState::Compatible
                ) || matches!(state, ScalarValueState::Null) && definition.accepts_null())
            {
                operations |= AllowedOperations::INPUT_TEXT;
            }
            if can_write_missing
                && ((matches!(state, ScalarValueState::Missing)
                    || matches!(state, ScalarValueState::Null) && definition.accepts_null())
                    && has_concrete_value
                    || matches!(
                        state,
                        ScalarValueState::Empty | ScalarValueState::Compatible
                    ) && (definition.semantic_kind() == Some(SemanticKind::Boolean)
                        || definition.semantic_kind() == Some(SemanticKind::Choice)
                            && definition.is_choice_selectable()))
            {
                operations |= AllowedOperations::SET_VALUE;
            }
            if definition.accepts_null() && state != ScalarValueState::Null && can_write_missing {
                operations |= AllowedOperations::SET_NULL;
            }
            if !definition.is_required() && state != ScalarValueState::Missing {
                operations |= AllowedOperations::REMOVE_VALUE;
            }
            if has_concrete_value
                && (state == ScalarValueState::Incompatible
                    || state == ScalarValueState::Null && !definition.accepts_null())
            {
                operations |= AllowedOperations::REPLACE_VALUE;
            }
            if self.is_write_only()
                && has_concrete_value
                && matches!(
                    state,
                    ScalarValueState::Empty | ScalarValueState::Compatible
                )
            {
                operations |= AllowedOperations::REPLACE_VALUE;
            }
            operations
        }

        /// Returns whether this node or an enclosing schema scope is read-only.
        pub fn is_read_only(&self) -> bool {
            if self.definition().data_schema_annotations().is_read_only() {
                return true;
            }
            if self.identity.item.is_some()
                && self.form.definition.template_has_annotation(
                    DefinitionNodeId(self.identity.node),
                    DataSchemaAnnotations::is_read_only,
                )
            {
                return true;
            }
            let Some(binding) = self.binding() else {
                return false;
            };
            self.form.definition.binding_is_read_only(binding.pointer())
        }

        /// Returns whether this node or an enclosing schema scope is write-only.
        pub fn is_write_only(&self) -> bool {
            if self.definition().data_schema_annotations().is_write_only() {
                return true;
            }
            if self.identity.item.is_some()
                && self.form.definition.template_has_annotation(
                    DefinitionNodeId(self.identity.node),
                    DataSchemaAnnotations::is_write_only,
                )
            {
                return true;
            }
            let Some(binding) = self.binding() else {
                return false;
            };
            self.form
                .definition
                .binding_is_write_only(binding.pointer())
        }

        fn parent_is_object(&self) -> bool {
            let Some(binding) = self.binding() else {
                return false;
            };
            let Ok(pointer) = jsonptr::Pointer::parse(binding.pointer().as_str()) else {
                return false;
            };
            pointer
                .split_back()
                .and_then(|(parent, _)| parent.resolve(self.form.form_data()).ok())
                .is_some_and(Value::is_object)
        }

        /// Iterates current child instance identities in display order.
        ///
        /// For arrays, one child is produced per current item rather than exposing
        /// the definition's item template directly.
        pub fn children(&self) -> impl Iterator<Item = InstanceIdentity> + 'a {
            let form = self.form;
            if self.definition().semantic_kind() == Some(SemanticKind::HomogeneousArray) {
                let array = self
                    .definition()
                    .binding()
                    .expect("array definitions have bindings");
                let template = form
                    .definition
                    .array_template(DefinitionNodeId(self.identity.node))
                    .expect("compiled arrays own an item template");
                return form
                    .engine
                    .array_item_identities(array.as_str())
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |item| form.item_identity(template, form.item(item)))
                    .collect::<Vec<_>>()
                    .into_iter();
            }
            let children = self.definition().children().collect::<Vec<_>>();
            let item = self.identity.item;
            children
                .into_iter()
                .filter(move |id| {
                    form.definition
                        .node(*id)
                        .is_some_and(|node| item.is_some() || !node.is_item_template())
                })
                .map(move |id| match item {
                    Some(item) => form.item_identity(id, item),
                    None => form.identity(id),
                })
                .collect::<Vec<_>>()
                .into_iter()
        }

        /// Returns the exact temporary textual spelling retained for the active edit, if any.
        pub fn edit_buffer(&self) -> Option<&'a str> {
            let binding = self.binding()?;
            self.form.engine.control(binding.as_str())?.edit_buffer()
        }

        /// Returns why the current edit buffer cannot become canonical form data.
        pub fn parse_blocker(&self) -> Option<ParseBlockerKind> {
            let binding = self.binding()?;
            self.form
                .engine
                .control(binding.as_str())?
                .parse_blocker()
                .map(|blocker| match blocker {
                    engine::ParseBlocker::InvalidNumber => ParseBlockerKind::InvalidNumber,
                    engine::ParseBlocker::InvalidInteger => ParseBlockerKind::InvalidInteger,
                    engine::ParseBlocker::ResourceLimitExceeded => {
                        ParseBlockerKind::ResourceLimitExceeded
                    }
                })
        }

        /// Iterates visible schema-validation findings attached to this node.
        pub fn validation_findings(&self) -> impl Iterator<Item = &ValidationFinding> + '_ {
            self.form
                .validation_findings()
                .iter()
                .filter(move |finding| {
                    self.finding_attached(finding.instance_location())
                        && self.form.validation_finding_visible(finding)
                })
        }

        /// Iterates capability findings attached to this node.
        pub fn capability_findings(
            &self,
        ) -> impl Iterator<Item = &crate::definition::CapabilityFinding> + '_ {
            self.form
                .definition
                .capability_findings()
                .filter(move |finding| self.finding_attached(finding.instance_location()))
        }

        /// Iterates visible external findings attached to this node, paired with
        /// their source identifiers.
        pub fn external_findings(
            self,
        ) -> impl Iterator<Item = (&'a str, &'a ExternalFinding)> + 'a {
            self.form
                .external_finding_batches
                .iter()
                .flat_map(move |batch| {
                    batch.findings.iter().filter_map(move |finding| {
                        (self.finding_attached(finding.instance_location())
                            && self.form.external_finding_visible(finding))
                        .then_some((batch.source.as_str(), finding))
                    })
                })
        }

        /// Iterates every currently visible finding attached to this node.
        pub fn visible_findings(self) -> impl Iterator<Item = FindingView<'a>> + 'a {
            let target = self.identity;
            let parse = self
                .parse_blocker()
                .map(|kind| FindingView::Parse { target, kind });
            let validation = self.form.validation_findings().iter().filter_map({
                move |finding| {
                    (self.finding_attached(finding.instance_location())
                        && self.form.validation_finding_visible(finding))
                    .then_some(FindingView::Validation { target, finding })
                }
            });
            let validation_truncated = (target == self.form.identity(self.form.definition.root()))
                .then(|| match &self.form.validation {
                    validation::Outcome::Invalid {
                        findings,
                        truncated: true,
                    } if self.form.aggregate_validation_outcome_visible() => {
                        Some(FindingView::ValidationFindingsTruncated {
                            target,
                            retained: findings.len(),
                        })
                    }
                    _ => None,
                })
                .flatten();
            let indeterminate = (target == self.form.identity(self.form.definition.root()))
                .then(|| match &self.form.validation {
                    validation::Outcome::Indeterminate(reason)
                        if self.form.aggregate_validation_outcome_visible() =>
                    {
                        Some(FindingView::Indeterminate { target, reason })
                    }
                    _ => None,
                })
                .flatten();
            let capability = self.form.definition.capability_findings().filter_map({
                move |finding| {
                    self.finding_attached(finding.instance_location())
                        .then_some(FindingView::Capability { target, finding })
                }
            });
            let external =
                self.external_findings()
                    .map(move |(source, finding)| FindingView::External {
                        target,
                        source,
                        finding,
                    });
            validation
                .chain(validation_truncated)
                .chain(indeterminate)
                .chain(capability)
                .chain(external)
                .chain(parse)
        }

        fn finding_attached(&self, location: &JsonPointer) -> bool {
            match self.binding() {
                Some(binding) => binding.pointer() == location,
                None => {
                    self.identity == self.form.identity(self.form.definition.root())
                        && self.form.identity_for_binding(location.as_str()).is_none()
                }
            }
        }

        /// Returns whether the user has blurred this control in the current state.
        pub fn is_touched(&self) -> bool {
            self.binding()
                .and_then(|binding| self.form.engine.control(binding.as_str()))
                .is_some_and(|control| control.is_touched())
        }

        /// Returns whether the bound value differs from the form's baseline.
        pub fn is_dirty(&self) -> bool {
            self.binding()
                .is_some_and(|binding| self.form.engine.bound_value_is_dirty(binding.as_str()))
        }

        /// Borrows the canonical value at the current binding, or `None` if the
        /// node is unbound or the path is absent.
        pub fn current_data(&self) -> Option<&'a Value> {
            let binding = self.binding()?;
            jsonptr::Pointer::parse(binding.as_str())
                .ok()?
                .resolve(self.form.form_data())
                .ok()
        }

        /// Classifies canonical data for scalar controls without validating the
        /// complete form.
        pub fn value_state(&self) -> Option<ScalarValueState> {
            let definition = self.definition();
            let semantic_kind = definition.semantic_kind()?;
            if !matches!(
                semantic_kind,
                SemanticKind::String
                    | SemanticKind::Number
                    | SemanticKind::Integer
                    | SemanticKind::Boolean
                    | SemanticKind::Null
                    | SemanticKind::Choice
            ) {
                return None;
            }
            let Some(value) = self.current_data() else {
                return Some(ScalarValueState::Missing);
            };
            if value.is_null() {
                return Some(ScalarValueState::Null);
            }
            let compatible = self.binding().is_some_and(|binding| {
                self.form
                    .engine
                    .control_accepts_value(binding.as_str(), value)
                    .unwrap_or(false)
            });
            if compatible && value.as_str() == Some("") {
                return Some(ScalarValueState::Empty);
            }
            Some(if compatible {
                ScalarValueState::Compatible
            } else {
                ScalarValueState::Incompatible
            })
        }

        /// Returns the choice semantically equal to current canonical data.
        pub fn selected_choice(&self) -> Option<crate::definition::ChoiceOptionView<'a>> {
            let current = self.current_data()?;
            self.definition()
                .choice_options()
                .find(|option| engine::json_values_equal(option.value(), current))
        }

        /// Formats current scalar data for display, preferring an edit buffer and
        /// then a compiled choice label.
        pub fn display_text(&self) -> Option<String> {
            if let Some(buffer) = self.edit_buffer() {
                return Some(buffer.to_owned());
            }
            let definition = self.definition();
            let value = self.current_data()?;
            match definition.semantic_kind()? {
                SemanticKind::String => value.as_str().map(str::to_owned),
                SemanticKind::Number | SemanticKind::Integer => {
                    value.as_number().map(ToString::to_string)
                }
                SemanticKind::Boolean => value.as_bool().map(|value| value.to_string()),
                SemanticKind::Choice | SemanticKind::Null => self
                    .selected_choice()
                    .map(|option| option.label().to_owned())
                    .or_else(|| match value {
                        Value::Null => Some("null".to_owned()),
                        Value::Bool(value) => Some(value.to_string()),
                        Value::Number(value) => Some(value.to_string()),
                        Value::String(value) => Some(value.clone()),
                        Value::Array(_) | Value::Object(_) => None,
                    }),
                _ => None,
            }
        }
    }

    /// The current absolute JSON binding of a runtime node.
    ///
    /// Bindings for array items can change after topology edits; retain the
    /// [`ItemIdentity`] rather than this pointer when tracking an item over time.
    #[derive(Clone)]
    pub struct CurrentBinding {
        pointer: JsonPointer,
        item: Option<ItemIdentity>,
    }

    impl CurrentBinding {
        pub fn pointer(&self) -> &JsonPointer {
            &self.pointer
        }

        fn as_str(&self) -> &str {
            self.pointer.as_str()
        }

        pub fn item(&self) -> Option<ItemIdentity> {
            self.item
        }
    }

    /// Submission or presentation status of schema validation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ValidationOutcomeView<'a> {
        Valid,
        Invalid {
            findings: &'a [ValidationFinding],
            truncated: bool,
        },
        Indeterminate(&'a IndeterminateReason),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct IndeterminateReason {
        code: String,
    }

    impl IndeterminateReason {
        pub(crate) fn new(code: impl Into<String>) -> Self {
            Self { code: code.into() }
        }

        /// Returns the stable machine-readable reason code.
        ///
        /// Production builds currently return `validator-evaluation-failed`.
        /// Repository qualification builds may also return the test-only
        /// `injected-validator-failure`. Existing codes retain their meaning within
        /// a compatible crate release; future compatible releases may add codes, so
        /// consumers must handle unknown values.
        pub fn code(&self) -> &str {
            &self.code
        }
    }

    /// Opaque identity of one current node instance in one [`Form`].
    ///
    /// Identities must not be transferred between forms. An identity remains
    /// stable while its node exists, including across array reordering.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct InstanceIdentity {
        form: u64,
        node: u64,
        item: Option<ItemIdentity>,
    }

    /// Opaque, form-scoped identity of an array item independent of its index.
    ///
    /// It survives moves and index shifts but becomes invalid when the item is
    /// removed or when an authoritative replacement creates fresh topology.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ItemIdentity {
        form: u64,
        local: u64,
    }

    /// Form-scoped revision of canonical JSON data.
    ///
    /// Equality is meaningful only for the same form. It advances when canonical
    /// data changes and on every successful reinitialization, and is used to
    /// reject stale external findings.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct DataRevision {
        form: u64,
        revision: u64,
    }

    /// Form-scoped revision of all observable data and interaction state.
    ///
    /// It may advance while [`DataRevision`] stays fixed, for example after blur,
    /// an invalid edit buffer, or a finding-visibility change.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct StateRevision {
        form: u64,
        revision: u64,
    }

    /// A snapshot of schema-aware user operations currently allowed on a node.
    ///
    /// Permissions can become stale after any mutation and are always rechecked by
    /// [`UserActions`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct AllowedOperations(u32);

    impl AllowedOperations {
        const INPUT_TEXT: Self = Self(1);
        const SET_VALUE: Self = Self(1 << 1);
        const SET_NULL: Self = Self(1 << 2);
        const REMOVE_VALUE: Self = Self(1 << 3);
        const REPLACE_VALUE: Self = Self(1 << 4);
        const MATERIALIZE: Self = Self(1 << 5);
        const APPEND_ITEM: Self = Self(1 << 6);
        const REMOVE_ITEM: Self = Self(1 << 7);
        const MOVE_ITEM: Self = Self(1 << 8);

        pub fn can_input_text(self) -> bool {
            self.0 & Self::INPUT_TEXT.0 != 0
        }

        pub fn can_set_value(self) -> bool {
            self.0 & Self::SET_VALUE.0 != 0
        }

        pub fn can_set_null(self) -> bool {
            self.0 & Self::SET_NULL.0 != 0
        }

        pub fn can_remove_value(self) -> bool {
            self.0 & Self::REMOVE_VALUE.0 != 0
        }

        pub fn can_replace_value(self) -> bool {
            self.0 & Self::REPLACE_VALUE.0 != 0
        }

        pub fn can_materialize(self) -> bool {
            self.0 & Self::MATERIALIZE.0 != 0
        }

        pub fn can_append_item(self) -> bool {
            self.0 & Self::APPEND_ITEM.0 != 0
        }

        pub fn can_remove_item(self) -> bool {
            self.0 & Self::REMOVE_ITEM.0 != 0
        }

        pub fn can_move_item(self) -> bool {
            self.0 & Self::MOVE_ITEM.0 != 0
        }
    }

    impl std::ops::BitOrAssign for AllowedOperations {
        fn bitor_assign(&mut self, rhs: Self) {
            self.0 |= rhs.0;
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ScalarValueState {
        Missing,
        Null,
        Empty,
        Compatible,
        Incompatible,
    }

    fn definition_has_concrete_value(definition: DefinitionNodeView<'_>) -> bool {
        match definition.semantic_kind() {
            Some(
                SemanticKind::String
                | SemanticKind::Number
                | SemanticKind::Integer
                | SemanticKind::Boolean,
            ) => true,
            Some(SemanticKind::Choice) => definition
                .choice_options()
                .any(|option| !option.value().is_null()),
            _ => false,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum FindingVisibility {
        Immediate,
        TouchedOrSubmission,
        SubmissionOnly,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FindingVisibilityPolicy {
        validation: FindingVisibility,
        external: FindingVisibility,
    }

    impl FindingVisibilityPolicy {
        /// Configures validation and external-finding presentation independently.
        pub fn new(validation: FindingVisibility, external: FindingVisibility) -> Self {
            Self {
                validation,
                external,
            }
        }

        /// Returns the validation-finding visibility policy.
        pub fn validation(self) -> FindingVisibility {
            self.validation
        }

        /// Returns the external-finding visibility policy.
        pub fn external(self) -> FindingVisibility {
            self.external
        }

        /// Replaces only validation-finding visibility.
        pub fn with_validation(mut self, validation: FindingVisibility) -> Self {
            self.validation = validation;
            self
        }

        /// Replaces only external-finding visibility.
        pub fn with_external(mut self, external: FindingVisibility) -> Self {
            self.external = external;
            self
        }
    }

    /// Bounds incoming and active external findings and their untrusted parameters.
    ///
    /// Incoming limits apply to each raw batch before sorting and deduplication.
    /// Active limits apply to canonical findings retained across all sources.
    /// Limits are inclusive and enforced atomically; a rejected replacement does
    /// not disturb the previous batch.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ExternalFindingLimits {
        max_active_findings: usize,
        max_active_bytes: usize,
        max_incoming_findings: usize,
        max_incoming_bytes: usize,
        max_parameter_depth: usize,
        max_parameter_nodes: usize,
        max_parameter_collection_length: usize,
        max_parameter_scalar_bytes: usize,
    }

    impl ExternalFindingLimits {
        /// Creates limits for aggregate active finding count and encoded bytes.
        ///
        /// Raw incoming count and byte limits default to four times their active
        /// counterparts using saturating multiplication, allowing ordinary
        /// duplicate-heavy batches while bounding pre-canonicalization work.
        /// Per-parameter limits retain their bounded release defaults.
        pub fn new(max_active_findings: NonZeroUsize, max_active_bytes: NonZeroUsize) -> Self {
            let max_active_findings = max_active_findings.get();
            let max_active_bytes = max_active_bytes.get();
            Self {
                max_active_findings,
                max_active_bytes,
                max_incoming_findings: max_active_findings
                    .saturating_mul(DEFAULT_INCOMING_EXTERNAL_FINDING_MULTIPLIER),
                max_incoming_bytes: max_active_bytes
                    .saturating_mul(DEFAULT_INCOMING_EXTERNAL_FINDING_MULTIPLIER),
                max_parameter_depth: DEFAULT_MAX_EXTERNAL_PARAMETER_DEPTH,
                max_parameter_nodes: DEFAULT_MAX_EXTERNAL_PARAMETER_NODES,
                max_parameter_collection_length: DEFAULT_MAX_EXTERNAL_PARAMETER_COLLECTION_LENGTH,
                max_parameter_scalar_bytes: DEFAULT_MAX_EXTERNAL_PARAMETER_SCALAR_BYTES,
            }
        }

        /// Sets the maximum finding count in one raw incoming batch.
        ///
        /// This independent pre-canonicalization limit bounds work even when many
        /// findings would later be removed as duplicates.
        pub fn max_incoming_findings(mut self, maximum: usize) -> Self {
            self.max_incoming_findings = maximum;
            self
        }

        /// Sets the maximum encoded bytes in one raw incoming batch.
        ///
        /// This counts the source once plus every finding's code, instance pointer,
        /// and JSON-encoded parameters. The source is also counted for an empty
        /// removal batch; only retained active-byte accounting treats that batch as
        /// zero bytes.
        pub fn max_incoming_bytes(mut self, maximum: usize) -> Self {
            self.max_incoming_bytes = maximum;
            self
        }

        /// Sets the maximum nesting depth of one finding's parameters.
        pub fn max_parameter_depth(mut self, maximum: usize) -> Self {
            self.max_parameter_depth = maximum;
            self
        }

        /// Sets the maximum JSON node count in one finding's parameters.
        pub fn max_parameter_nodes(mut self, maximum: usize) -> Self {
            self.max_parameter_nodes = maximum;
            self
        }

        /// Sets the maximum length of one parameter object or array.
        pub fn max_parameter_collection_length(mut self, maximum: usize) -> Self {
            self.max_parameter_collection_length = maximum;
            self
        }

        /// Sets the maximum encoded bytes of one parameter scalar.
        pub fn max_parameter_scalar_bytes(mut self, maximum: usize) -> Self {
            self.max_parameter_scalar_bytes = maximum;
            self
        }

        pub fn max_active_findings(self) -> usize {
            self.max_active_findings
        }

        pub fn max_active_bytes(self) -> usize {
            self.max_active_bytes
        }

        /// Returns the inclusive raw incoming finding-count maximum.
        pub fn incoming_findings(self) -> usize {
            self.max_incoming_findings
        }

        /// Returns the inclusive raw incoming encoded-byte maximum.
        ///
        /// The source is counted once for every incoming batch, including an empty
        /// removal batch.
        pub fn incoming_bytes(self) -> usize {
            self.max_incoming_bytes
        }

        pub fn parameter_depth(self) -> usize {
            self.max_parameter_depth
        }

        pub fn parameter_nodes(self) -> usize {
            self.max_parameter_nodes
        }

        pub fn parameter_collection_length(self) -> usize {
            self.max_parameter_collection_length
        }

        pub fn parameter_scalar_bytes(self) -> usize {
            self.max_parameter_scalar_bytes
        }
    }

    impl Default for ExternalFindingLimits {
        fn default() -> Self {
            Self {
                max_active_findings: DEFAULT_MAX_ACTIVE_EXTERNAL_FINDINGS,
                max_active_bytes: DEFAULT_MAX_ACTIVE_EXTERNAL_FINDING_BYTES,
                max_incoming_findings: DEFAULT_MAX_ACTIVE_EXTERNAL_FINDINGS
                    * DEFAULT_INCOMING_EXTERNAL_FINDING_MULTIPLIER,
                max_incoming_bytes: DEFAULT_MAX_ACTIVE_EXTERNAL_FINDING_BYTES
                    * DEFAULT_INCOMING_EXTERNAL_FINDING_MULTIPLIER,
                max_parameter_depth: DEFAULT_MAX_EXTERNAL_PARAMETER_DEPTH,
                max_parameter_nodes: DEFAULT_MAX_EXTERNAL_PARAMETER_NODES,
                max_parameter_collection_length: DEFAULT_MAX_EXTERNAL_PARAMETER_COLLECTION_LENGTH,
                max_parameter_scalar_bytes: DEFAULT_MAX_EXTERNAL_PARAMETER_SCALAR_BYTES,
            }
        }
    }

    impl Default for FindingVisibilityPolicy {
        fn default() -> Self {
            Self::new(
                FindingVisibility::TouchedOrSubmission,
                FindingVisibility::TouchedOrSubmission,
            )
        }
    }

    /// Schema-aware, limit-checked mutations attributed to an end user.
    ///
    /// Each method rechecks applicable [`NodeView::allowed_operations`] and limits;
    /// data-changing operations revalidate before returning an exact [`Transition`].
    /// A returned error leaves the form unchanged.
    pub struct UserActions<'a> {
        form: &'a mut Form,
    }

    impl UserActions<'_> {
        /// Applies textual input to a string, number, or integer control.
        ///
        /// Strings and parseable numbers update canonical data immediately.
        /// Incomplete or invalid numeric text remains in an edit buffer and changes
        /// only state. Per-buffer, aggregate-buffer, integer-digit, and form-data
        /// limits are checked before mutation.
        pub fn input_text(
            &mut self,
            target: InstanceIdentity,
            text: impl AsRef<str>,
        ) -> Result<Transition, UserOperationError> {
            let binding = self.binding_for_operation(target, AllowedOperations::can_input_text)?;
            let text = text.as_ref();
            if text.len() > self.form.limits.edit_buffer_bytes() {
                return Err(UserOperationError::ResourceLimit(ResourceLimitError::new(
                    ResourceLimitPhase::Operation,
                    "edit_buffer_bytes",
                    self.form.limits.edit_buffer_bytes(),
                    text.len(),
                    JsonPointer::parse(binding.clone())
                        .expect("control bindings are valid JSON Pointers"),
                )));
            }
            let maximum_digits = self.form.limits.canonical_integer_digits();
            let (active_buffers, total_bytes) = self
                .form
                .engine
                .prospective_edit_buffer_metrics(&binding, text)
                .map_err(map_user_edit_error)?;
            for (dimension, maximum, observed) in [
                (
                    "active_edit_buffers",
                    self.form.limits.active_edit_buffers(),
                    active_buffers,
                ),
                (
                    "total_edit_buffer_bytes",
                    self.form.limits.total_edit_buffer_bytes(),
                    total_bytes,
                ),
            ] {
                if observed > maximum {
                    return Err(UserOperationError::ResourceLimit(ResourceLimitError::new(
                        ResourceLimitPhase::Operation,
                        dimension,
                        maximum,
                        observed,
                        JsonPointer::parse(binding.clone())
                            .expect("control bindings are valid JSON Pointers"),
                    )));
                }
            }
            if let Some(candidate) = self
                .form
                .engine
                .prospective_text_form_data(&binding, text, maximum_digits)
                .map_err(map_user_edit_error)?
            {
                check_runtime_form_data(&self.form.definition, &candidate, self.form.limits)
                    .map_err(UserOperationError::ResourceLimit)?;
            }

            let before = self.form.revisions();
            let findings_before = self.form.visible_validation_findings_by_node();
            let external_targets_before = self.form.visible_external_finding_targets();
            self.form
                .engine
                .edit_text_with_integer_digit_limit(&binding, text, maximum_digits)
                .map_err(map_user_edit_error)?;
            self.form.revalidate_if_data_changed(before.0);
            let after = self.form.revisions();
            if before.0 != after.0 {
                self.form.external_finding_batches.clear();
            }
            Ok(self.form.user_transition(
                target,
                before,
                findings_before,
                external_targets_before,
                after,
            ))
        }

        /// Marks a writable control as touched and finalizes its edit buffer.
        ///
        /// This can change interaction state but does not bypass parsing or schema
        /// permissions.
        pub fn blur(&mut self, target: InstanceIdentity) -> Result<Transition, UserOperationError> {
            if target.form != self.form.id {
                return Err(UserOperationError::UnknownTarget);
            }
            if self
                .form
                .node(target)
                .is_some_and(|node| node.is_read_only())
            {
                return Err(UserOperationError::OperationNotAllowed);
            }
            let binding = self
                .form
                .node(target)
                .filter(|node| {
                    node.definition().kind() == crate::definition::DefinitionNodeKind::Control
                })
                .and_then(|node| node.binding())
                .ok_or(UserOperationError::OperationNotAllowed)?
                .pointer()
                .as_str()
                .to_owned();
            let before = self.form.revisions();
            self.form
                .engine
                .blur(&binding)
                .map_err(|_| UserOperationError::UnknownTarget)?;
            let after = self.form.revisions();
            let changed = (before != after).then_some(target).into_iter().collect();
            Ok(self.form.transition(before, after, changed))
        }

        /// Sets a non-null scalar value accepted by the target control.
        ///
        /// Use [`Self::set_null`] for null and [`Self::replace_value`] to recover
        /// from an incompatible current value.
        pub fn set_value(
            &mut self,
            target: InstanceIdentity,
            value: Value,
        ) -> Result<Transition, UserOperationError> {
            let binding = self.binding_for_operation(target, AllowedOperations::can_set_value)?;
            if !self
                .form
                .engine
                .control_accepts_value(&binding, &value)
                .unwrap_or(false)
                || value.is_null()
            {
                return Err(UserOperationError::OperationNotAllowed);
            }
            check_runtime_input_value(&value, &binding, self.form.limits)
                .map_err(UserOperationError::ResourceLimit)?;
            self.apply_scalar_edit(target, binding, |form, binding| {
                form.set_value(binding, &value)
            })
        }

        /// Sets a null-capable scalar control to JSON null.
        pub fn set_null(
            &mut self,
            target: InstanceIdentity,
        ) -> Result<Transition, UserOperationError> {
            let binding = self.binding_for_operation(target, AllowedOperations::can_set_null)?;
            self.apply_scalar_edit(target, binding, |form, binding| {
                form.set_value(binding, &Value::Null)
            })
        }

        /// Removes an optional bound property or structure.
        ///
        /// Structural removal may remove runtime descendants; their identities are
        /// reported by [`Transition::removed`].
        pub fn remove_value(
            &mut self,
            target: InstanceIdentity,
        ) -> Result<Transition, UserOperationError> {
            let binding =
                self.binding_for_operation(target, AllowedOperations::can_remove_value)?;
            if self.form.node(target).is_some_and(|node| {
                matches!(
                    node.definition().semantic_kind(),
                    Some(SemanticKind::FixedObject | SemanticKind::HomogeneousArray)
                )
            }) {
                return self.apply_structural_edit(binding, engine::Form::remove_value);
            }
            self.apply_scalar_edit(target, binding, engine::Form::remove_value)
        }

        /// Replaces incompatible or write-only current data with a compatible
        /// non-null scalar, object, or array as permitted by the target.
        ///
        /// Structural replacement creates fresh array-item identities and reports
        /// topology changes. The replacement is limit-checked atomically.
        pub fn replace_value(
            &mut self,
            target: InstanceIdentity,
            value: Value,
        ) -> Result<Transition, UserOperationError> {
            let binding =
                self.binding_for_operation(target, AllowedOperations::can_replace_value)?;
            if self.form.node(target).is_some_and(|node| {
                matches!(
                    node.definition().semantic_kind(),
                    Some(SemanticKind::FixedObject | SemanticKind::HomogeneousArray)
                )
            }) {
                let kind = self
                    .form
                    .node(target)
                    .and_then(|node| node.definition().semantic_kind());
                if (kind == Some(SemanticKind::FixedObject) && !value.is_object())
                    || (kind == Some(SemanticKind::HomogeneousArray) && !value.is_array())
                {
                    return Err(UserOperationError::OperationNotAllowed);
                }
                check_runtime_input_value(&value, &binding, self.form.limits)
                    .map_err(UserOperationError::ResourceLimit)?;
                return self.apply_structural_edit(binding, |form, binding| {
                    form.replace_structure(binding, &value)
                });
            }
            if !self
                .form
                .engine
                .control_accepts_value(&binding, &value)
                .unwrap_or(false)
                || value.is_null()
            {
                return Err(UserOperationError::OperationNotAllowed);
            }
            check_runtime_input_value(&value, &binding, self.form.limits)
                .map_err(UserOperationError::ResourceLimit)?;
            self.apply_scalar_edit(target, binding, |form, binding| {
                form.set_value(binding, &value)
            })
        }

        fn binding_for_operation(
            &self,
            target: InstanceIdentity,
            allowed: impl FnOnce(AllowedOperations) -> bool,
        ) -> Result<String, UserOperationError> {
            if target.form != self.form.id {
                return Err(UserOperationError::UnknownTarget);
            }
            let node = self
                .form
                .node(target)
                .ok_or(UserOperationError::UnknownTarget)?;
            if !allowed(node.allowed_operations()) {
                return Err(UserOperationError::OperationNotAllowed);
            }
            node.binding()
                .map(|binding| binding.pointer().as_str().to_owned())
                .ok_or(UserOperationError::OperationNotAllowed)
        }

        fn apply_scalar_edit(
            &mut self,
            target: InstanceIdentity,
            binding: String,
            edit: impl FnOnce(&mut engine::Form, &str) -> Result<(), engine::EditError>,
        ) -> Result<Transition, UserOperationError> {
            let before = self.form.revisions();
            let findings_before = self.form.visible_validation_findings_by_node();
            let external_targets_before = self.form.visible_external_finding_targets();
            let mut candidate = self.form.engine.clone();
            edit(&mut candidate, &binding).map_err(map_user_edit_error)?;
            check_runtime_form_data(
                &self.form.definition,
                candidate.form_data(),
                self.form.limits,
            )
            .map_err(UserOperationError::ResourceLimit)?;
            self.form.engine = candidate;
            self.form.revalidate_if_data_changed(before.0);
            let after = self.form.revisions();
            if before.0 != after.0 {
                self.form.external_finding_batches.clear();
            }
            Ok(self.form.user_transition(
                target,
                before,
                findings_before,
                external_targets_before,
                after,
            ))
        }

        fn apply_structural_edit(
            &mut self,
            binding: String,
            edit: impl FnOnce(&mut engine::Form, &str) -> Result<(), engine::EditError>,
        ) -> Result<Transition, UserOperationError> {
            let before = self.form.revisions();
            let identities_before = self.form.instance_identities();
            let findings_before = self.form.visible_validation_findings_by_node();
            let external_targets_before = self.form.visible_external_finding_targets();
            let mut candidate = self.form.engine.clone();
            edit(&mut candidate, &binding).map_err(map_user_edit_error)?;
            check_runtime_form_data(
                &self.form.definition,
                candidate.form_data(),
                self.form.limits,
            )
            .map_err(UserOperationError::ResourceLimit)?;
            self.form.engine = candidate;
            self.form.revalidate_if_data_changed(before.0);
            let after = self.form.revisions();
            if before.0 != after.0 {
                self.form.external_finding_batches.clear();
            }

            let target_binding = JsonPointer::parse(binding)
                .expect("structural control bindings are valid JSON Pointers");
            let findings_before = findings_before.into_iter().collect::<HashMap<_, _>>();
            let mut changed = self
                .form
                .visible_validation_findings_by_node()
                .into_iter()
                .filter_map(|(identity, findings_after)| {
                    let structurally_affected = self
                        .form
                        .node(identity)
                        .and_then(|node| node.binding())
                        .is_some_and(|binding| target_binding.intersects(binding.pointer()));
                    (structurally_affected
                        || findings_before.get(&identity) != Some(&findings_after))
                    .then_some(identity)
                })
                .collect::<Vec<_>>();
            changed.extend(external_targets_before);
            Ok(self
                .form
                .topology_transition(before, after, identities_before, changed))
        }

        /// Creates a missing object or array from the definition's compiled seed.
        ///
        /// The operation is available only when the parent exists as an object and
        /// the definition has a creation seed.
        pub fn materialize(
            &mut self,
            target: InstanceIdentity,
        ) -> Result<Transition, UserOperationError> {
            let binding = self.binding_for_operation(target, AllowedOperations::can_materialize)?;
            let seed = self
                .form
                .definition
                .node(DefinitionNodeId(target.node))
                .and_then(|definition| definition.creation_seed())
                .cloned()
                .ok_or(UserOperationError::OperationNotAllowed)?;
            self.apply_structural_edit(binding, |form, binding| {
                form.materialize_structure(binding, &seed)
            })
        }

        /// Appends a newly seeded item to a homogeneous array.
        ///
        /// The new item and its descendant identities appear in the returned
        /// transition's changed set.
        pub fn append_item(
            &mut self,
            array: InstanceIdentity,
        ) -> Result<Transition, UserOperationError> {
            let binding = self.binding_for_operation(array, AllowedOperations::can_append_item)?;
            let template = self
                .form
                .definition
                .array_template(DefinitionNodeId(array.node))
                .ok_or(UserOperationError::OperationNotAllowed)?;
            let before = self.form.revisions();
            let findings_before = self.form.visible_validation_findings_by_node();
            let external_targets_before = self.form.visible_external_finding_targets();
            let mut candidate = self.form.engine.clone();
            let item = candidate
                .append_array_item(&binding)
                .map_err(map_user_edit_error)?;
            check_runtime_form_data(
                &self.form.definition,
                candidate.form_data(),
                self.form.limits,
            )
            .map_err(UserOperationError::ResourceLimit)?;
            self.form.engine = candidate;
            self.form.revalidate();
            self.form.external_finding_batches.clear();
            let after = self.form.revisions();
            let mut changed = vec![array];
            changed.extend(
                self.form
                    .item_subtree_identities(template, self.form.item(item)),
            );
            changed.extend(external_targets_before);
            let changed = self
                .form
                .host_changed_nodes(&changed, before, findings_before, after);
            Ok(Transition {
                before_data: before.0,
                after_data: after.0,
                before_state: before.1,
                after_state: after.1,
                changed,
                removed: Vec::new(),
            })
        }

        /// Inserts a newly seeded item immediately before an existing stable item.
        ///
        /// `before` must belong to this form and currently be in `array`; existing
        /// item identities survive index shifts.
        pub fn insert_item_before(
            &mut self,
            array: InstanceIdentity,
            before: ItemIdentity,
        ) -> Result<Transition, UserOperationError> {
            if before.form != self.form.id {
                return Err(UserOperationError::UnknownTarget);
            }
            let binding = self.binding_for_operation(array, AllowedOperations::can_append_item)?;
            let template = self
                .form
                .definition
                .array_template(DefinitionNodeId(array.node))
                .ok_or(UserOperationError::OperationNotAllowed)?;
            let before_revision = self.form.revisions();
            let findings_before = self.form.visible_validation_findings_by_node();
            let external_targets_before = self.form.visible_external_finding_targets();
            let mut candidate = self.form.engine.clone();
            let inserted = candidate
                .insert_array_item_before(&binding, before.local)
                .map_err(map_user_edit_error)?;
            check_runtime_form_data(
                &self.form.definition,
                candidate.form_data(),
                self.form.limits,
            )
            .map_err(UserOperationError::ResourceLimit)?;
            self.form.engine = candidate;
            self.form.revalidate();
            self.form.external_finding_batches.clear();
            let after = self.form.revisions();
            let mut changed = vec![array];
            changed.extend(
                self.form
                    .item_subtree_identities(template, self.form.item(inserted.identity)),
            );
            changed.extend(inserted.shifted.into_iter().flat_map(|item| {
                self.form
                    .item_subtree_identities(template, self.form.item(item))
            }));
            changed.extend(external_targets_before);
            let changed =
                self.form
                    .host_changed_nodes(&changed, before_revision, findings_before, after);
            Ok(Transition {
                before_data: before_revision.0,
                after_data: after.0,
                before_state: before_revision.1,
                after_state: after.1,
                changed,
                removed: Vec::new(),
            })
        }

        /// Removes an existing stable item from a homogeneous array.
        ///
        /// The removed item's complete runtime subtree is listed by
        /// [`Transition::removed`]; shifted surviving identities remain stable.
        pub fn remove_item(
            &mut self,
            array: InstanceIdentity,
            item: ItemIdentity,
        ) -> Result<Transition, UserOperationError> {
            if item.form != self.form.id {
                return Err(UserOperationError::UnknownTarget);
            }
            let binding = self.binding_for_operation(array, AllowedOperations::can_remove_item)?;
            let template = self
                .form
                .definition
                .array_template(DefinitionNodeId(array.node))
                .ok_or(UserOperationError::OperationNotAllowed)?;
            let removed_identities = self.form.item_subtree_identities(template, item);
            let removed_identity = removed_identities[0];
            if self.form.node(removed_identity).is_none() {
                return Err(UserOperationError::UnknownTarget);
            }
            let before = self.form.revisions();
            let findings_before = self.form.visible_validation_findings_by_node();
            let external_targets_before = self.form.visible_external_finding_targets();
            let removed = self
                .form
                .engine
                .remove_array_item(&binding, item.local)
                .map_err(map_user_edit_error)?;
            self.form.revalidate();
            self.form.external_finding_batches.clear();
            let after = self.form.revisions();
            let mut changed = vec![array];
            changed.extend(removed.shifted.into_iter().flat_map(|item| {
                self.form
                    .item_subtree_identities(template, self.form.item(item))
            }));
            changed.extend(
                external_targets_before
                    .into_iter()
                    .filter(|identity| !removed_identities.contains(identity)),
            );
            let changed = self
                .form
                .host_changed_nodes(&changed, before, findings_before, after);
            debug_assert_eq!(removed.identity, item.local);
            Ok(Transition {
                before_data: before.0,
                after_data: after.0,
                before_state: before.1,
                after_state: after.1,
                changed,
                removed: removed_identities,
            })
        }

        /// Swaps an existing array item with its predecessor.
        ///
        /// Moving the first item is rejected rather than treated as a no-op.
        pub fn move_item_up(
            &mut self,
            array: InstanceIdentity,
            item: ItemIdentity,
        ) -> Result<Transition, UserOperationError> {
            self.move_item(array, item, engine::Form::move_array_item_up)
        }

        /// Swaps an existing array item with its successor.
        ///
        /// Moving the last item is rejected rather than treated as a no-op.
        pub fn move_item_down(
            &mut self,
            array: InstanceIdentity,
            item: ItemIdentity,
        ) -> Result<Transition, UserOperationError> {
            self.move_item(array, item, engine::Form::move_array_item_down)
        }

        fn move_item(
            &mut self,
            array: InstanceIdentity,
            item: ItemIdentity,
            operation: impl FnOnce(
                &mut engine::Form,
                &str,
                u64,
            ) -> Result<engine::MovedArrayItem, engine::EditError>,
        ) -> Result<Transition, UserOperationError> {
            if item.form != self.form.id {
                return Err(UserOperationError::UnknownTarget);
            }
            let binding = self.binding_for_operation(array, AllowedOperations::can_move_item)?;
            let template = self
                .form
                .definition
                .array_template(DefinitionNodeId(array.node))
                .ok_or(UserOperationError::OperationNotAllowed)?;
            let before = self.form.revisions();
            let findings_before = self.form.visible_validation_findings_by_node();
            let external_targets_before = self.form.visible_external_finding_targets();
            let moved = operation(&mut self.form.engine, &binding, item.local)
                .map_err(map_user_edit_error)?;
            debug_assert_eq!(moved.identity, item.local);
            if moved.data_changed {
                self.form.revalidate();
                self.form.external_finding_batches.clear();
            } else {
                self.form
                    .move_external_findings(&binding, moved.from, moved.to);
            }
            let after = self.form.revisions();
            let mut changed = vec![array];
            changed.extend(self.form.item_subtree_identities(template, item));
            changed.extend(
                self.form
                    .item_subtree_identities(template, self.form.item(moved.displaced)),
            );
            if moved.data_changed {
                changed.extend(external_targets_before);
            }
            let changed = self
                .form
                .host_changed_nodes(&changed, before, findings_before, after);
            Ok(Transition {
                before_data: before.0,
                after_data: after.0,
                before_state: before.1,
                after_state: after.1,
                changed,
                removed: Vec::new(),
            })
        }
    }

    fn map_user_edit_error(error: engine::EditError) -> UserOperationError {
        match error {
            engine::EditError::UnknownControl(_) => UserOperationError::UnknownTarget,
            engine::EditError::UnresolvedControl(_) | engine::EditError::OperationNotAllowed(_) => {
                UserOperationError::OperationNotAllowed
            }
        }
    }

    #[derive(Clone)]
    struct HostArrayTopology {
        binding: PointerBuf,
        original: Option<Value>,
        original_items: Vec<engine::HostArrayItem>,
        items: Vec<engine::HostArrayItem>,
        authoritative_replacement: bool,
    }

    #[derive(Clone, Copy)]
    enum HostArrayMove {
        Up,
        Down,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum UnmatchedFinding {
        Preserve,
        Remove,
    }

    /// An isolated host-authored candidate used by [`Form::try_transact`] and
    /// [`Form::transact`].
    ///
    /// Operations intentionally return `()`: an invalid target, stale item
    /// identity, invalid shape, or operation-limit violation poisons the candidate
    /// and is reported when the enclosing transaction commits. Later operations
    /// become no-ops, and no partial mutation reaches the form.
    pub struct HostTransaction<'a> {
        form_id: u64,
        candidate: &'a mut Value,
        valid: &'a mut bool,
        writes: &'a mut Vec<JsonPointer>,
        item_writes: &'a mut Vec<engine::HostItemWrite>,
        array_topologies: &'a mut [HostArrayTopology],
        operation_count: &'a mut usize,
        maximum_operations: usize,
        commit_error: &'a mut Option<HostCommitError>,
    }

    impl HostTransaction<'_> {
        /// Replaces an existing value or inserts a missing property into an
        /// existing object.
        ///
        /// Missing intermediate containers and array-index insertion are invalid.
        /// Arrays whose final value differs semantically from the transaction's
        /// original array receive fresh item identities. Arrays restored unchanged
        /// retain their topology; use dedicated array methods to preserve identities
        /// through structural changes.
        pub fn set(&mut self, pointer: &JsonPointer, value: Value) {
            if !self.begin_operation() {
                return;
            }
            let write = pointer.clone();
            let Ok(pointer) = jsonptr::Pointer::parse(pointer.as_str()) else {
                *self.valid = false;
                return;
            };
            let affected_arrays = self.authoritative_array_values(&write);
            let item_writes = self.item_writes_for(&write);
            if pointer.resolve(self.candidate).is_ok() {
                *pointer
                    .resolve_mut(self.candidate)
                    .expect("a resolved candidate pointer remains mutable") = value;
            } else {
                let Some((parent, property)) = pointer.split_back() else {
                    *self.valid = false;
                    return;
                };
                let Some(parent) = parent
                    .resolve_mut(self.candidate)
                    .ok()
                    .and_then(Value::as_object_mut)
                else {
                    *self.valid = false;
                    return;
                };
                parent.insert(property.decoded().into_owned(), value);
            }
            self.update_authoritative_arrays(affected_arrays);
            self.item_writes.extend(item_writes);
            self.writes.push(write);
        }

        /// Removes an existing object property.
        ///
        /// Removing the root, an array index, or a missing path poisons the whole
        /// transaction.
        pub fn remove(&mut self, pointer: &JsonPointer) {
            if !self.begin_operation() {
                return;
            }
            let write = pointer.clone();
            let Ok(pointer) = jsonptr::Pointer::parse(pointer.as_str()) else {
                *self.valid = false;
                return;
            };
            let Some((parent, property)) = pointer.split_back() else {
                *self.valid = false;
                return;
            };
            let affected_arrays = self.authoritative_array_values(&write);
            let item_writes = self.item_writes_for(&write);
            let Some(parent) = parent
                .resolve_mut(self.candidate)
                .ok()
                .and_then(Value::as_object_mut)
            else {
                *self.valid = false;
                return;
            };
            if parent.remove(property.decoded().as_ref()).is_none() {
                *self.valid = false;
                return;
            }
            self.update_authoritative_arrays(affected_arrays);
            self.item_writes.extend(item_writes);
            self.writes.push(
                JsonPointer::parse(pointer.to_string())
                    .expect("a parsed JSON Pointer remains valid when converted to a string"),
            );
        }

        /// Replaces the entire candidate form data.
        ///
        /// The value must be an object and satisfy runtime limits at commit. Array
        /// item identities are retained only where replacement is semantically
        /// unchanged; changed arrays receive fresh identities.
        pub fn replace_all(&mut self, value: Value) {
            if !self.begin_operation() {
                return;
            }
            let root = JsonPointer::parse("").expect("the root JSON Pointer is always valid");
            let affected_arrays = self.authoritative_array_values(&root);
            let item_writes = self.item_writes_for(&root);
            *self.candidate = value;
            self.update_authoritative_arrays(affected_arrays);
            self.item_writes.extend(item_writes);
            self.writes.push(root);
        }

        /// Appends `value` to a compiled homogeneous array as a fresh item.
        pub fn append_item(&mut self, array: &JsonPointer, value: Value) {
            if !self.begin_operation() {
                return;
            }
            let Some((values, topology)) = self.array_parts(array) else {
                *self.valid = false;
                return;
            };
            if values.len() != topology.items.len() {
                *self.valid = false;
                return;
            }
            values.push(value);
            topology.items.push(engine::HostArrayItem::Fresh);
        }

        /// Inserts a fresh value before an existing stable array item.
        ///
        /// The array binding and item identity must belong to this form and the
        /// same current array topology.
        pub fn insert_item_before(
            &mut self,
            array: &JsonPointer,
            before: ItemIdentity,
            value: Value,
        ) {
            if !self.begin_operation() {
                return;
            }
            if before.form != self.form_id {
                *self.valid = false;
                return;
            }
            let Some((values, topology)) = self.array_parts(array) else {
                *self.valid = false;
                return;
            };
            let Some(index) = topology
                .items
                .iter()
                .position(|item| *item == engine::HostArrayItem::Existing(before.local))
            else {
                *self.valid = false;
                return;
            };
            if values.len() != topology.items.len() {
                *self.valid = false;
                return;
            }
            values.insert(index, value);
            topology.items.insert(index, engine::HostArrayItem::Fresh);
        }

        /// Removes an existing stable item while preserving identities of survivors.
        pub fn remove_item(&mut self, array: &JsonPointer, item: ItemIdentity) {
            if !self.begin_operation() {
                return;
            }
            if item.form != self.form_id {
                *self.valid = false;
                return;
            }
            let Some((values, topology)) = self.array_parts(array) else {
                *self.valid = false;
                return;
            };
            let Some(index) = topology
                .items
                .iter()
                .position(|candidate| *candidate == engine::HostArrayItem::Existing(item.local))
            else {
                *self.valid = false;
                return;
            };
            if values.len() != topology.items.len() {
                *self.valid = false;
                return;
            }
            values.remove(index);
            topology.items.remove(index);
        }

        /// Moves an existing stable item one position toward the start of its array.
        ///
        /// A missing item or an item already first poisons the transaction.
        pub fn move_item_up(&mut self, array: &JsonPointer, item: ItemIdentity) {
            if !self.begin_operation() {
                return;
            }
            self.move_item(array, item, HostArrayMove::Up);
        }

        /// Moves an existing stable item one position toward the end of its array.
        ///
        /// A missing item or an item already last poisons the transaction.
        pub fn move_item_down(&mut self, array: &JsonPointer, item: ItemIdentity) {
            if !self.begin_operation() {
                return;
            }
            self.move_item(array, item, HostArrayMove::Down);
        }

        fn move_item(&mut self, array: &JsonPointer, item: ItemIdentity, direction: HostArrayMove) {
            if item.form != self.form_id {
                *self.valid = false;
                return;
            }
            let Some((values, topology)) = self.array_parts(array) else {
                *self.valid = false;
                return;
            };
            let Some(index) = topology
                .items
                .iter()
                .position(|candidate| *candidate == engine::HostArrayItem::Existing(item.local))
            else {
                *self.valid = false;
                return;
            };
            let destination = match direction {
                HostArrayMove::Up => index.checked_sub(1),
                HostArrayMove::Down => index
                    .checked_add(1)
                    .filter(|index| *index < topology.items.len()),
            };
            let Some(destination) = destination else {
                *self.valid = false;
                return;
            };
            if values.len() != topology.items.len() {
                *self.valid = false;
                return;
            }
            values.swap(index, destination);
            topology.items.swap(index, destination);
        }

        fn array_parts(
            &mut self,
            array: &JsonPointer,
        ) -> Option<(&mut Vec<Value>, &mut HostArrayTopology)> {
            let pointer = jsonptr::Pointer::parse(array.as_str()).ok()?;
            let index = self
                .array_topologies
                .iter()
                .position(|topology| topology.binding == pointer)?;
            let values = pointer.resolve_mut(self.candidate).ok()?.as_array_mut()?;
            Some((values, &mut self.array_topologies[index]))
        }

        fn authoritative_array_values(&self, write: &JsonPointer) -> Vec<(usize, Option<Value>)> {
            let write = PointerBuf::parse(write.as_str().to_owned())
                .expect("public JSON Pointers are validated during construction");
            self.array_topologies
                .iter()
                .enumerate()
                .filter(|(_, topology)| {
                    topology.binding == write || topology.binding.starts_with(&write)
                })
                .map(|(index, topology)| {
                    (
                        index,
                        topology.binding.resolve(self.candidate).ok().cloned(),
                    )
                })
                .collect()
        }

        fn item_writes_for(&self, write: &JsonPointer) -> Vec<engine::HostItemWrite> {
            let write = PointerBuf::parse(write.as_str().to_owned())
                .expect("public JSON Pointers are validated during construction");
            let mut writes = Vec::new();
            for (array, topology) in self.array_topologies.iter().enumerate() {
                if topology.binding == write || topology.binding.starts_with(&write) {
                    writes.extend(topology.items.iter().filter_map(|item| {
                        let engine::HostArrayItem::Existing(identity) = item else {
                            return None;
                        };
                        Some(engine::HostItemWrite {
                            array,
                            identity: *identity,
                            binding: PointerBuf::new(),
                        })
                    }));
                    continue;
                }
                if !write.starts_with(&topology.binding) {
                    continue;
                }
                for (index, item) in topology.items.iter().enumerate() {
                    let engine::HostArrayItem::Existing(identity) = item else {
                        continue;
                    };
                    let item_binding =
                        PointerBuf::parse(format!("{}/{index}", topology.binding.as_str()))
                            .expect("an array binding with an index is a valid JSON Pointer");
                    let binding = if item_binding == write || item_binding.starts_with(&write) {
                        Some(PointerBuf::new())
                    } else if write.starts_with(&item_binding) {
                        Some(
                            PointerBuf::parse(
                                write.as_str()[item_binding.as_str().len()..].to_owned(),
                            )
                            .expect("a suffix of a descendant JSON Pointer is valid"),
                        )
                    } else {
                        None
                    };
                    if let Some(binding) = binding {
                        writes.push(engine::HostItemWrite {
                            array,
                            identity: *identity,
                            binding,
                        });
                    }
                }
            }
            writes
        }

        fn update_authoritative_arrays(&mut self, before: Vec<(usize, Option<Value>)>) {
            for (index, before) in before {
                let topology = &mut self.array_topologies[index];
                let after = topology.binding.resolve(self.candidate).ok();
                if optional_json_values_equal(before.as_ref(), after) {
                    continue;
                }
                topology.authoritative_replacement =
                    !optional_json_values_equal(topology.original.as_ref(), after);
                topology.items = if topology.authoritative_replacement {
                    vec![
                        engine::HostArrayItem::Fresh;
                        after.and_then(Value::as_array).map_or(0, Vec::len)
                    ]
                } else {
                    topology.original_items.clone()
                };
            }
        }

        fn begin_operation(&mut self) -> bool {
            if !*self.valid || self.commit_error.is_some() {
                return false;
            }
            *self.operation_count += 1;
            if *self.operation_count > self.maximum_operations {
                *self.commit_error = Some(HostCommitError::ResourceLimit(ResourceLimitError::new(
                    ResourceLimitPhase::Operation,
                    "host_operations_per_transaction",
                    self.maximum_operations,
                    *self.operation_count,
                    JsonPointer::parse("").expect("the root JSON Pointer is valid"),
                )));
                return false;
            }
            true
        }
    }

    fn optional_json_values_equal(left: Option<&Value>, right: Option<&Value>) -> bool {
        match (left, right) {
            (Some(left), Some(right)) => engine::json_values_equal(left, right),
            (None, None) => true,
            _ => false,
        }
    }

    /// Revision and node invalidation information for one completed operation.
    ///
    /// `changed` contains extant instances that should be reread; `removed`
    /// contains identities that must no longer be resolved. Both lists are
    /// deduplicated and scoped to the form named by the revisions.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Transition {
        before_data: DataRevision,
        after_data: DataRevision,
        before_state: StateRevision,
        after_state: StateRevision,
        changed: Vec<InstanceIdentity>,
        removed: Vec<InstanceIdentity>,
    }

    impl Transition {
        /// Returns the canonical-data revision observed before the operation.
        pub fn before_data_revision(&self) -> DataRevision {
            self.before_data
        }

        /// Returns the canonical-data revision after the operation committed.
        pub fn after_data_revision(&self) -> DataRevision {
            self.after_data
        }

        /// Returns the observable-state revision observed before the operation.
        pub fn before_state_revision(&self) -> StateRevision {
            self.before_state
        }

        /// Returns the observable-state revision after the operation committed.
        pub fn after_state_revision(&self) -> StateRevision {
            self.after_state
        }

        /// Returns whether neither revisions nor observable node sets changed.
        pub fn is_empty(&self) -> bool {
            self.before_data == self.after_data
                && self.before_state == self.after_state
                && self.changed.is_empty()
                && self.removed.is_empty()
        }

        /// Iterates extant node instances whose observable view may have changed.
        pub fn changed(&self) -> impl Iterator<Item = InstanceIdentity> + '_ {
            self.changed.iter().copied()
        }

        /// Iterates node instances removed by a topology change.
        pub fn removed(&self) -> impl Iterator<Item = InstanceIdentity> + '_ {
            self.removed.iter().copied()
        }
    }

    /// The state transition and ordinary outcome of one submission attempt.
    ///
    /// The transition must be processed even when submission is blocked because
    /// preparation can finalize edit buffers and reveal submission-only findings.
    pub struct SubmissionPreparation {
        transition: Transition,
        outcome: SubmissionOutcome,
    }

    impl SubmissionPreparation {
        /// Borrows the state changes caused by submission preparation.
        pub fn transition(&self) -> &Transition {
            &self.transition
        }

        /// Borrows the ready snapshot or complete blocker collection.
        pub fn outcome(&self) -> &SubmissionOutcome {
            &self.outcome
        }

        /// Consumes the preparation into its transition and outcome.
        pub fn into_parts(self) -> (Transition, SubmissionOutcome) {
            (self.transition, self.outcome)
        }
    }

    /// Ordinary outcome of atomic submission preparation.
    ///
    /// `Ready` contains an owned point-in-time snapshot. `Blocked` contains every
    /// retained parse, validation, capability, and external blocker known at the
    /// end of preparation; neither variant performs transport.
    pub enum SubmissionOutcome {
        Ready(SubmissionSnapshot),
        Blocked(SubmissionBlockers),
    }

    /// Owned canonical data tied to an exact data revision and definition fingerprint.
    ///
    /// Later form edits do not modify the snapshot. Consumers can use both tokens
    /// to reject stale or definition-mismatched submissions.
    #[derive(Debug, Clone)]
    pub struct SubmissionSnapshot {
        form_data: Value,
        data_revision: DataRevision,
        definition_fingerprint: DefinitionFingerprint,
    }

    impl SubmissionSnapshot {
        /// Borrows the canonical JSON object captured for submission.
        pub fn form_data(&self) -> &Value {
            &self.form_data
        }

        /// Returns the exact form data revision captured by this snapshot.
        pub fn data_revision(&self) -> DataRevision {
            self.data_revision
        }

        /// Returns the semantic fingerprint of the definition used to prepare it.
        pub fn definition_fingerprint(&self) -> DefinitionFingerprint {
            self.definition_fingerprint
        }
    }

    /// Complete retained reasons that prevented creation of a submission snapshot.
    pub struct SubmissionBlockers {
        blockers: Vec<SubmissionBlocker>,
    }

    impl SubmissionBlockers {
        /// Iterates blockers in deterministic category order.
        pub fn iter(&self) -> impl Iterator<Item = &SubmissionBlocker> {
            self.blockers.iter()
        }
    }

    /// A structured reason that prevents a submission snapshot.
    #[non_exhaustive]
    pub enum SubmissionBlocker {
        Parse {
            target: InstanceIdentity,
            kind: ParseBlockerKind,
        },
        External {
            source: String,
            finding: ExternalFinding,
        },
        Validation(ValidationFinding),
        ValidationFindingsTruncated {
            retained: usize,
        },
        Indeterminate(IndeterminateReason),
        Capability(crate::definition::CapabilityFinding),
    }

    /// Why an edit buffer cannot currently be converted to canonical JSON data.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ParseBlockerKind {
        InvalidNumber,
        InvalidInteger,
        ResourceLimitExceeded,
    }

    /// Failure to construct form state from a definition and initial JSON value.
    ///
    /// Schema-invalid object data is not an error; it is represented by validation
    /// findings. Non-object roots and resource-limit violations fail atomically.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum FormBuildError {
        FormDataMustBeObject,
        ResourceLimit(ResourceLimitError),
    }

    /// Rejection of one schema-aware end-user operation.
    ///
    /// Unknown includes foreign or removed identities; not allowed reflects the
    /// current schema/data state. Every error leaves the form unchanged.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum UserOperationError {
        UnknownTarget,
        OperationNotAllowed,
        ResourceLimit(ResourceLimitError),
    }

    /// Failure to atomically commit a [`HostTransaction`].
    ///
    /// Invalid operations are deferred by the transaction's `()` methods and
    /// reported here; no candidate writes are committed on either variant.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum HostCommitError {
        InvalidOperation,
        ResourceLimit(ResourceLimitError),
    }

    /// Failure of a fallible host transaction closure or its atomic commit.
    ///
    /// [`Self::Closure`] preserves the caller's error. [`Self::Commit`] reports a
    /// rejected candidate. In both cases the form remains unchanged.
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum TransactionError<E> {
        Closure(E),
        Commit(HostCommitError),
    }

    /// Rejection of an external-finding batch before any state is changed.
    ///
    /// Batches are revision-gated and bounded as untrusted input.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ExternalFindingError {
        StaleRevision {
            current: DataRevision,
            supplied: DataRevision,
        },
        ResourceLimit(ResourceLimitError),
    }

    /// Failure to atomically replace a form's baseline and current data.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum ReinitializeError {
        InvalidFormData,
        ResourceLimit(ResourceLimitError),
    }

    impl fmt::Display for FormBuildError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::FormDataMustBeObject => {
                    formatter.write_str("form data must be a JSON object")
                }
                Self::ResourceLimit(error) => error.fmt(formatter),
            }
        }
    }

    impl fmt::Display for UserOperationError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::UnknownTarget => formatter.write_str("unknown or removed operation target"),
                Self::OperationNotAllowed => {
                    formatter.write_str("operation is not allowed for the current target")
                }
                Self::ResourceLimit(error) => error.fmt(formatter),
            }
        }
    }

    impl fmt::Display for HostCommitError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidOperation => formatter.write_str("invalid host transaction operation"),
                Self::ResourceLimit(error) => error.fmt(formatter),
            }
        }
    }

    impl<E: fmt::Display> fmt::Display for TransactionError<E> {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Closure(error) => write!(formatter, "transaction closure failed: {error}"),
                Self::Commit(error) => error.fmt(formatter),
            }
        }
    }

    impl fmt::Display for ExternalFindingError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::StaleRevision { .. } => {
                    formatter.write_str("external findings target a stale data revision")
                }
                Self::ResourceLimit(error) => error.fmt(formatter),
            }
        }
    }

    impl fmt::Display for ReinitializeError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidFormData => formatter.write_str("invalid replacement form data"),
                Self::ResourceLimit(error) => error.fmt(formatter),
            }
        }
    }

    impl Error for FormBuildError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::ResourceLimit(error) => Some(error),
                Self::FormDataMustBeObject => None,
            }
        }
    }

    impl Error for UserOperationError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::ResourceLimit(error) => Some(error),
                _ => None,
            }
        }
    }

    impl Error for HostCommitError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::ResourceLimit(error) => Some(error),
                Self::InvalidOperation => None,
            }
        }
    }

    impl<E: Error + 'static> Error for TransactionError<E> {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::Closure(error) => Some(error),
                Self::Commit(error) => Some(error),
            }
        }
    }

    impl Error for ExternalFindingError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::ResourceLimit(error) => Some(error),
                Self::StaleRevision { .. } => None,
            }
        }
    }

    impl Error for ReinitializeError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::ResourceLimit(error) => Some(error),
                Self::InvalidFormData => None,
            }
        }
    }
}

pub use address::{ExtensionNamespace, JsonPointer, RetrievalUri, SchemaLocation, WidgetSymbol};
pub use definition::{
    AnalysisError, CapabilityFinding, CapabilityReport, CapabilitySeverity,
    CompilationLimitDimension, CompilationLimitError, CompilationLimitPhase, CompilationProfile,
    CompileError, DataSchemaAnnotations, DefinitionFingerprint, DefinitionNodeId, Dialect,
    FormAnalysis, FormCompiler, FormDefinition, ResourceError, ResourceLimitError,
    ResourceLimitPhase, SchemaResource,
};
pub use finding::{ExternalFinding, ExternalFindingBatch, FindingView, ValidationFinding};
pub use form::{
    DataRevision, FindingVisibility, FindingVisibilityPolicy, Form, FormBuildError, FormView,
    InstanceIdentity, ItemIdentity, NodeView, StateRevision, SubmissionOutcome,
    SubmissionPreparation, SubmissionSnapshot, Transition, UserActions,
};
pub use json::{FormDataLimits, JsonParseError, JsonSyntaxError};
pub use qualification::{QualificationError, QualificationLocation, QualificationResource};
