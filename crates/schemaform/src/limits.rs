use std::{error::Error, fmt};

use referencing::Uri;
use serde::{Serialize, ser};
use serde_json::Value;

use crate::{CompilationProfile, JsonPointer, RetrievalUri, SchemaResource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompilationLimitPhase {
    Parse,
    Structure,
    Graph,
    Projection,
    Definition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompilationLimitDimension {
    Bytes,
    Tokens,
    Depth,
    Nodes,
    Members,
    ScalarBytes,
    Resources,
    References,
    Traversal,
    DefinitionNodes,
    Controls,
    UriBytes,
    PointerBytes,
    CapabilityFindings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilationLimitError {
    phase: CompilationLimitPhase,
    dimension: CompilationLimitDimension,
    maximum: usize,
    observed: usize,
    pointer: JsonPointer,
}

impl CompilationLimitError {
    pub(crate) fn new(
        phase: CompilationLimitPhase,
        dimension: CompilationLimitDimension,
        maximum: usize,
        observed: usize,
        pointer: String,
    ) -> Self {
        Self {
            phase,
            dimension,
            maximum,
            observed,
            pointer: JsonPointer::parse(pointer)
                .expect("limit scans construct escaped JSON Pointers"),
        }
    }

    pub fn phase(&self) -> CompilationLimitPhase {
        self.phase
    }

    pub fn dimension(&self) -> CompilationLimitDimension {
        self.dimension
    }

    pub fn maximum(&self) -> usize {
        self.maximum
    }

    pub fn observed(&self) -> usize {
        self.observed
    }

    pub fn pointer(&self) -> &JsonPointer {
        &self.pointer
    }
}

impl fmt::Display for CompilationLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "data-schema {:?} {:?} limit {} exceeded by {} at {}",
            self.phase, self.dimension, self.maximum, self.observed, self.pointer
        )
    }
}

impl Error for CompilationLimitError {}

#[derive(Clone, Copy)]
pub(crate) struct DataSchemaLimits {
    pub(crate) bytes: usize,
    pub(crate) tokens: usize,
    pub(crate) depth: usize,
    pub(crate) nodes: usize,
    pub(crate) members: usize,
    pub(crate) scalar_bytes: usize,
    pub(crate) resources: usize,
    pub(crate) references: usize,
    pub(crate) traversal: usize,
    pub(crate) definition_nodes: usize,
    pub(crate) controls: usize,
    pub(crate) uri_bytes: usize,
    pub(crate) pointer_bytes: usize,
    pub(crate) capability_findings: usize,
}

impl DataSchemaLimits {
    pub(crate) fn values(self) -> [usize; 14] {
        [
            self.bytes,
            self.tokens,
            self.depth,
            self.nodes,
            self.members,
            self.scalar_bytes,
            self.resources,
            self.references,
            self.traversal,
            self.definition_nodes,
            self.controls,
            self.uri_bytes,
            self.pointer_bytes,
            self.capability_findings,
        ]
    }
}

#[derive(Default)]
struct ValueMetrics {
    tokens: usize,
    depth: usize,
    depth_pointer: String,
    nodes: usize,
    members: usize,
    members_pointer: String,
    scalar_bytes: usize,
    scalar_pointer: String,
    pointer_bytes: usize,
    pointer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputLimitError {
    pub(crate) dimension: &'static str,
    pub(crate) maximum: usize,
    pub(crate) observed: usize,
    pub(crate) pointer: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct InputLimits {
    pub(crate) bytes: usize,
    pub(crate) tokens: usize,
    pub(crate) depth: usize,
    pub(crate) nodes: usize,
    pub(crate) members: usize,
    pub(crate) collection_length: usize,
    pub(crate) scalar_bytes: usize,
}

impl InputLimits {
    pub(crate) fn values(self) -> [usize; 7] {
        [
            self.bytes,
            self.tokens,
            self.depth,
            self.nodes,
            self.members,
            self.collection_length,
            self.scalar_bytes,
        ]
    }
}

impl fmt::Display for InputLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "input {} limit {} exceeded by {}",
            self.dimension, self.maximum, self.observed
        )
    }
}

impl Error for InputLimitError {}

impl ser::Error for InputLimitError {
    fn custom<T: fmt::Display>(_message: T) -> Self {
        Self {
            dimension: "serialization",
            maximum: 0,
            observed: 1,
            pointer: String::new(),
        }
    }
}

#[derive(Default)]
struct SerializedMetrics {
    nodes: usize,
    members: usize,
}

pub(crate) fn check_serializable_input<T: Serialize>(
    value: &T,
    limits: InputLimits,
) -> Result<(), InputLimitError> {
    value.serialize(MetricSerializer {
        metrics: &mut SerializedMetrics::default(),
        limits,
        depth: 0,
    })
}

struct MetricSerializer<'a> {
    metrics: &'a mut SerializedMetrics,
    limits: InputLimits,
    depth: usize,
}

impl MetricSerializer<'_> {
    fn node(&mut self) -> Result<(), InputLimitError> {
        if self.depth > self.limits.depth {
            return Err(self.error("depth", self.limits.depth, self.depth));
        }
        self.metrics.nodes = self.metrics.nodes.saturating_add(1);
        if self.metrics.nodes > self.limits.nodes {
            return Err(self.error("nodes", self.limits.nodes, self.metrics.nodes));
        }
        Ok(())
    }

    fn collection(&mut self, length: usize, members: bool) -> Result<(), InputLimitError> {
        self.node()?;
        if length > self.limits.collection_length {
            return Err(self.error("collection_length", self.limits.collection_length, length));
        }
        if members {
            self.metrics.members = self.metrics.members.saturating_add(length);
            if self.metrics.members > self.limits.members {
                return Err(self.error("members", self.limits.members, self.metrics.members));
            }
        }
        Ok(())
    }

    fn scalar(&mut self, bytes: usize) -> Result<(), InputLimitError> {
        self.node()?;
        self.scalar_without_node(bytes)
    }

    fn scalar_without_node(&self, bytes: usize) -> Result<(), InputLimitError> {
        if bytes > self.limits.scalar_bytes {
            return Err(self.error("scalar_bytes", self.limits.scalar_bytes, bytes));
        }
        Ok(())
    }

    fn error(&self, dimension: &'static str, maximum: usize, observed: usize) -> InputLimitError {
        InputLimitError {
            dimension,
            maximum,
            observed,
            pointer: String::new(),
        }
    }

    fn child(&mut self) -> MetricSerializer<'_> {
        MetricSerializer {
            metrics: self.metrics,
            limits: self.limits,
            depth: self.depth.saturating_add(1),
        }
    }
}

struct MetricCompound<'a> {
    serializer: MetricSerializer<'a>,
    unknown_length: bool,
    length: usize,
}

impl<'a> ser::Serializer for MetricSerializer<'a> {
    type Ok = ();
    type Error = InputLimitError;
    type SerializeSeq = MetricCompound<'a>;
    type SerializeTuple = MetricCompound<'a>;
    type SerializeTupleStruct = MetricCompound<'a>;
    type SerializeTupleVariant = MetricCompound<'a>;
    type SerializeMap = MetricCompound<'a>;
    type SerializeStruct = MetricCompound<'a>;
    type SerializeStructVariant = MetricCompound<'a>;

    fn serialize_bool(mut self, value: bool) -> Result<(), Self::Error> {
        self.scalar(if value { 4 } else { 5 })
    }

    fn serialize_i8(self, value: i8) -> Result<(), Self::Error> {
        self.serialize_i64(value.into())
    }
    fn serialize_i16(self, value: i16) -> Result<(), Self::Error> {
        self.serialize_i64(value.into())
    }
    fn serialize_i32(self, value: i32) -> Result<(), Self::Error> {
        self.serialize_i64(value.into())
    }
    fn serialize_i64(mut self, value: i64) -> Result<(), Self::Error> {
        self.scalar(value.to_string().len())
    }
    fn serialize_u8(self, value: u8) -> Result<(), Self::Error> {
        self.serialize_u64(value.into())
    }
    fn serialize_u16(self, value: u16) -> Result<(), Self::Error> {
        self.serialize_u64(value.into())
    }
    fn serialize_u32(self, value: u32) -> Result<(), Self::Error> {
        self.serialize_u64(value.into())
    }
    fn serialize_u64(mut self, value: u64) -> Result<(), Self::Error> {
        self.scalar(value.to_string().len())
    }
    fn serialize_f32(self, value: f32) -> Result<(), Self::Error> {
        self.serialize_f64(value.into())
    }
    fn serialize_f64(mut self, value: f64) -> Result<(), Self::Error> {
        self.scalar(value.to_string().len())
    }
    fn serialize_char(mut self, value: char) -> Result<(), Self::Error> {
        self.scalar(value.len_utf8())
    }
    fn serialize_str(mut self, value: &str) -> Result<(), Self::Error> {
        self.scalar(value.len())
    }
    fn serialize_bytes(mut self, value: &[u8]) -> Result<(), Self::Error> {
        self.collection(value.len(), false)
    }
    fn serialize_none(mut self) -> Result<(), Self::Error> {
        self.scalar(4)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_unit(mut self) -> Result<(), Self::Error> {
        self.scalar(4)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), Self::Error> {
        self.scalar(variant.len())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.collection(1, true)?;
        self.scalar_without_node(variant.len())?;
        value.serialize(self.child())
    }
    fn serialize_seq(mut self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        if let Some(len) = len {
            self.collection(len, false)?;
        } else {
            self.node()?;
        }
        Ok(MetricCompound {
            serializer: self,
            unknown_length: len.is_none(),
            length: 0,
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.collection(1, true)?;
        self.scalar_without_node(variant.len())?;
        let mut child = MetricSerializer {
            metrics: self.metrics,
            limits: self.limits,
            depth: self.depth.saturating_add(1),
        };
        child.collection(len, false)?;
        Ok(MetricCompound {
            serializer: child,
            unknown_length: false,
            length: 0,
        })
    }
    fn serialize_map(mut self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        if let Some(len) = len {
            self.collection(len, true)?;
        } else {
            self.node()?;
        }
        Ok(MetricCompound {
            serializer: self,
            unknown_length: len.is_none(),
            length: 0,
        })
    }
    fn serialize_struct(
        mut self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.collection(len, true)?;
        Ok(MetricCompound {
            serializer: self,
            unknown_length: false,
            length: 0,
        })
    }
    fn serialize_struct_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.collection(1, true)?;
        self.scalar_without_node(variant.len())?;
        let mut child = MetricSerializer {
            metrics: self.metrics,
            limits: self.limits,
            depth: self.depth.saturating_add(1),
        };
        child.collection(len, true)?;
        Ok(MetricCompound {
            serializer: child,
            unknown_length: false,
            length: 0,
        })
    }
}

impl ser::SerializeSeq for MetricCompound<'_> {
    type Ok = ();
    type Error = InputLimitError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        if self.unknown_length {
            self.length = self.length.saturating_add(1);
            if self.length > self.serializer.limits.collection_length {
                return Err(self.serializer.error(
                    "collection_length",
                    self.serializer.limits.collection_length,
                    self.length,
                ));
            }
        }
        value.serialize(self.serializer.child())
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTuple for MetricCompound<'_> {
    type Ok = ();
    type Error = InputLimitError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for MetricCompound<'_> {
    type Ok = ();
    type Error = InputLimitError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for MetricCompound<'_> {
    type Ok = ();
    type Error = InputLimitError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeMap for MetricCompound<'_> {
    type Ok = ();
    type Error = InputLimitError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        if self.unknown_length {
            self.length = self.length.saturating_add(1);
            if self.length > self.serializer.limits.collection_length {
                return Err(self.serializer.error(
                    "collection_length",
                    self.serializer.limits.collection_length,
                    self.length,
                ));
            }
            self.serializer.metrics.members = self.serializer.metrics.members.saturating_add(1);
            if self.serializer.metrics.members > self.serializer.limits.members {
                return Err(self.serializer.error(
                    "members",
                    self.serializer.limits.members,
                    self.serializer.metrics.members,
                ));
            }
        }
        key.serialize(MapKeySerializer {
            serializer: &mut self.serializer,
        })
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self.serializer.child())
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeStruct for MetricCompound<'_> {
    type Ok = ();
    type Error = InputLimitError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.serializer.scalar_without_node(key.len())?;
        value.serialize(self.serializer.child())
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeStructVariant for MetricCompound<'_> {
    type Ok = ();
    type Error = InputLimitError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        ser::SerializeStruct::serialize_field(self, key, value)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct MapKeySerializer<'a, 'b> {
    serializer: &'a mut MetricSerializer<'b>,
}

impl ser::Serializer for MapKeySerializer<'_, '_> {
    type Ok = ();
    type Error = InputLimitError;
    type SerializeSeq = ser::Impossible<(), InputLimitError>;
    type SerializeTuple = ser::Impossible<(), InputLimitError>;
    type SerializeTupleStruct = ser::Impossible<(), InputLimitError>;
    type SerializeTupleVariant = ser::Impossible<(), InputLimitError>;
    type SerializeMap = ser::Impossible<(), InputLimitError>;
    type SerializeStruct = ser::Impossible<(), InputLimitError>;
    type SerializeStructVariant = ser::Impossible<(), InputLimitError>;
    fn serialize_str(self, value: &str) -> Result<(), Self::Error> {
        self.serializer.scalar_without_node(value.len())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_bool(self, value: bool) -> Result<(), Self::Error> {
        self.serialize_str(if value { "true" } else { "false" })
    }
    fn serialize_i8(self, value: i8) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_i16(self, value: i16) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_i32(self, value: i32) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_i64(self, value: i64) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_u8(self, value: u8) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_u16(self, value: u16) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_u32(self, value: u32) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_u64(self, value: u64) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_f32(self, value: f32) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_f64(self, value: f64) -> Result<(), Self::Error> {
        self.serialize_str(&value.to_string())
    }
    fn serialize_char(self, value: char) -> Result<(), Self::Error> {
        self.serializer.scalar_without_node(value.len_utf8())
    }
    fn serialize_bytes(self, _value: &[u8]) -> Result<(), Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_none(self) -> Result<(), Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), Self::Error> {
        self.serialize_str(variant)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(ser::Error::custom("non-string map key"))
    }
}

pub(crate) fn check_input_source(bytes: &[u8], limits: InputLimits) -> Result<(), InputLimitError> {
    if bytes.len() > limits.bytes {
        return Err(InputLimitError {
            dimension: "bytes",
            maximum: limits.bytes,
            observed: bytes.len(),
            pointer: String::new(),
        });
    }
    let (tokens, depth) = source_metrics(bytes);
    for (dimension, maximum, observed) in [
        ("tokens", limits.tokens, tokens),
        ("depth", limits.depth, depth),
    ] {
        if observed > maximum {
            return Err(InputLimitError {
                dimension,
                maximum,
                observed,
                pointer: String::new(),
            });
        }
    }
    let metrics = source_value_metrics(bytes);
    for (dimension, maximum, observed) in [
        ("nodes", limits.nodes, metrics.nodes),
        ("members", limits.members, metrics.members),
        (
            "collection_length",
            limits.collection_length,
            metrics.collection_length,
        ),
        ("scalar_bytes", limits.scalar_bytes, metrics.scalar_bytes),
    ] {
        if observed > maximum {
            return Err(InputLimitError {
                dimension,
                maximum,
                observed,
                pointer: String::new(),
            });
        }
    }
    Ok(())
}

pub(crate) fn check_input_value(value: &Value, limits: InputLimits) -> Result<(), InputLimitError> {
    enum Frame<'a> {
        Value(&'a Value, usize, String),
        Object(serde_json::map::Iter<'a>, usize, String),
        Array(std::slice::Iter<'a, Value>, usize, usize, String),
    }

    let mut nodes = 0usize;
    let mut members = 0usize;
    let mut pending = vec![Frame::Value(value, 0, String::new())];
    while let Some(frame) = pending.pop() {
        let Frame::Value(value, depth, pointer) = frame else {
            match frame {
                Frame::Object(mut children, depth, pointer) => {
                    if let Some((key, child)) = children.next() {
                        if key.len() > limits.scalar_bytes {
                            return Err(InputLimitError {
                                dimension: "scalar_bytes",
                                maximum: limits.scalar_bytes,
                                observed: key.len(),
                                pointer: append_token(&pointer, key),
                            });
                        }
                        pending.push(Frame::Object(children, depth, pointer.clone()));
                        pending.push(Frame::Value(child, depth + 1, append_token(&pointer, key)));
                    }
                }
                Frame::Array(mut children, next_index, depth, pointer) => {
                    if let Some(child) = children.next() {
                        pending.push(Frame::Array(
                            children,
                            next_index + 1,
                            depth,
                            pointer.clone(),
                        ));
                        pending.push(Frame::Value(
                            child,
                            depth + 1,
                            format!("{pointer}/{next_index}"),
                        ));
                    }
                }
                Frame::Value(..) => unreachable!(),
            }
            continue;
        };

        nodes = nodes.saturating_add(1);
        for (dimension, maximum, observed) in [
            ("depth", limits.depth, depth),
            ("nodes", limits.nodes, nodes),
        ] {
            if observed > maximum {
                return Err(InputLimitError {
                    dimension,
                    maximum,
                    observed,
                    pointer,
                });
            }
        }

        let scalar_bytes = match value {
            Value::Null => 4,
            Value::Bool(true) => 4,
            Value::Bool(false) => 5,
            Value::Number(number) => number.as_str().len(),
            Value::String(value) => value.len(),
            Value::Array(values) => {
                if values.len() > limits.collection_length {
                    return Err(InputLimitError {
                        dimension: "collection_length",
                        maximum: limits.collection_length,
                        observed: values.len(),
                        pointer,
                    });
                }
                pending.push(Frame::Array(values.iter(), 0, depth, pointer));
                continue;
            }
            Value::Object(values) => {
                members = members.saturating_add(values.len());
                if members > limits.members {
                    return Err(InputLimitError {
                        dimension: "members",
                        maximum: limits.members,
                        observed: members,
                        pointer,
                    });
                }
                if values.len() > limits.collection_length {
                    return Err(InputLimitError {
                        dimension: "collection_length",
                        maximum: limits.collection_length,
                        observed: values.len(),
                        pointer,
                    });
                }
                pending.push(Frame::Object(values.iter(), depth, pointer));
                continue;
            }
        };
        if scalar_bytes > limits.scalar_bytes {
            return Err(InputLimitError {
                dimension: "scalar_bytes",
                maximum: limits.scalar_bytes,
                observed: scalar_bytes,
                pointer,
            });
        }
    }
    Ok(())
}

pub(crate) fn check_schema_source(
    bytes: &[u8],
    profile: &CompilationProfile,
) -> Result<(), CompilationLimitError> {
    let limits = profile.data_schema_limits();
    if bytes.len() > limits.bytes {
        return Err(CompilationLimitError::new(
            CompilationLimitPhase::Parse,
            CompilationLimitDimension::Bytes,
            limits.bytes,
            bytes.len(),
            String::new(),
        ));
    }
    let (tokens, depth) = source_metrics(bytes);
    if tokens > limits.tokens {
        return Err(CompilationLimitError::new(
            CompilationLimitPhase::Parse,
            CompilationLimitDimension::Tokens,
            limits.tokens,
            tokens,
            String::new(),
        ));
    }
    if depth > limits.depth {
        return Err(CompilationLimitError::new(
            CompilationLimitPhase::Structure,
            CompilationLimitDimension::Depth,
            limits.depth,
            depth,
            String::new(),
        ));
    }
    Ok(())
}

pub(crate) fn check_parsed_schema(
    bytes: usize,
    value: &Value,
    profile: &CompilationProfile,
) -> Result<(), CompilationLimitError> {
    let limits = profile.data_schema_limits();
    if bytes > limits.bytes {
        return Err(CompilationLimitError::new(
            CompilationLimitPhase::Parse,
            CompilationLimitDimension::Bytes,
            limits.bytes,
            bytes,
            String::new(),
        ));
    }
    let metrics = value_metrics(value, limits.depth);
    if metrics.tokens > limits.tokens {
        return Err(CompilationLimitError::new(
            CompilationLimitPhase::Parse,
            CompilationLimitDimension::Tokens,
            limits.tokens,
            metrics.tokens,
            String::new(),
        ));
    }
    check_structure_metrics(&metrics, limits)?;

    let (resources, references, traversal, uri_bytes) = graph_metrics(value, None);
    check_graph_metrics(resources, references, traversal, uri_bytes, limits)?;
    Ok(())
}

pub(crate) fn check_compilation_inputs(
    root_uri: &RetrievalUri,
    root: &Value,
    resources: &[SchemaResource],
    profile: &CompilationProfile,
) -> Result<(), CompilationLimitError> {
    let limits = profile.data_schema_limits();
    let mut aggregate = ValueMetrics::default();
    for document in std::iter::once(root).chain(resources.iter().map(SchemaResource::document)) {
        let metrics = value_metrics(document, limits.depth);
        if metrics.depth > limits.depth {
            return check_structure_metrics(&metrics, limits);
        }
        aggregate.nodes = aggregate.nodes.saturating_add(metrics.nodes);
        if metrics.depth > aggregate.depth {
            aggregate.depth = metrics.depth;
            aggregate.depth_pointer = metrics.depth_pointer;
        }
        if metrics.members > aggregate.members {
            aggregate.members = metrics.members;
            aggregate.members_pointer = metrics.members_pointer;
        }
        if metrics.scalar_bytes > aggregate.scalar_bytes {
            aggregate.scalar_bytes = metrics.scalar_bytes;
            aggregate.scalar_pointer = metrics.scalar_pointer;
        }
        if metrics.pointer_bytes > aggregate.pointer_bytes {
            aggregate.pointer_bytes = metrics.pointer_bytes;
            aggregate.pointer = metrics.pointer;
        }
    }

    check_structure_metrics(&aggregate, limits)?;

    let mut resource_count = resources.len() + 1;
    let mut reference_count = 0usize;
    let mut traversal = 0usize;
    let mut max_uri = root_uri.as_str().len();
    for resource in resources {
        max_uri = max_uri.max(resource.uri().as_str().len());
    }
    let (document_resources, document_references, document_traversal, document_uri_bytes) =
        graph_metrics(root, Some(root_uri.as_str()));
    resource_count = resource_count.saturating_add(document_resources.saturating_sub(1));
    reference_count = reference_count.saturating_add(document_references);
    traversal = traversal.saturating_add(document_traversal);
    max_uri = max_uri.max(document_uri_bytes);
    for resource in resources {
        let (document_resources, document_references, document_traversal, document_uri_bytes) =
            graph_metrics(resource.document(), Some(resource.uri().as_str()));
        resource_count = resource_count.saturating_add(document_resources.saturating_sub(1));
        reference_count = reference_count.saturating_add(document_references);
        traversal = traversal.saturating_add(document_traversal);
        max_uri = max_uri.max(document_uri_bytes);
    }

    check_graph_metrics(resource_count, reference_count, traversal, max_uri, limits)
}

fn check_structure_metrics(
    metrics: &ValueMetrics,
    limits: DataSchemaLimits,
) -> Result<(), CompilationLimitError> {
    for (dimension, maximum, observed, pointer) in [
        (
            CompilationLimitDimension::Depth,
            limits.depth,
            metrics.depth,
            metrics.depth_pointer.as_str(),
        ),
        (
            CompilationLimitDimension::Nodes,
            limits.nodes,
            metrics.nodes,
            "",
        ),
        (
            CompilationLimitDimension::Members,
            limits.members,
            metrics.members,
            metrics.members_pointer.as_str(),
        ),
        (
            CompilationLimitDimension::ScalarBytes,
            limits.scalar_bytes,
            metrics.scalar_bytes,
            metrics.scalar_pointer.as_str(),
        ),
        (
            CompilationLimitDimension::PointerBytes,
            limits.pointer_bytes,
            metrics.pointer_bytes,
            metrics.pointer.as_str(),
        ),
    ] {
        if observed > maximum {
            return Err(CompilationLimitError::new(
                CompilationLimitPhase::Structure,
                dimension,
                maximum,
                observed,
                pointer.to_owned(),
            ));
        }
    }
    Ok(())
}

fn graph_metrics(document: &Value, retrieval_uri: Option<&str>) -> (usize, usize, usize, usize) {
    let mut resource_count = 1usize;
    let mut reference_count = 0usize;
    let mut traversal = 0usize;
    let mut max_uri = retrieval_uri.map_or(0, str::len);
    let mut pending = vec![(document, true, retrieval_uri.unwrap_or_default().to_owned())];
    while let Some((schema, is_document_root, inherited_base)) = pending.pop() {
        traversal = traversal.saturating_add(1);
        let Some(object) = schema.as_object() else {
            continue;
        };
        let base = object
            .get("$id")
            .and_then(Value::as_str)
            .and_then(|identity| resolve_uri(&inherited_base, identity))
            .unwrap_or(inherited_base);
        if !is_document_root && object.get("$id").and_then(Value::as_str).is_some() {
            resource_count = resource_count.saturating_add(1);
        }
        max_uri = max_uri.max(base.len());
        for keyword in ["$schema", "$id", "$ref", "$dynamicRef"] {
            if let Some(value) = object.get(keyword).and_then(Value::as_str) {
                max_uri = max_uri.max(value.len());
                if matches!(keyword, "$ref" | "$dynamicRef")
                    && let Some(resolved) = resolve_uri(&base, value)
                {
                    max_uri = max_uri.max(resolved.len());
                }
            }
        }
        if let Some(vocabularies) = object.get("$vocabulary").and_then(Value::as_object) {
            max_uri = max_uri.max(vocabularies.keys().map(String::len).max().unwrap_or(0));
        }
        for keyword in ["$anchor", "$dynamicAnchor"] {
            if let Some(anchor) = object.get(keyword).and_then(Value::as_str) {
                max_uri = max_uri.max(base.len().saturating_add(anchor.len()).saturating_add(1));
            }
        }
        reference_count = reference_count.saturating_add(
            ["$ref", "$dynamicRef"]
                .into_iter()
                .filter(|keyword| object.get(*keyword).and_then(Value::as_str).is_some())
                .count(),
        );
        traversal = traversal.saturating_add(
            ["$ref", "$dynamicRef"]
                .into_iter()
                .filter(|keyword| object.get(*keyword).and_then(Value::as_str).is_some())
                .count(),
        );
        push_schema_children(object, &base, &mut pending);
    }
    (resource_count, reference_count, traversal, max_uri)
}

fn check_graph_metrics(
    resource_count: usize,
    reference_count: usize,
    traversal: usize,
    max_uri: usize,
    limits: DataSchemaLimits,
) -> Result<(), CompilationLimitError> {
    for (dimension, maximum, observed) in [
        (
            CompilationLimitDimension::Resources,
            limits.resources,
            resource_count,
        ),
        (
            CompilationLimitDimension::References,
            limits.references,
            reference_count,
        ),
        (
            CompilationLimitDimension::Traversal,
            limits.traversal,
            traversal,
        ),
        (
            CompilationLimitDimension::UriBytes,
            limits.uri_bytes,
            max_uri,
        ),
    ] {
        if observed > maximum {
            return Err(CompilationLimitError::new(
                CompilationLimitPhase::Graph,
                dimension,
                maximum,
                observed,
                String::new(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn check_compilation_outputs(
    definition_nodes: usize,
    controls: usize,
    capability_findings: usize,
    profile: &CompilationProfile,
) -> Result<(), CompilationLimitError> {
    let limits = profile.data_schema_limits();
    for (dimension, maximum, observed) in [
        (
            CompilationLimitDimension::DefinitionNodes,
            limits.definition_nodes,
            definition_nodes,
        ),
        (
            CompilationLimitDimension::Controls,
            limits.controls,
            controls,
        ),
        (
            CompilationLimitDimension::CapabilityFindings,
            limits.capability_findings,
            capability_findings,
        ),
    ] {
        if observed > maximum {
            return Err(CompilationLimitError::new(
                CompilationLimitPhase::Definition,
                dimension,
                maximum,
                observed,
                String::new(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn check_qualified_graph(
    metrics: (usize, usize, usize, usize, usize),
    profile: &CompilationProfile,
) -> Result<(), CompilationLimitError> {
    let limits = profile.data_schema_limits();
    let (resources, references, traversal, uri_bytes, pointer_bytes) = metrics;
    for (dimension, maximum, observed) in [
        (
            CompilationLimitDimension::Resources,
            limits.resources,
            resources,
        ),
        (
            CompilationLimitDimension::References,
            limits.references,
            references,
        ),
        (
            CompilationLimitDimension::Traversal,
            limits.traversal,
            traversal.saturating_add(references),
        ),
        (
            CompilationLimitDimension::UriBytes,
            limits.uri_bytes,
            uri_bytes,
        ),
        (
            CompilationLimitDimension::PointerBytes,
            limits.pointer_bytes,
            pointer_bytes,
        ),
    ] {
        if observed > maximum {
            return Err(CompilationLimitError::new(
                CompilationLimitPhase::Graph,
                dimension,
                maximum,
                observed,
                String::new(),
            ));
        }
    }
    Ok(())
}

fn value_metrics(value: &Value, depth_limit: usize) -> ValueMetrics {
    enum Frame<'a> {
        Value(&'a Value, usize, String),
        Object(serde_json::map::Iter<'a>, usize, String),
        Array(std::slice::Iter<'a, Value>, usize, usize, String),
    }

    let mut metrics = ValueMetrics::default();
    let mut pending = vec![Frame::Value(value, 0, String::new())];
    while let Some(frame) = pending.pop() {
        let Frame::Value(value, depth, pointer) = frame else {
            match frame {
                Frame::Object(mut children, depth, pointer) => {
                    if let Some((key, child)) = children.next() {
                        let key_pointer = append_token(&pointer, key);
                        if key.len() > metrics.scalar_bytes {
                            metrics.scalar_bytes = key.len();
                            metrics.scalar_pointer = key_pointer.clone();
                        }
                        pending.push(Frame::Object(children, depth, pointer.clone()));
                        pending.push(Frame::Value(child, depth + 1, key_pointer));
                    }
                }
                Frame::Array(mut children, next_index, depth, pointer) => {
                    if let Some(child) = children.next() {
                        pending.push(Frame::Array(
                            children,
                            next_index + 1,
                            depth,
                            pointer.clone(),
                        ));
                        pending.push(Frame::Value(
                            child,
                            depth + 1,
                            format!("{pointer}/{next_index}"),
                        ));
                    }
                }
                Frame::Value(..) => unreachable!(),
            }
            continue;
        };
        metrics.nodes = metrics.nodes.saturating_add(1);
        metrics.tokens = metrics.tokens.saturating_add(1);
        if depth > metrics.depth {
            metrics.depth = depth;
            metrics.depth_pointer = pointer.clone();
            if depth > depth_limit {
                break;
            }
        }
        if pointer.len() > metrics.pointer_bytes {
            metrics.pointer_bytes = pointer.len();
            metrics.pointer = pointer.clone();
        }
        match value {
            Value::Object(object) => {
                metrics.tokens = metrics.tokens.saturating_add(object.len());
                if object.len() > metrics.members {
                    metrics.members = object.len();
                    metrics.members_pointer = pointer.clone();
                }
                pending.push(Frame::Object(object.iter(), depth, pointer));
            }
            Value::Array(array) => {
                if array.len() > metrics.members {
                    metrics.members = array.len();
                    metrics.members_pointer = pointer.clone();
                }
                pending.push(Frame::Array(array.iter(), 0, depth, pointer));
            }
            Value::Null => record_scalar_metric(&mut metrics, 4, pointer),
            Value::Bool(true) => record_scalar_metric(&mut metrics, 4, pointer),
            Value::Bool(false) => record_scalar_metric(&mut metrics, 5, pointer),
            Value::Number(number) => {
                record_scalar_metric(&mut metrics, number.as_str().len(), pointer)
            }
            Value::String(value) => record_scalar_metric(&mut metrics, value.len(), pointer),
        }
    }
    metrics
}

fn record_scalar_metric(metrics: &mut ValueMetrics, bytes: usize, pointer: String) {
    if bytes > metrics.scalar_bytes {
        metrics.scalar_bytes = bytes;
        metrics.scalar_pointer = pointer;
    }
}

fn source_metrics(bytes: &[u8]) -> (usize, usize) {
    let mut tokens = 0usize;
    let mut depth = 0usize;
    let mut nesting = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'{' | b'[' => {
                tokens = tokens.saturating_add(1);
                depth = depth.max(nesting);
                nesting = nesting.saturating_add(1);
                index += 1;
            }
            b'}' | b']' => {
                nesting = nesting.saturating_sub(1);
                index += 1;
            }
            b'"' => {
                tokens = tokens.saturating_add(1);
                depth = depth.max(nesting);
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index = index.saturating_add(2).min(bytes.len()),
                        b'"' => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'-' | b'0'..=b'9' => {
                tokens = tokens.saturating_add(1);
                depth = depth.max(nesting);
                index += 1;
                while index < bytes.len()
                    && !matches!(
                        bytes[index],
                        b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}'
                    )
                {
                    index += 1;
                }
            }
            b't' | b'f' | b'n' => {
                tokens = tokens.saturating_add(1);
                depth = depth.max(nesting);
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    (tokens, depth)
}

#[derive(Default)]
struct SourceValueMetrics {
    nodes: usize,
    members: usize,
    collection_length: usize,
    scalar_bytes: usize,
}

fn source_value_metrics(bytes: &[u8]) -> SourceValueMetrics {
    let mut metrics = SourceValueMetrics::default();
    let mut collections: Vec<(u8, usize)> = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'{' | b'[' => {
                metrics.nodes = metrics.nodes.saturating_add(1);
                if let Some((b'[', length)) = collections.last_mut() {
                    *length = length.saturating_add(1);
                }
                collections.push((bytes[index], 0));
                index += 1;
            }
            b'}' | b']' => {
                if let Some((_, length)) = collections.pop() {
                    metrics.collection_length = metrics.collection_length.max(length);
                }
                index += 1;
            }
            b'"' => {
                let start = index + 1;
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index = index.saturating_add(2).min(bytes.len()),
                        b'"' => break,
                        _ => index += 1,
                    }
                }
                let scalar_bytes = json_string_decoded_len(&bytes[start..index]);
                let mut next = index.saturating_add(1);
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if next < bytes.len() && bytes[next] == b':' {
                    metrics.members = metrics.members.saturating_add(1);
                    if let Some((b'{', length)) = collections.last_mut() {
                        *length = length.saturating_add(1);
                    }
                } else {
                    metrics.nodes = metrics.nodes.saturating_add(1);
                    if let Some((b'[', length)) = collections.last_mut() {
                        *length = length.saturating_add(1);
                    }
                }
                metrics.scalar_bytes = metrics.scalar_bytes.max(scalar_bytes);
                index = index.saturating_add(1);
            }
            b'-' | b'0'..=b'9' => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && !matches!(
                        bytes[index],
                        b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}'
                    )
                {
                    index += 1;
                }
                record_source_scalar(&mut metrics, &mut collections, index - start);
            }
            b't' | b'f' | b'n' => {
                let start = index;
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
                record_source_scalar(&mut metrics, &mut collections, index - start);
            }
            _ => index += 1,
        }
    }
    for (_, length) in collections {
        metrics.collection_length = metrics.collection_length.max(length);
    }
    metrics
}

fn json_string_decoded_len(bytes: &[u8]) -> usize {
    let mut decoded = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            decoded = decoded.saturating_add(1);
            index += 1;
            continue;
        }
        if index + 1 >= bytes.len() {
            break;
        }
        if bytes[index + 1] != b'u' {
            decoded = decoded.saturating_add(1);
            index += 2;
            continue;
        }
        let Some(high) = parse_hex_quad(bytes.get(index + 2..index + 6)) else {
            index += 2;
            continue;
        };
        if (0xD800..=0xDBFF).contains(&high)
            && bytes.get(index + 6..index + 8) == Some(br"\u")
            && let Some(low) = parse_hex_quad(bytes.get(index + 8..index + 12))
            && (0xDC00..=0xDFFF).contains(&low)
        {
            decoded = decoded.saturating_add(4);
            index += 12;
            continue;
        }
        decoded = decoded.saturating_add(
            char::from_u32(u32::from(high))
                .map(char::len_utf8)
                .unwrap_or(3),
        );
        index += 6;
    }
    decoded
}

fn parse_hex_quad(bytes: Option<&[u8]>) -> Option<u16> {
    let bytes: [u8; 4] = bytes?.try_into().ok()?;
    bytes.into_iter().try_fold(0u16, |value, digit| {
        value.checked_mul(16)?.checked_add(match digit {
            b'0'..=b'9' => u16::from(digit - b'0'),
            b'a'..=b'f' => u16::from(digit - b'a' + 10),
            b'A'..=b'F' => u16::from(digit - b'A' + 10),
            _ => return None,
        })
    })
}

fn record_source_scalar(
    metrics: &mut SourceValueMetrics,
    collections: &mut [(u8, usize)],
    scalar_bytes: usize,
) {
    metrics.nodes = metrics.nodes.saturating_add(1);
    metrics.scalar_bytes = metrics.scalar_bytes.max(scalar_bytes);
    if let Some((b'[', length)) = collections.last_mut() {
        *length = length.saturating_add(1);
    }
}

fn append_token(pointer: &str, token: &str) -> String {
    format!("{pointer}/{}", token.replace('~', "~0").replace('/', "~1"))
}

fn resolve_uri(base: &str, reference: &str) -> Option<String> {
    if base.is_empty() {
        return Uri::parse(reference.to_owned())
            .ok()
            .map(|uri| uri.normalize().to_string());
    }
    let base = Uri::parse(base.to_owned()).ok()?;
    referencing::uri::resolve_against(&base.borrow(), reference)
        .ok()
        .map(|uri| uri.normalize().to_string())
}

fn push_schema_children<'a>(
    object: &'a serde_json::Map<String, Value>,
    base: &str,
    pending: &mut Vec<(&'a Value, bool, String)>,
) {
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
        if let Some(child) = object.get(keyword) {
            pending.push((child, false, base.to_owned()));
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(children) = object.get(keyword).and_then(Value::as_array) {
            pending.extend(
                children
                    .iter()
                    .rev()
                    .map(|child| (child, false, base.to_owned())),
            );
        }
    }
    for keyword in [
        "$defs",
        "definitions",
        "dependentSchemas",
        "patternProperties",
        "properties",
    ] {
        if let Some(children) = object.get(keyword).and_then(Value::as_object) {
            pending.extend(
                children
                    .values()
                    .rev()
                    .map(|child| (child, false, base.to_owned())),
            );
        }
    }
}
