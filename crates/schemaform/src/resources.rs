use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
};

use jsonptr::PointerBuf;
use referencing::{Draft, Registry, RegistryBuilder, Retrieve, Uri};
use serde_json::Value;

use crate::{
    JsonPointer, QualificationError, QualificationLocation, QualificationResource, RetrievalUri,
    SchemaLocation,
};

const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";
const BUILT_IN_RESOURCE_URIS: [&str; 9] = [
    DRAFT_2020_12,
    "https://json-schema.org/draft/2020-12/meta/core",
    "https://json-schema.org/draft/2020-12/meta/applicator",
    "https://json-schema.org/draft/2020-12/meta/unevaluated",
    "https://json-schema.org/draft/2020-12/meta/validation",
    "https://json-schema.org/draft/2020-12/meta/meta-data",
    "https://json-schema.org/draft/2020-12/meta/format-annotation",
    "https://json-schema.org/draft/2020-12/meta/content",
    "https://json-schema.org/draft/2020-12/meta/format-assertion",
];
const SUPPORTED_VOCABULARIES: [&str; 7] = [
    "https://json-schema.org/draft/2020-12/vocab/core",
    "https://json-schema.org/draft/2020-12/vocab/applicator",
    "https://json-schema.org/draft/2020-12/vocab/unevaluated",
    "https://json-schema.org/draft/2020-12/vocab/validation",
    "https://json-schema.org/draft/2020-12/vocab/meta-data",
    "https://json-schema.org/draft/2020-12/vocab/format-annotation",
    "https://json-schema.org/draft/2020-12/vocab/content",
];

#[derive(Debug)]
pub(crate) enum PrepareError {
    Qualification(QualificationError),
    InvalidGraph,
}

pub(crate) struct ResourceGraph {
    documents: Vec<ResourceDocument>,
    registry: Option<Registry<'static>>,
    aliases: HashMap<String, ResourceRoot>,
    identity_locations: HashMap<String, QualificationLocation>,
    anchors: HashMap<(String, String), AnchorTarget>,
    indexed_schemas: HashSet<(usize, String, String)>,
    qualification_traversal: usize,
    reference_count: usize,
    root: ResourceRoot,
}

struct ResourceDocument {
    origin: QualificationResource,
    retrieval_uri: String,
    document: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResourceRoot {
    document_index: usize,
    resource: String,
    document_pointer: PointerBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AnchorTarget {
    root: ResourceRoot,
    document_pointer: PointerBuf,
    pointer: PointerBuf,
    location: QualificationLocation,
}

pub(crate) struct GraphLocation<'a> {
    pub(crate) schema: &'a Value,
    pub(crate) document_index: usize,
    pub(crate) resource: String,
    pub(crate) pointer: PointerBuf,
    pub(crate) document_pointer: PointerBuf,
    pub(crate) resource_root: PointerBuf,
}

impl ResourceGraph {
    pub(crate) fn prepare(
        root_uri: RetrievalUri,
        root_document: Value,
        resources: Vec<(RetrievalUri, Value)>,
    ) -> Result<Self, PrepareError> {
        Self::prepare_with_allowed_dialects(root_uri, root_document, resources, &[], None)
    }

    pub(crate) fn prepare_with_default_dialect(
        root_uri: RetrievalUri,
        root_document: Value,
        resources: Vec<(RetrievalUri, Value)>,
        default_dialect: Option<&str>,
    ) -> Result<Self, PrepareError> {
        Self::prepare_with_allowed_dialects(
            root_uri,
            root_document,
            resources,
            &[],
            default_dialect,
        )
    }

    #[cfg(test)]
    pub(crate) fn prepare_for_validator_suite(
        root_uri: RetrievalUri,
        root_document: Value,
        resources: Vec<(RetrievalUri, Value)>,
        allowed_dialects: &[&str],
    ) -> Result<Self, PrepareError> {
        // This prepares dependency conformance fixtures for Validator tests; forms continue to
        // call `prepare`, which enforces the product's standard-dialect-only input policy.
        Self::prepare_with_allowed_dialects(
            root_uri,
            root_document,
            resources,
            allowed_dialects,
            None,
        )
    }

    fn prepare_with_allowed_dialects(
        root_uri: RetrievalUri,
        root_document: Value,
        resources: Vec<(RetrievalUri, Value)>,
        allowed_dialects: &[&str],
        default_dialect: Option<&str>,
    ) -> Result<Self, PrepareError> {
        let mut documents = Vec::with_capacity(resources.len() + 1);
        documents.push(ResourceDocument {
            origin: QualificationResource::Root,
            retrieval_uri: normalize_uri(root_uri.as_str())?,
            document: root_document,
        });
        for (index, (uri, document)) in resources.into_iter().enumerate() {
            documents.push(ResourceDocument {
                origin: QualificationResource::Caller(index),
                retrieval_uri: normalize_uri(uri.as_str())?,
                document,
            });
        }

        let known_resource_identities = known_resource_identities(&documents);
        let mut retrieval_uris = HashMap::<String, QualificationLocation>::new();
        for document in &documents {
            let location = qualification_location(document, "");
            if let Some(first_location) =
                retrieval_uris.insert(document.retrieval_uri.clone(), location.clone())
            {
                return Err(PrepareError::Qualification(
                    QualificationError::DuplicateRetrievalIdentity {
                        identity: document.retrieval_uri.clone(),
                        first_location: Box::new(first_location),
                        second_location: Box::new(location),
                    },
                ));
            }
            let dialect = document.document.get("$schema").and_then(Value::as_str);
            let uses_default = dialect.is_none() && default_dialect == Some(DRAFT_2020_12);
            if uses_default {
                qualify_nested_dialects(document).map_err(PrepareError::Qualification)?;
            } else if !dialect.is_some_and(|dialect| allowed_dialects.contains(&dialect)) {
                qualify_dialect(document, &documents).map_err(PrepareError::Qualification)?;
            }
            qualify_missing_resource_references(document, &known_resource_identities)
                .map_err(PrepareError::Qualification)?;
        }

        let mut identity_locations = BUILT_IN_RESOURCE_URIS
            .into_iter()
            .map(|identity| {
                (
                    identity.to_owned(),
                    QualificationLocation::new(
                        QualificationResource::BuiltIn,
                        RetrievalUri::parse(identity)
                            .expect("built-in resource identities are valid retrieval URIs"),
                        JsonPointer::parse("").expect("the root pointer is valid"),
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        for document in &documents {
            let identity = document.retrieval_uri.clone();
            let location = qualification_location(document, "");
            if let Some(first_location) =
                identity_locations.insert(identity.clone(), location.clone())
            {
                return Err(PrepareError::Qualification(
                    QualificationError::DuplicateCanonicalIdentity {
                        identity,
                        first_location: Box::new(first_location),
                        second_location: Box::new(location),
                    },
                ));
            }
        }

        let placeholder = ResourceRoot {
            document_index: 0,
            resource: documents[0].retrieval_uri.clone(),
            document_pointer: PointerBuf::new(),
        };
        let mut graph = Self {
            documents,
            registry: None,
            aliases: HashMap::new(),
            identity_locations,
            anchors: HashMap::new(),
            indexed_schemas: HashSet::new(),
            qualification_traversal: 0,
            reference_count: 0,
            root: placeholder,
        };

        for index in 0..graph.documents.len() {
            let retrieval_uri = graph.documents[index].retrieval_uri.clone();
            let canonical = qualified_canonical_resource_uri(
                &graph.documents[index],
                &PointerBuf::new(),
                &retrieval_uri,
                &graph.documents[index].document,
            )?;
            let retrieval_location = qualification_location(&graph.documents[index], "");
            let canonical_location = qualification_location(
                &graph.documents[index],
                if graph.documents[index].document.get("$id").is_some() {
                    "/$id"
                } else {
                    ""
                },
            );
            let root = ResourceRoot {
                document_index: index,
                resource: canonical,
                document_pointer: PointerBuf::new(),
            };
            graph.insert_alias(retrieval_uri, root.clone(), retrieval_location)?;
            graph.insert_alias(root.resource.clone(), root.clone(), canonical_location)?;
            if index == 0 {
                graph.root = root.clone();
            }
            graph.index_schema(PointerBuf::new(), root)?;
        }
        for document in &graph.documents {
            if let Err(error) = jsonschema::draft202012::meta::validate(&document.document) {
                return Err(PrepareError::Qualification(invalid_schema(
                    document,
                    error.instance_path().as_str(),
                )));
            }
        }
        graph.qualify_references()?;

        // The registry discovers standard subschemas itself; only opaque extension paths need
        // explicit registration, otherwise legal nested $id resources are registered twice.
        let mut undiscovered_resources = graph
            .aliases
            .values()
            .filter(|root| {
                !root.document_pointer.is_root()
                    && root
                        .document_pointer
                        .resolve(&graph.documents[root.document_index].document)
                        .is_ok_and(|schema| {
                            !is_draft_subresource(
                                &graph.documents[root.document_index].document,
                                schema,
                            )
                        })
            })
            .cloned()
            .collect::<Vec<_>>();
        undiscovered_resources.sort_by(|left, right| {
            left.document_index
                .cmp(&right.document_index)
                .then_with(|| left.document_pointer.cmp(&right.document_pointer))
                .then_with(|| left.resource.cmp(&right.resource))
        });
        undiscovered_resources.dedup();

        let mut registry = empty_validator_registry();
        for document_index in 0..graph.documents.len() {
            let retrieval_uri = graph.documents[document_index].retrieval_uri.clone();
            registry = registry
                .add(
                    retrieval_uri,
                    graph.validator_document(document_index, &PointerBuf::new()),
                )
                .map_err(|_| PrepareError::InvalidGraph)?;
        }
        for root in undiscovered_resources {
            registry = registry
                .add(
                    root.resource,
                    graph.validator_document(root.document_index, &root.document_pointer),
                )
                .map_err(|_| PrepareError::InvalidGraph)?;
        }
        graph.registry = Some(registry.prepare().map_err(|_| PrepareError::InvalidGraph)?);

        Ok(graph)
    }

    pub(crate) fn root_location(&self) -> GraphLocation<'_> {
        self.location_from_root(&self.root)
    }

    #[cfg(schemaform_test_validation_faults)]
    pub(crate) fn root_document(&self) -> &Value {
        &self.documents[0].document
    }

    pub(crate) fn validator_root_document(&self) -> Value {
        self.validator_document(0, &PointerBuf::new())
    }

    pub(crate) fn root_resource(&self) -> &str {
        &self.root.resource
    }

    pub(crate) fn limit_metrics(&self) -> (usize, usize, usize, usize, usize) {
        let resources = self.resource_schemas().len();
        let uri_bytes = self
            .aliases
            .keys()
            .map(String::len)
            .chain(
                self.anchors
                    .keys()
                    .map(|(resource, anchor)| resource.len().saturating_add(anchor.len() + 1)),
            )
            .max()
            .unwrap_or(0);
        let pointer_bytes = self
            .indexed_schemas
            .iter()
            .map(|(_, pointer, _)| pointer.len())
            .chain(
                self.anchors
                    .values()
                    .map(|target| target.pointer.as_str().len()),
            )
            .max()
            .unwrap_or(0);
        (
            resources,
            self.reference_count,
            self.qualification_traversal,
            uri_bytes,
            pointer_bytes,
        )
    }

    pub(crate) fn registry(&self) -> &Registry<'static> {
        self.registry
            .as_ref()
            .expect("qualified resource graphs have a prepared registry")
    }

    pub(crate) fn qualification_location_for_error(
        &self,
        resource: Option<&str>,
        pointer: &str,
        instance: &Value,
    ) -> QualificationLocation {
        if let Some(root) = resource.and_then(|resource| self.aliases.get(resource))
            && let Some(document_pointer) = append_pointer_text(&root.document_pointer, pointer)
            && document_pointer
                .resolve(&self.documents[root.document_index].document)
                .is_ok_and(|value| value == instance)
        {
            return qualification_location(
                &self.documents[root.document_index],
                document_pointer.as_str(),
            );
        }

        let mut roots = self.aliases.values().cloned().collect::<Vec<_>>();
        roots.sort_by(|left, right| {
            left.document_index
                .cmp(&right.document_index)
                .then_with(|| left.document_pointer.cmp(&right.document_pointer))
                .then_with(|| left.resource.cmp(&right.resource))
        });
        roots.dedup();
        for root in roots {
            let Some(document_pointer) = append_pointer_text(&root.document_pointer, pointer)
            else {
                continue;
            };
            if document_pointer
                .resolve(&self.documents[root.document_index].document)
                .is_ok_and(|value| value == instance)
            {
                return qualification_location(
                    &self.documents[root.document_index],
                    document_pointer.as_str(),
                );
            }
        }

        qualification_location(&self.documents[0], pointer)
    }

    pub(crate) fn document_for_resource(&self, resource: &str) -> Option<&Value> {
        let root = self.aliases.get(resource)?;
        Some(&self.documents[root.document_index].document)
    }

    fn resource_schemas(&self) -> Vec<(String, &Value)> {
        let mut roots = self.aliases.values().cloned().collect::<Vec<_>>();
        roots.sort_by(|left, right| {
            left.document_index
                .cmp(&right.document_index)
                .then_with(|| left.document_pointer.cmp(&right.document_pointer))
                .then_with(|| left.resource.cmp(&right.resource))
        });
        roots.dedup();
        roots
            .into_iter()
            .map(|root| {
                let schema = root
                    .document_pointer
                    .resolve(&self.documents[root.document_index].document)
                    .expect("indexed resource roots remain addressable");
                (root.resource, schema)
            })
            .collect()
    }

    fn validator_document(&self, document_index: usize, root: &PointerBuf) -> Value {
        let document = &self.documents[document_index].document;
        root.resolve(document)
            .expect("indexed validator roots remain addressable")
            .clone()
    }

    pub(crate) fn cardinality_bound_limits(&self) -> HashMap<SchemaLocation, Value> {
        let mut limits = HashMap::new();
        for (document_index, document_pointer, _) in &self.indexed_schemas {
            let document_pointer = PointerBuf::parse(document_pointer.clone())
                .expect("indexed schema locations are JSON Pointers");
            let schema = document_pointer
                .resolve(&self.documents[*document_index].document)
                .expect("indexed schemas remain addressable");
            let fallback = self
                .aliases
                .get(&self.documents[*document_index].retrieval_uri)
                .expect("document retrieval identities retain their resource roots");
            let resource_root =
                self.deepest_resource_root(*document_index, &document_pointer, fallback);
            let resource = RetrievalUri::parse(resource_root.resource)
                .expect("qualified resource identities are absolute and fragment-free");
            let pointer = relative_pointer(&resource_root.document_pointer, &document_pointer)
                .expect("indexed schemas remain within their resource roots");
            let Some(object) = schema.as_object() else {
                continue;
            };
            for keyword in ["minItems", "maxItems", "minContains", "maxContains"] {
                if let Some(limit) = object.get(keyword) {
                    let bound_pointer = append_pointer(&pointer, [keyword]);
                    limits.insert(
                        SchemaLocation::new(
                            resource.clone(),
                            JsonPointer::parse(bound_pointer.to_string())
                                .expect("indexed schema locations are JSON Pointers"),
                        ),
                        limit.clone(),
                    );
                }
            }
        }
        limits
    }

    pub(crate) fn anchor_locations(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.anchors.iter().map(|((resource, anchor), target)| {
            (format!("{resource}#{anchor}"), target.pointer.to_string())
        })
    }

    pub(crate) fn schema_location_aliases(&self) -> HashMap<String, String> {
        let mut aliases = HashMap::<String, Option<String>>::new();
        for ((resource, _), target) in &self.anchors {
            let anchored = target
                .document_pointer
                .resolve(&self.documents[target.root.document_index].document)
                .expect("indexed anchors remain addressable");
            for relative in value_pointers(anchored) {
                let direct = append_pointer_text(&target.root.document_pointer, &relative)
                    .and_then(|pointer| {
                        pointer
                            .resolve(&self.documents[target.root.document_index].document)
                            .ok()
                    });
                if direct.is_some() {
                    continue;
                }
                let full = format!("{}{}", target.pointer.as_str(), relative);
                let key = format!("{resource}#{relative}");
                aliases
                    .entry(key)
                    .and_modify(|existing| {
                        if existing.as_deref() != Some(full.as_str()) {
                            *existing = None;
                        }
                    })
                    .or_insert(Some(full));
            }
        }
        aliases
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect()
    }

    pub(crate) fn normalize_location<'a>(
        &'a self,
        mut location: GraphLocation<'a>,
    ) -> Result<GraphLocation<'a>, PrepareError> {
        if self.aliases.get(&location.resource).is_some_and(|root| {
            root.document_index == location.document_index
                && root.document_pointer == location.document_pointer
        }) {
            return Ok(location);
        }
        let Some(id) = location.schema.get("$id").and_then(Value::as_str) else {
            return Ok(location);
        };
        let canonical = resolve_resource_uri(&location.resource, id)?;
        let root = self
            .aliases
            .get(&canonical)
            .ok_or(PrepareError::InvalidGraph)?;
        if root.document_index != location.document_index
            || root.document_pointer != location.document_pointer
        {
            return Err(PrepareError::InvalidGraph);
        }
        location.resource = root.resource.clone();
        location.pointer = PointerBuf::new();
        location.resource_root = root.document_pointer.clone();
        Ok(location)
    }

    pub(crate) fn resolve_reference<'a>(
        &'a self,
        base_resource: &str,
        reference: &str,
    ) -> Result<GraphLocation<'a>, PrepareError> {
        let location = self.resolve_reference_raw(base_resource, reference)?;
        self.normalize_location(location)
    }

    pub(crate) fn applicable_children<'a>(
        &'a self,
        location: &GraphLocation<'a>,
    ) -> Result<Vec<GraphLocation<'a>>, PrepareError> {
        applicable_schema_children(location.schema, &location.document_pointer)
            .into_iter()
            .map(|document_pointer| {
                let schema = document_pointer
                    .resolve(&self.documents[location.document_index].document)
                    .map_err(|_| PrepareError::InvalidGraph)?;
                let relative = relative_pointer(&location.document_pointer, &document_pointer)?;
                let pointer = append_pointer_text(&location.pointer, relative.as_str())
                    .ok_or(PrepareError::InvalidGraph)?;
                self.normalize_location(GraphLocation {
                    schema,
                    document_index: location.document_index,
                    resource: location.resource.clone(),
                    pointer,
                    document_pointer,
                    resource_root: location.resource_root.clone(),
                })
            })
            .collect()
    }

    fn resolve_reference_raw<'a>(
        &'a self,
        base_resource: &str,
        reference: &str,
    ) -> Result<GraphLocation<'a>, PrepareError> {
        let base = Uri::parse(base_resource.to_owned()).map_err(|_| PrepareError::InvalidGraph)?;
        let resolved = referencing::uri::resolve_against(&base.borrow(), reference)
            .map_err(|_| PrepareError::InvalidGraph)?;
        let resolved_text = resolved.as_str();
        let (resource_uri, encoded_fragment) = resolved_text
            .split_once('#')
            .map_or((resolved_text, ""), |parts| parts);
        let normalized_resource = normalize_uri(resource_uri)?;
        let root = self
            .aliases
            .get(&normalized_resource)
            .ok_or(PrepareError::InvalidGraph)?;
        if encoded_fragment.is_empty() {
            return Ok(self.location_from_root(root));
        }
        if !encoded_fragment.starts_with('/') {
            let anchor = decode_uri_fragment(encoded_fragment).ok_or(PrepareError::InvalidGraph)?;
            let anchored = self
                .anchors
                .get(&(root.resource.clone(), anchor))
                .ok_or(PrepareError::InvalidGraph)?;
            let schema = anchored
                .document_pointer
                .resolve(&self.documents[anchored.root.document_index].document)
                .expect("indexed anchors remain addressable");
            return Ok(GraphLocation {
                schema,
                document_index: anchored.root.document_index,
                resource: anchored.root.resource.clone(),
                pointer: anchored.pointer.clone(),
                document_pointer: anchored.document_pointer.clone(),
                resource_root: anchored.root.document_pointer.clone(),
            });
        }

        let fragment = decode_uri_fragment(encoded_fragment).ok_or(PrepareError::InvalidGraph)?;
        let requested_pointer =
            PointerBuf::parse(fragment).map_err(|_| PrepareError::InvalidGraph)?;
        let document_pointer =
            append_pointer_text(&root.document_pointer, requested_pointer.as_str())
                .ok_or(PrepareError::InvalidGraph)?;
        let schema = document_pointer
            .resolve(&self.documents[root.document_index].document)
            .map_err(|_| PrepareError::InvalidGraph)?;
        let root = self.deepest_resource_root(root.document_index, &document_pointer, root);
        let pointer = relative_pointer(&root.document_pointer, &document_pointer)?;
        Ok(GraphLocation {
            schema,
            document_index: root.document_index,
            resource: root.resource.clone(),
            pointer,
            document_pointer,
            resource_root: root.document_pointer.clone(),
        })
    }

    fn deepest_resource_root(
        &self,
        document_index: usize,
        target: &PointerBuf,
        fallback: &ResourceRoot,
    ) -> ResourceRoot {
        self.aliases
            .values()
            .filter(|candidate| {
                candidate.document_index == document_index
                    && pointer_contains(&candidate.document_pointer, target)
            })
            .max_by(|left, right| {
                left.document_pointer
                    .as_str()
                    .len()
                    .cmp(&right.document_pointer.as_str().len())
                    .then_with(|| left.resource.cmp(&right.resource))
            })
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    }

    fn index_schema(
        &mut self,
        document_pointer: PointerBuf,
        mut resource_root: ResourceRoot,
    ) -> Result<(), PrepareError> {
        let document_index = resource_root.document_index;
        if !self.indexed_schemas.insert((
            document_index,
            document_pointer.to_string(),
            resource_root.resource.clone(),
        )) {
            return Ok(());
        }
        let (id, anchors, children) = {
            let schema = document_pointer
                .resolve(&self.documents[document_index].document)
                .map_err(|_| PrepareError::InvalidGraph)?;
            (
                schema.get("$id").and_then(Value::as_str).map(str::to_owned),
                ["$anchor", "$dynamicAnchor"]
                    .into_iter()
                    .filter_map(|keyword| {
                        schema
                            .get(keyword)
                            .and_then(Value::as_str)
                            .map(|anchor| (keyword, anchor.to_owned()))
                    })
                    .collect::<Vec<_>>(),
                schema_children(schema, &document_pointer),
            )
        };

        if document_pointer != resource_root.document_pointer {
            if let Some(id) = id {
                let canonical =
                    resolve_resource_uri(&resource_root.resource, &id).map_err(|_| {
                        PrepareError::Qualification(QualificationError::InvalidCanonicalIdentity {
                            location: qualification_location(
                                &self.documents[document_index],
                                append_pointer(&document_pointer, ["$id"]).as_str(),
                            ),
                            identity: id.clone(),
                        })
                    })?;
                resource_root = ResourceRoot {
                    document_index,
                    resource: canonical,
                    document_pointer: document_pointer.clone(),
                };
                let location = qualification_location(
                    &self.documents[document_index],
                    append_pointer(&document_pointer, ["$id"]).as_str(),
                );
                self.insert_alias(
                    resource_root.resource.clone(),
                    resource_root.clone(),
                    location,
                )?;
            }
        }

        for (keyword, anchor) in anchors {
            let location = qualification_location(
                &self.documents[document_index],
                append_pointer(&document_pointer, [keyword]).as_str(),
            );
            let key = (resource_root.resource.clone(), anchor.clone());
            if let Some(first) = self.anchors.get(&key) {
                return Err(PrepareError::Qualification(
                    QualificationError::DuplicateAnchorIdentity {
                        resource_uri: resource_root.resource.clone(),
                        anchor,
                        first_location: Box::new(first.location.clone()),
                        second_location: Box::new(location),
                    },
                ));
            }
            self.anchors.insert(
                key,
                AnchorTarget {
                    pointer: relative_pointer(&resource_root.document_pointer, &document_pointer)?,
                    root: resource_root.clone(),
                    document_pointer: document_pointer.clone(),
                    location,
                },
            );
        }

        for child in children {
            self.index_schema(child, resource_root.clone())?;
        }
        Ok(())
    }

    fn qualify_references(&mut self) -> Result<(), PrepareError> {
        let mut visited = HashSet::new();
        for document_index in 0..self.documents.len() {
            let retrieval_uri = self.documents[document_index].retrieval_uri.clone();
            self.qualify_schema_references(
                document_index,
                &PointerBuf::new(),
                &retrieval_uri,
                true,
                &mut visited,
            )?;
        }
        Ok(())
    }

    fn index_reference_target(
        &mut self,
        document_index: usize,
        target: &PointerBuf,
        mut resource_root: ResourceRoot,
    ) -> Result<(), PrepareError> {
        let suffix = target
            .as_str()
            .strip_prefix(resource_root.document_pointer.as_str())
            .ok_or(PrepareError::InvalidGraph)?;
        if !resource_root.document_pointer.is_root()
            && !suffix.is_empty()
            && !suffix.starts_with('/')
        {
            return Err(PrepareError::InvalidGraph);
        }
        let mut pointer = resource_root.document_pointer.clone();
        for token in suffix.split('/').skip(1) {
            pointer = PointerBuf::parse(format!("{}/{token}", pointer.as_str()))
                .map_err(|_| PrepareError::InvalidGraph)?;
            let identity = pointer
                .resolve(&self.documents[document_index].document)
                .map_err(|_| PrepareError::InvalidGraph)?
                .get("$id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if identity.is_some() || pointer == *target {
                self.index_schema(pointer.clone(), resource_root.clone())?;
            }
            if let Some(identity) = identity {
                let canonical = resolve_resource_uri(&resource_root.resource, &identity)?;
                resource_root = self
                    .aliases
                    .get(&canonical)
                    .expect("indexed reference-reached identities retain their resource roots")
                    .clone();
            }
        }
        Ok(())
    }

    fn qualify_schema_references(
        &mut self,
        document_index: usize,
        pointer: &PointerBuf,
        inherited_resource: &str,
        apply_identity: bool,
        visited: &mut HashSet<(usize, String, String)>,
    ) -> Result<(), PrepareError> {
        let (identity, dialect, references, children, invalid_pointer) = {
            let schema = pointer
                .resolve(&self.documents[document_index].document)
                .map_err(|_| PrepareError::InvalidGraph)?;
            let invalid_pointer = jsonschema::draft202012::meta::validate(schema)
                .err()
                .and_then(|error| append_pointer_text(pointer, error.instance_path().as_str()));
            (
                schema.get("$id").and_then(Value::as_str).map(str::to_owned),
                schema
                    .get("$schema")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                ["$ref", "$dynamicRef"]
                    .into_iter()
                    .filter_map(|keyword| {
                        schema
                            .get(keyword)
                            .and_then(Value::as_str)
                            .map(|reference| (keyword, reference.to_owned()))
                    })
                    .collect::<Vec<_>>(),
                schema_children(schema, pointer),
                invalid_pointer,
            )
        };
        if let Some(dialect) = dialect
            && !pointer.is_root()
            && dialect != DRAFT_2020_12
        {
            return Err(PrepareError::Qualification(
                QualificationError::NestedDialectSwitch {
                    location: qualification_location(
                        &self.documents[document_index],
                        append_pointer(pointer, ["$schema"]).as_str(),
                    ),
                    dialect,
                },
            ));
        }
        if let Some(invalid_pointer) = invalid_pointer {
            return Err(PrepareError::Qualification(invalid_schema(
                &self.documents[document_index],
                invalid_pointer.as_str(),
            )));
        }
        let resource = if apply_identity {
            identity.map_or_else(
                || Ok(inherited_resource.to_owned()),
                |identity| {
                    resolve_resource_uri(inherited_resource, &identity).map_err(|_| {
                        PrepareError::Qualification(QualificationError::InvalidCanonicalIdentity {
                            location: qualification_location(
                                &self.documents[document_index],
                                append_pointer(pointer, ["$id"]).as_str(),
                            ),
                            identity,
                        })
                    })
                },
            )?
        } else {
            inherited_resource.to_owned()
        };
        if !visited.insert((document_index, pointer.to_string(), resource.clone())) {
            return Ok(());
        }
        self.qualification_traversal = self.qualification_traversal.saturating_add(1);
        self.reference_count = self.reference_count.saturating_add(references.len());
        for (keyword, reference) in references {
            match self.resolve_reference_raw(&resource, &reference) {
                Ok(target) => {
                    let target_document_index = target.document_index;
                    let target_pointer = target.document_pointer.clone();
                    let unresolved_target_resource = target.resource.clone();
                    let target_root = ResourceRoot {
                        document_index: target.document_index,
                        resource: target.resource.clone(),
                        document_pointer: target.resource_root.clone(),
                    };
                    self.index_reference_target(
                        target_document_index,
                        &target_pointer,
                        target_root,
                    )?;
                    let target = self.resolve_reference(&resource, &reference)?;
                    let target_resource = target.resource.clone();
                    let target_pointer = target.document_pointer.clone();
                    let target_resource_pointer = target.pointer.clone();
                    if keyword == "$ref" && target_resource != unresolved_target_resource {
                        let rewritten = if target_resource_pointer.is_root() {
                            target_resource.clone()
                        } else {
                            format!("{target_resource}#{target_resource_pointer}")
                        };
                        let reference_pointer = append_pointer(pointer, [keyword]);
                        *reference_pointer
                            .resolve_mut(&mut self.documents[document_index].document)
                            .map_err(|_| PrepareError::InvalidGraph)? = Value::String(rewritten);
                    }
                    self.qualify_schema_references(
                        target_document_index,
                        &target_pointer,
                        &target_resource,
                        false,
                        visited,
                    )?;
                }
                Err(_) if built_in_reference_resolves(&resource, &reference) => {}
                Err(_) => {
                    return Err(PrepareError::Qualification(
                        QualificationError::UnresolvedReference {
                            location: qualification_location(
                                &self.documents[document_index],
                                append_pointer(pointer, [keyword]).as_str(),
                            ),
                            reference,
                        },
                    ));
                }
            }
        }

        for child_pointer in children {
            self.qualify_schema_references(
                document_index,
                &child_pointer,
                &resource,
                true,
                visited,
            )?;
        }
        Ok(())
    }

    fn insert_alias(
        &mut self,
        uri: String,
        root: ResourceRoot,
        location: QualificationLocation,
    ) -> Result<(), PrepareError> {
        if let Some(existing) = self.aliases.get(&uri) {
            if existing != &root {
                return Err(PrepareError::Qualification(
                    QualificationError::DuplicateCanonicalIdentity {
                        identity: uri.clone(),
                        first_location: Box::new(
                            self.identity_locations
                                .get(&uri)
                                .expect("indexed aliases retain their source location")
                                .clone(),
                        ),
                        second_location: Box::new(location),
                    },
                ));
            }
            return Ok(());
        }
        if let Some(first_location) = self.identity_locations.get(&uri)
            && first_location != &location
        {
            return Err(PrepareError::Qualification(
                QualificationError::DuplicateCanonicalIdentity {
                    identity: uri,
                    first_location: Box::new(first_location.clone()),
                    second_location: Box::new(location),
                },
            ));
        }
        self.identity_locations.insert(uri.clone(), location);
        self.aliases.insert(uri, root);
        Ok(())
    }

    fn location_from_root<'a>(&'a self, root: &ResourceRoot) -> GraphLocation<'a> {
        let schema = root
            .document_pointer
            .resolve(&self.documents[root.document_index].document)
            .expect("indexed resource roots remain addressable");
        GraphLocation {
            schema,
            document_index: root.document_index,
            resource: root.resource.clone(),
            pointer: PointerBuf::new(),
            document_pointer: root.document_pointer.clone(),
            resource_root: root.document_pointer.clone(),
        }
    }
}

fn is_draft_subresource(document: &Value, target: &Value) -> bool {
    let mut pending = Draft::Draft202012
        .subresources_of(document)
        .collect::<Vec<_>>();
    while let Some(schema) = pending.pop() {
        if std::ptr::eq(schema, target) {
            return true;
        }
        pending.extend(Draft::Draft202012.subresources_of(schema));
    }
    false
}

fn schema_children(schema: &Value, parent: &PointerBuf) -> Vec<PointerBuf> {
    let Some(object) = schema.as_object() else {
        return Vec::new();
    };
    let mut children = Vec::new();
    for keyword in [
        "$defs",
        "definitions",
        "dependentSchemas",
        "patternProperties",
        "properties",
    ] {
        if let Some(values) = object.get(keyword).and_then(Value::as_object) {
            let mut names = values.keys().collect::<Vec<_>>();
            names.sort_unstable();
            for name in names {
                children.push(append_pointer(parent, [keyword, name.as_str()]));
            }
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(values) = object.get(keyword).and_then(Value::as_array) {
            for index in 0..values.len() {
                children.push(append_pointer(
                    parent,
                    [keyword, index.to_string().as_str()],
                ));
            }
        }
    }
    for keyword in [
        "additionalProperties",
        "contains",
        "contentSchema",
        "else",
        "if",
        "items",
        "not",
        "propertyNames",
        "then",
        "unevaluatedItems",
        "unevaluatedProperties",
    ] {
        if object.get(keyword).is_some() {
            children.push(append_pointer(parent, [keyword]));
        }
    }
    children
}

fn empty_validator_registry() -> RegistryBuilder<'static> {
    Registry::new()
        .draft(Draft::Draft202012)
        .retriever(DenyRetrieval)
}

fn applicable_schema_children(schema: &Value, parent: &PointerBuf) -> Vec<PointerBuf> {
    let Some(object) = schema.as_object() else {
        return Vec::new();
    };
    let mut children = Vec::new();
    for keyword in ["dependentSchemas", "patternProperties", "properties"] {
        if let Some(values) = object.get(keyword).and_then(Value::as_object) {
            let mut names = values.keys().collect::<Vec<_>>();
            names.sort_unstable();
            for name in names {
                children.push(append_pointer(parent, [keyword, name.as_str()]));
            }
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(values) = object.get(keyword).and_then(Value::as_array) {
            for index in 0..values.len() {
                children.push(append_pointer(
                    parent,
                    [keyword, index.to_string().as_str()],
                ));
            }
        }
    }
    for keyword in [
        "additionalProperties",
        "contains",
        "else",
        "if",
        "items",
        "not",
        "propertyNames",
        "then",
        "unevaluatedItems",
        "unevaluatedProperties",
    ] {
        if object.get(keyword).is_some() {
            children.push(append_pointer(parent, [keyword]));
        }
    }
    children
}

fn known_resource_identities(documents: &[ResourceDocument]) -> HashSet<String> {
    fn collect(
        document: &ResourceDocument,
        schema: &Value,
        pointer: &PointerBuf,
        inherited_resource: &str,
        identities: &mut HashSet<String>,
    ) {
        let resource = schema
            .get("$id")
            .and_then(Value::as_str)
            .and_then(|identity| resolve_resource_uri(inherited_resource, identity).ok())
            .unwrap_or_else(|| inherited_resource.to_owned());
        identities.insert(resource.clone());
        for child_pointer in schema_children(schema, pointer) {
            if let Ok(child) = child_pointer.resolve(&document.document) {
                collect(document, child, &child_pointer, &resource, identities);
            }
        }
    }

    let mut identities = HashSet::new();
    for document in documents {
        identities.insert(document.retrieval_uri.clone());
        collect(
            document,
            &document.document,
            &PointerBuf::new(),
            &document.retrieval_uri,
            &mut identities,
        );
    }
    identities
}

fn qualify_missing_resource_references(
    document: &ResourceDocument,
    known_resources: &HashSet<String>,
) -> Result<(), QualificationError> {
    fn visit(
        document: &ResourceDocument,
        schema: &Value,
        pointer: &PointerBuf,
        inherited_resource: &str,
        known_resources: &HashSet<String>,
    ) -> Result<(), QualificationError> {
        let resource = schema
            .get("$id")
            .and_then(Value::as_str)
            .map_or_else(
                || Ok(inherited_resource.to_owned()),
                |identity| resolve_resource_uri(inherited_resource, identity),
            )
            .unwrap_or_else(|_| inherited_resource.to_owned());
        for keyword in ["$ref", "$dynamicRef"] {
            let Some(reference) = schema.get(keyword).and_then(Value::as_str) else {
                continue;
            };
            let resolved_resource = Uri::parse(resource.clone())
                .ok()
                .and_then(|base| referencing::uri::resolve_against(&base.borrow(), reference).ok())
                .map(|mut resolved| {
                    resolved.set_fragment(None);
                    resolved.normalize().to_string()
                });
            if resolved_resource.as_ref().is_none_or(|target| {
                !known_resources.contains(target)
                    && !BUILT_IN_RESOURCE_URIS.contains(&target.as_str())
            }) {
                return Err(QualificationError::UnresolvedReference {
                    location: qualification_location(
                        document,
                        append_pointer(pointer, [keyword]).as_str(),
                    ),
                    reference: reference.to_owned(),
                });
            }
        }
        for child_pointer in schema_children(schema, pointer) {
            if let Ok(child) = child_pointer.resolve(&document.document) {
                visit(document, child, &child_pointer, &resource, known_resources)?;
            }
        }
        Ok(())
    }

    visit(
        document,
        &document.document,
        &PointerBuf::new(),
        &document.retrieval_uri,
        known_resources,
    )
}

fn qualified_canonical_resource_uri(
    document: &ResourceDocument,
    pointer: &PointerBuf,
    base_uri: &str,
    schema: &Value,
) -> Result<String, PrepareError> {
    let Some(identity) = schema.get("$id").and_then(Value::as_str) else {
        return normalize_uri(base_uri);
    };
    resolve_resource_uri(base_uri, identity).map_err(|_| {
        PrepareError::Qualification(QualificationError::InvalidCanonicalIdentity {
            location: qualification_location(document, append_pointer(pointer, ["$id"]).as_str()),
            identity: identity.to_owned(),
        })
    })
}

fn resolve_resource_uri(base_uri: &str, id: &str) -> Result<String, PrepareError> {
    let base = Uri::parse(base_uri.to_owned()).map_err(|_| PrepareError::InvalidGraph)?;
    let resolved = referencing::uri::resolve_against(&base.borrow(), id)
        .map_err(|_| PrepareError::InvalidGraph)?;
    if resolved
        .fragment()
        .is_some_and(|fragment| !fragment.as_str().is_empty())
    {
        return Err(PrepareError::InvalidGraph);
    }
    normalize_uri(resolved.as_str().trim_end_matches('#'))
}

fn normalize_uri(uri: &str) -> Result<String, PrepareError> {
    let parsed = Uri::parse(uri.to_owned()).map_err(|_| PrepareError::InvalidGraph)?;
    if parsed.fragment().is_some() {
        return Err(PrepareError::InvalidGraph);
    }
    Ok(parsed.normalize().to_string())
}

fn append_pointer<'a>(
    parent: &PointerBuf,
    tokens: impl IntoIterator<Item = &'a str>,
) -> PointerBuf {
    let mut pointer = parent.to_string();
    for token in tokens {
        pointer.push('/');
        pointer.push_str(&token.replace('~', "~0").replace('/', "~1"));
    }
    PointerBuf::parse(pointer).expect("escaped tokens form a valid JSON Pointer")
}

fn append_pointer_text(parent: &PointerBuf, suffix: &str) -> Option<PointerBuf> {
    PointerBuf::parse(format!("{}{suffix}", parent.as_str())).ok()
}

fn relative_pointer(root: &PointerBuf, target: &PointerBuf) -> Result<PointerBuf, PrepareError> {
    let suffix = target
        .as_str()
        .strip_prefix(root.as_str())
        .ok_or(PrepareError::InvalidGraph)?;
    if !root.as_str().is_empty() && !suffix.is_empty() && !suffix.starts_with('/') {
        return Err(PrepareError::InvalidGraph);
    }
    PointerBuf::parse(suffix).map_err(|_| PrepareError::InvalidGraph)
}

fn pointer_contains(root: &PointerBuf, target: &PointerBuf) -> bool {
    target.as_str() == root.as_str()
        || target
            .as_str()
            .strip_prefix(root.as_str())
            .is_some_and(|suffix| root.is_root() || suffix.starts_with('/'))
}

fn value_pointers(value: &Value) -> Vec<String> {
    fn visit(value: &Value, pointer: String, pointers: &mut Vec<String>) {
        pointers.push(pointer.clone());
        match value {
            Value::Object(values) => {
                for (key, value) in values {
                    let token = key.replace('~', "~0").replace('/', "~1");
                    visit(value, format!("{pointer}/{token}"), pointers);
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    visit(value, format!("{pointer}/{index}"), pointers);
                }
            }
            _ => {}
        }
    }

    let mut pointers = Vec::new();
    visit(value, String::new(), &mut pointers);
    pointers
}

fn invalid_schema(document: &ResourceDocument, pointer: &str) -> QualificationError {
    QualificationError::InvalidSchema {
        location: QualificationLocation::new(
            document.origin,
            RetrievalUri::parse(document.retrieval_uri.clone())
                .expect("normalized retrieval URIs remain valid"),
            JsonPointer::parse(pointer).expect("validator instance paths are JSON Pointers"),
        ),
    }
}

fn qualification_location(document: &ResourceDocument, pointer: &str) -> QualificationLocation {
    QualificationLocation::new(
        document.origin,
        RetrievalUri::parse(document.retrieval_uri.clone())
            .expect("normalized retrieval URIs remain valid"),
        JsonPointer::parse(pointer).expect("qualified schema locations are JSON Pointers"),
    )
}

fn qualify_dialect(
    document: &ResourceDocument,
    documents: &[ResourceDocument],
) -> Result<(), QualificationError> {
    let Some(dialect) = document.document.get("$schema") else {
        return Err(QualificationError::MissingDialect {
            location: qualification_location(document, ""),
        });
    };
    let Some(dialect) = dialect.as_str() else {
        return Ok(());
    };
    if dialect != DRAFT_2020_12 {
        if let Some((document, meta_schema, pointer)) = find_dialect_target(dialect, documents) {
            qualify_required_vocabularies(document, meta_schema, &pointer)?;
        }
        return Err(QualificationError::UnsupportedDialect {
            location: qualification_location(document, "/$schema"),
            dialect: dialect.to_owned(),
        });
    }

    qualify_nested_dialects(document)
}

fn qualify_nested_dialects(document: &ResourceDocument) -> Result<(), QualificationError> {
    if let Some((pointer, dialect)) =
        first_nested_dialect(&document.document, &document.document, &PointerBuf::new())
    {
        return Err(QualificationError::NestedDialectSwitch {
            location: qualification_location(document, &pointer),
            dialect,
        });
    }
    Ok(())
}

fn qualify_required_vocabularies(
    document: &ResourceDocument,
    meta_schema: &Value,
    pointer: &PointerBuf,
) -> Result<(), QualificationError> {
    let Some(vocabularies) = meta_schema.get("$vocabulary").and_then(Value::as_object) else {
        return Ok(());
    };
    for (vocabulary, required) in vocabularies {
        if required.as_bool() == Some(true)
            && !SUPPORTED_VOCABULARIES.contains(&vocabulary.as_str())
        {
            return Err(QualificationError::UnsupportedRequiredVocabulary {
                location: qualification_location(
                    document,
                    append_pointer(pointer, ["$vocabulary", vocabulary.as_str()]).as_str(),
                ),
                vocabulary: vocabulary.clone(),
            });
        }
    }
    Ok(())
}

fn find_dialect_target<'a>(
    dialect: &str,
    documents: &'a [ResourceDocument],
) -> Option<(&'a ResourceDocument, &'a Value, PointerBuf)> {
    let parsed = Uri::parse(dialect.to_owned()).ok()?;
    let raw_fragment = parsed
        .fragment()
        .map(|fragment| fragment.as_str().to_owned());
    let mut resource_uri = parsed;
    resource_uri.set_fragment(None);
    let resource_uri = normalize_uri(resource_uri.as_str()).ok()?;

    for document in documents {
        let mut pointers = value_pointers(&document.document)
            .into_iter()
            .filter_map(|pointer| PointerBuf::parse(pointer).ok())
            .collect::<Vec<_>>();
        pointers.sort();
        for resource_pointer in pointers.iter().filter(|pointer| {
            pointer.is_root()
                || pointer
                    .resolve(&document.document)
                    .is_ok_and(|value| value.get("$id").and_then(Value::as_str).is_some())
        }) {
            if resource_at_pointer(document, resource_pointer).as_deref()
                != Some(resource_uri.as_str())
            {
                continue;
            }
            match raw_fragment.as_deref() {
                None | Some("") => {
                    let target = resource_pointer.resolve(&document.document).ok()?;
                    return Some((document, target, resource_pointer.clone()));
                }
                Some(fragment) if fragment.starts_with('/') => {
                    let relative = decode_uri_fragment(fragment)?;
                    let target_pointer = append_pointer_text(resource_pointer, &relative)?;
                    let target = target_pointer.resolve(&document.document).ok()?;
                    return Some((document, target, target_pointer));
                }
                Some(fragment) => {
                    let anchor = decode_uri_fragment(fragment)?;
                    if let Some(target_pointer) = pointers.iter().find(|pointer| {
                        pointer_contains(resource_pointer, pointer)
                            && resource_at_pointer(document, pointer).as_deref()
                                == Some(resource_uri.as_str())
                            && pointer.resolve(&document.document).is_ok_and(|value| {
                                ["$anchor", "$dynamicAnchor"].into_iter().any(|keyword| {
                                    value.get(keyword).and_then(Value::as_str) == Some(&anchor)
                                })
                            })
                    }) {
                        let target = target_pointer.resolve(&document.document).ok()?;
                        return Some((document, target, target_pointer.clone()));
                    }
                }
            }
        }
    }
    None
}

fn resource_at_pointer(document: &ResourceDocument, target: &PointerBuf) -> Option<String> {
    let mut resource = document.retrieval_uri.clone();
    let mut pointers = value_pointers(&document.document)
        .into_iter()
        .filter_map(|pointer| PointerBuf::parse(pointer).ok())
        .filter(|pointer| pointer_contains(pointer, target))
        .collect::<Vec<_>>();
    pointers.sort_by_key(|pointer| pointer.as_str().len());
    for pointer in pointers {
        if let Some(identity) = pointer
            .resolve(&document.document)
            .ok()
            .and_then(|value| value.get("$id"))
            .and_then(Value::as_str)
        {
            resource = resolve_resource_uri(&resource, identity).ok()?;
        }
    }
    Some(resource)
}

fn first_nested_dialect(
    document: &Value,
    schema: &Value,
    pointer: &PointerBuf,
) -> Option<(String, String)> {
    for child_pointer in schema_children(schema, pointer) {
        let child = child_pointer.resolve(document).ok()?;
        if let Some(dialect) = child.get("$schema").and_then(Value::as_str)
            && dialect != DRAFT_2020_12
        {
            return Some((
                append_pointer(&child_pointer, ["$schema"]).to_string(),
                dialect.to_owned(),
            ));
        }
        if let Some(found) = first_nested_dialect(document, child, &child_pointer) {
            return Some(found);
        }
    }
    None
}

fn built_in_reference_resolves(base_resource: &str, reference: &str) -> bool {
    let Ok(base) = Uri::parse(base_resource.to_owned()) else {
        return false;
    };
    let Ok(mut resolved) = referencing::uri::resolve_against(&base.borrow(), reference) else {
        return false;
    };
    resolved.set_fragment(None);
    if !BUILT_IN_RESOURCE_URIS.contains(&resolved.as_str()) {
        return false;
    }
    referencing::SPECIFICATIONS
        .resolver(base)
        .lookup(reference)
        .is_ok()
}

pub(crate) fn decode_uri_fragment(fragment: &str) -> Option<String> {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DenyRetrieval;

impl Retrieve for DenyRetrieval {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        Err(RetrievalDenied(uri.as_str().to_owned()).into())
    }
}

#[derive(Debug)]
pub(crate) struct RetrievalDenied(pub(crate) String);

impl fmt::Display for RetrievalDenied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "resource is not caller-supplied: {}", self.0)
    }
}

impl Error for RetrievalDenied {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn registry_preparation_denies_resource_retrieval() {
        let result = empty_validator_registry()
            .add(
                "https://schemas.example/root.json",
                json!({
                    "$schema": DRAFT_2020_12,
                    "$ref": "https://schemas.example/missing.json"
                }),
            )
            .expect("the fixture resource should enter the registry")
            .prepare();
        let Err(error) = result else {
            panic!("registry preparation should deny missing resource retrieval");
        };

        let referencing::Error::Unretrievable { uri, source } = error else {
            panic!("registry preparation returned a non-retrieval error: {error}");
        };
        assert_eq!(uri, "https://schemas.example/missing.json");
        assert_eq!(
            source
                .downcast_ref::<RetrievalDenied>()
                .map(|denied| denied.0.as_str()),
            Some("https://schemas.example/missing.json")
        );
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn indexed_referenced_cardinality_bounds_retain_authored_resource_locations() {
        let bound = serde_json::from_str::<Value>("1e4096")
            .expect("the arbitrary-precision bound should parse");
        let graph = ResourceGraph::prepare(
            RetrievalUri::parse("https://schemas.example/root.json").unwrap(),
            json!({
                "$schema": DRAFT_2020_12,
                "$ref": "https://schemas.example/arrays.json#/$defs/bounded"
            }),
            vec![(
                RetrievalUri::parse("https://schemas.example/arrays.json").unwrap(),
                json!({
                    "$schema": DRAFT_2020_12,
                    "$defs": {
                        "bounded": {
                            "type": "array",
                            "minItems": bound,
                            "maxItems": 3e0
                        }
                    }
                }),
            )],
        )
        .expect("the referenced resource graph should prepare");
        let limits = graph.cardinality_bound_limits();

        let minimum = SchemaLocation::new(
            RetrievalUri::parse("https://schemas.example/arrays.json").unwrap(),
            JsonPointer::parse("/$defs/bounded/minItems").unwrap(),
        );
        let maximum = SchemaLocation::new(
            RetrievalUri::parse("https://schemas.example/arrays.json").unwrap(),
            JsonPointer::parse("/$defs/bounded/maxItems").unwrap(),
        );
        assert_eq!(limits.get(&minimum), Some(&bound));
        assert_eq!(limits.get(&maximum), Some(&json!(3.0)));
    }
}
