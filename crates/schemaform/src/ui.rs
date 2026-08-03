use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CompilationProfile, ExtensionNamespace, JsonPointer, WidgetSymbol,
    definition::{ExtensionLimits, UiSchemaInputErrorKind},
};

/// Stable UI-schema wire version 1.
///
/// Version 1 freezes accepted JSON documents and their framework-neutral, headless meaning.
/// It does not freeze DOM structure, styling, or future accessibility corrections.
pub mod v1 {
    use super::*;

    fn deserialize_extension_map<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ExtensionNamespace, Value>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ExtensionMapVisitor;

        impl<'de> serde::de::Visitor<'de> for ExtensionMapVisitor {
            type Value = BTreeMap<ExtensionNamespace, Value>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an exact-URI extension map with unique keys")
            }

            fn visit_map<A>(self, mut values: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut extensions = BTreeMap::new();
                while let Some((namespace, value)) = values.next_entry()? {
                    if extensions.insert(namespace, value).is_some() {
                        return Err(serde::de::Error::custom(
                            "extension namespace keys must be unique",
                        ));
                    }
                }
                Ok(extensions)
            }
        }

        deserializer.deserialize_map(ExtensionMapVisitor)
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct UiSchema {
        version: Version,
        #[serde(default)]
        required_extensions: Vec<ExtensionNamespace>,
        root: Element,
    }

    impl UiSchema {
        pub fn new(root: Element) -> Self {
            Self {
                version: Version::V1,
                required_extensions: Vec::new(),
                root,
            }
        }

        pub fn require_extension(mut self, namespace: ExtensionNamespace) -> Self {
            self.required_extensions.push(namespace);
            self
        }

        pub fn root(&self) -> &Element {
            &self.root
        }

        pub fn required_extensions(&self) -> impl Iterator<Item = &ExtensionNamespace> {
            self.required_extensions.iter()
        }

        pub(crate) fn validate_limits(
            &self,
            profile: &CompilationProfile,
        ) -> Result<(), ExtensionValidationError> {
            crate::limits::check_serializable_input(self, profile.ui_schema_limits())
                .map_err(ExtensionValidationError::input_resource)?;
            self.validate_extensions(profile)
        }

        fn validate_extensions(
            &self,
            profile: &CompilationProfile,
        ) -> Result<(), ExtensionValidationError> {
            let limits = profile.extension_limits();
            let mut declared = BTreeSet::new();
            for (index, namespace) in self.required_extensions.iter().enumerate() {
                let location = format!("/required_extensions/{index}");
                if index >= limits.namespaces {
                    return Err(ExtensionValidationError::resource(
                        location,
                        "extension_namespaces",
                        limits.namespaces,
                        index.saturating_add(1),
                    ));
                }
                if namespace.as_str().len() > limits.namespace_bytes {
                    return Err(ExtensionValidationError::resource(
                        location,
                        "extension_namespace_bytes",
                        limits.namespace_bytes,
                        namespace.as_str().len(),
                    ));
                }
                if !declared.insert(namespace) {
                    return Err(ExtensionValidationError::invalid(
                        format!("/required_extensions/{index}"),
                        UiSchemaInputErrorKind::DuplicateRequiredExtension,
                    ));
                }
            }

            let mut state = ExtensionValidationState::default();
            validate_element_extensions(&self.root, "/root", limits, &mut state)?;
            for (index, namespace) in self.required_extensions.iter().enumerate() {
                if !state.namespaces.contains(namespace) {
                    return Err(ExtensionValidationError::invalid(
                        format!("/required_extensions/{index}"),
                        UiSchemaInputErrorKind::MissingRequiredExtension,
                    ));
                }
            }
            Ok(())
        }
    }

    pub(crate) struct ExtensionValidationError {
        pub(crate) location: String,
        pub(crate) kind: UiSchemaInputErrorKind,
        pub(crate) limit: Option<crate::limits::InputLimitError>,
    }

    impl ExtensionValidationError {
        fn invalid(location: String, kind: UiSchemaInputErrorKind) -> Self {
            Self {
                location,
                kind,
                limit: None,
            }
        }

        fn input_resource(limit: crate::limits::InputLimitError) -> Self {
            Self {
                location: limit.pointer.clone(),
                kind: UiSchemaInputErrorKind::ResourceLimit,
                limit: Some(limit),
            }
        }

        fn resource(
            location: impl Into<String>,
            dimension: &'static str,
            maximum: usize,
            observed: usize,
        ) -> Self {
            Self::input_resource(crate::limits::InputLimitError {
                dimension,
                maximum,
                observed,
                pointer: location.into(),
            })
        }
    }

    fn extension_resource(
        location: &str,
        dimension: &'static str,
        maximum: usize,
        observed: usize,
    ) -> ExtensionValidationError {
        ExtensionValidationError::resource(location.to_owned(), dimension, maximum, observed)
    }

    #[derive(Default)]
    struct ExtensionValidationState<'a> {
        namespaces: BTreeSet<&'a ExtensionNamespace>,
        occurrences: usize,
    }

    fn validate_element_extensions<'a>(
        element: &'a Element,
        location: &str,
        limits: ExtensionLimits,
        state: &mut ExtensionValidationState<'a>,
    ) -> Result<(), ExtensionValidationError> {
        let (meta, children): (&ElementMeta, Vec<(&Element, String)>) = match element {
            Element::Control(control) => (
                control.meta_value(),
                control
                    .item_template_value()
                    .map(|child| (child, format!("{location}/value/item_template")))
                    .into_iter()
                    .collect(),
            ),
            Element::Auto(auto) => (auto.meta_value(), Vec::new()),
            Element::Stack(stack) => (
                stack.meta_value(),
                stack
                    .children()
                    .iter()
                    .enumerate()
                    .map(|(index, child)| (child, format!("{location}/value/children/{index}")))
                    .collect(),
            ),
            Element::Grid(grid) => (
                grid.meta_value(),
                grid.cells()
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| {
                        (
                            cell.child(),
                            format!("{location}/value/cells/{index}/child"),
                        )
                    })
                    .collect(),
            ),
            Element::Group(group) => (
                group.meta_value(),
                vec![(group.child(), format!("{location}/value/child"))],
            ),
            Element::Tabs(tabs) => (
                tabs.meta_value(),
                tabs.panels()
                    .iter()
                    .enumerate()
                    .map(|(index, panel)| {
                        (
                            panel.child(),
                            format!("{location}/value/panels/{index}/child"),
                        )
                    })
                    .collect(),
            ),
            Element::Text(text) => (text.meta_value(), Vec::new()),
        };

        for (namespace, value) in meta.extensions_value() {
            let namespace_location = format!(
                "{location}/value/extensions/{}",
                namespace.as_str().replace('~', "~0").replace('/', "~1")
            );
            if namespace.as_str().len() > limits.namespace_bytes {
                return Err(extension_resource(
                    &namespace_location,
                    "extension_namespace_bytes",
                    limits.namespace_bytes,
                    namespace.as_str().len(),
                ));
            }
            if !state.namespaces.contains(namespace) && state.namespaces.len() >= limits.namespaces
            {
                return Err(extension_resource(
                    &namespace_location,
                    "extension_namespaces",
                    limits.namespaces,
                    state.namespaces.len().saturating_add(1),
                ));
            }
            state.namespaces.insert(namespace);
            state.occurrences += 1;
            if state.occurrences > limits.occurrences {
                return Err(extension_resource(
                    &namespace_location,
                    "extension_occurrences",
                    limits.occurrences,
                    state.occurrences,
                ));
            }
            let mut value_nodes = 0usize;
            let mut value_bytes = 0usize;
            let mut pending = vec![(value, 1usize)];
            while let Some((value, depth)) = pending.pop() {
                if depth > limits.value_depth {
                    return Err(extension_resource(
                        &namespace_location,
                        "extension_value_depth",
                        limits.value_depth,
                        depth,
                    ));
                }
                value_nodes += 1;
                if value_nodes > limits.value_nodes {
                    return Err(extension_resource(
                        &namespace_location,
                        "extension_value_nodes",
                        limits.value_nodes,
                        value_nodes,
                    ));
                }
                let child_count = match value {
                    Value::Array(values) => values.len(),
                    Value::Object(values) => values.len(),
                    _ => 0,
                };
                let projected_nodes = value_nodes
                    .saturating_add(pending.len())
                    .saturating_add(child_count);
                if projected_nodes > limits.value_nodes {
                    return Err(extension_resource(
                        &namespace_location,
                        "extension_value_nodes",
                        limits.value_nodes,
                        projected_nodes,
                    ));
                }
                value_bytes = value_bytes.saturating_add(match value {
                    Value::Null => 4,
                    Value::Bool(true) => 4,
                    Value::Bool(false) => 5,
                    Value::Number(number) => number.as_str().len(),
                    Value::String(value) => value.len(),
                    Value::Array(_) => 0,
                    Value::Object(values) => values.keys().map(String::len).sum(),
                });
                if value_bytes > limits.value_bytes {
                    return Err(extension_resource(
                        &namespace_location,
                        "extension_value_bytes",
                        limits.value_bytes,
                        value_bytes,
                    ));
                }
                match value {
                    Value::Array(values) => {
                        pending.extend(values.iter().map(|value| (value, depth + 1)));
                    }
                    Value::Object(values) => {
                        pending.extend(values.values().map(|value| (value, depth + 1)));
                    }
                    _ => {}
                }
            }
        }

        for (child, child_location) in children {
            validate_element_extensions(child, &child_location, limits, state)?;
        }
        Ok(())
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
    #[serde(into = "u64")]
    pub enum Version {
        V1,
    }

    impl<'de> Deserialize<'de> for Version {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let number = serde_json::Number::deserialize(deserializer)?;
            crate::engine::json_values_equal(
                &Value::Number(number),
                &Value::Number(serde_json::Number::from(1)),
            )
            .then_some(Self::V1)
            .ok_or_else(|| serde::de::Error::custom("UI schema version must be 1"))
        }
    }

    impl TryFrom<u64> for Version {
        type Error = &'static str;

        fn try_from(value: u64) -> Result<Self, Self::Error> {
            (value == 1)
                .then_some(Self::V1)
                .ok_or("UI schema version must be 1")
        }
    }

    impl From<Version> for u64 {
        fn from(_: Version) -> Self {
            1
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    #[serde(tag = "type", content = "value", rename_all = "snake_case")]
    pub enum Element {
        Control(Control),
        Auto(Auto),
        Stack(Stack),
        Grid(Grid),
        Group(Group),
        Tabs(Tabs),
        Text(Text),
    }

    impl Element {
        pub fn control(&self) -> Option<&Control> {
            match self {
                Self::Control(control) => Some(control),
                _ => None,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
    pub struct ElementMeta {
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
    }

    impl ElementMeta {
        pub fn id(mut self, id: impl Into<String>) -> Self {
            self.id = Some(id.into());
            self
        }

        pub fn extension(mut self, namespace: ExtensionNamespace, value: Value) -> Self {
            self.extensions.insert(namespace, value);
            self
        }

        pub(crate) fn id_value(&self) -> Option<&str> {
            self.id.as_deref()
        }
        pub(crate) fn extensions_value(&self) -> &BTreeMap<ExtensionNamespace, Value> {
            &self.extensions
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Binding {
        origin: BindingOrigin,
        pointer: JsonPointer,
    }

    impl Binding {
        pub fn root(pointer: JsonPointer) -> Self {
            Self {
                origin: BindingOrigin::Root,
                pointer,
            }
        }

        pub fn item(pointer: JsonPointer) -> Self {
            Self {
                origin: BindingOrigin::ItemTemplate,
                pointer,
            }
        }

        pub fn origin(&self) -> BindingOrigin {
            self.origin
        }
        pub fn pointer(&self) -> &JsonPointer {
            &self.pointer
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum BindingOrigin {
        Root,
        ItemTemplate,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct TextReference {
        fallback: String,
        key: Option<String>,
    }

    impl TextReference {
        pub fn literal(fallback: impl Into<String>) -> Self {
            Self {
                fallback: fallback.into(),
                key: None,
            }
        }

        pub fn localized(key: impl Into<String>, fallback: impl Into<String>) -> Self {
            Self {
                fallback: fallback.into(),
                key: Some(key.into()),
            }
        }

        pub fn fallback(&self) -> &str {
            &self.fallback
        }

        pub fn key(&self) -> Option<&str> {
            self.key.as_deref()
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
    #[serde(rename_all = "snake_case")]
    pub enum TextSetting {
        #[default]
        Inherit,
        Suppress,
        Value(TextReference),
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct Control {
        #[serde(flatten)]
        meta: ElementMeta,
        binding: Binding,
        #[serde(default)]
        label: TextSetting,
        #[serde(default)]
        help: TextSetting,
        widget: Option<WidgetSymbol>,
        item_template: Option<Box<Element>>,
        item_label: Option<TextReference>,
    }

    impl Control {
        pub fn new(binding: Binding) -> Self {
            Self {
                meta: ElementMeta::default(),
                binding,
                label: TextSetting::Inherit,
                help: TextSetting::Inherit,
                widget: None,
                item_template: None,
                item_label: None,
            }
        }

        pub fn widget(mut self, widget: WidgetSymbol) -> Self {
            self.widget = Some(widget);
            self
        }

        pub fn label(mut self, label: TextSetting) -> Self {
            self.label = label;
            self
        }

        pub fn help(mut self, help: TextSetting) -> Self {
            self.help = help;
            self
        }

        pub fn item_label(mut self, label: TextReference) -> Self {
            self.item_label = Some(label);
            self
        }

        pub fn meta(mut self, meta: ElementMeta) -> Self {
            self.meta = meta;
            self
        }

        pub fn item_template(mut self, template: Element) -> Self {
            self.item_template = Some(Box::new(template));
            self
        }

        pub fn binding(&self) -> &Binding {
            &self.binding
        }
        pub fn label_setting(&self) -> &TextSetting {
            &self.label
        }
        pub fn help_setting(&self) -> &TextSetting {
            &self.help
        }
        pub(crate) fn meta_value(&self) -> &ElementMeta {
            &self.meta
        }
        pub(crate) fn widget_value(&self) -> Option<&WidgetSymbol> {
            self.widget.as_ref()
        }
        pub(crate) fn item_template_value(&self) -> Option<&Element> {
            self.item_template.as_deref()
        }
        pub(crate) fn item_label_value(&self) -> Option<&TextReference> {
            self.item_label.as_ref()
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct Auto {
        #[serde(flatten)]
        meta: ElementMeta,
        binding: Binding,
        #[serde(default)]
        properties: PropertySelection,
    }

    impl Auto {
        pub fn new(binding: Binding) -> Self {
            Self {
                meta: ElementMeta::default(),
                binding,
                properties: PropertySelection::default(),
            }
        }

        pub fn properties(mut self, properties: PropertySelection) -> Self {
            self.properties = properties;
            self
        }

        pub fn meta(mut self, meta: ElementMeta) -> Self {
            self.meta = meta;
            self
        }

        pub fn binding(&self) -> &Binding {
            &self.binding
        }
        pub(crate) fn meta_value(&self) -> &ElementMeta {
            &self.meta
        }
        pub(crate) fn properties_value(&self) -> &PropertySelection {
            &self.properties
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
    #[serde(deny_unknown_fields)]
    pub struct PropertySelection {
        #[serde(default)]
        include: Vec<String>,
        #[serde(default)]
        exclude: Vec<String>,
        #[serde(default)]
        order: Vec<PropertyPosition>,
    }

    impl PropertySelection {
        pub fn include(mut self, property: impl Into<String>) -> Self {
            self.include.push(property.into());
            self
        }

        pub fn exclude(mut self, property: impl Into<String>) -> Self {
            self.exclude.push(property.into());
            self
        }

        pub fn order(mut self, positions: impl IntoIterator<Item = PropertyPosition>) -> Self {
            self.order = positions.into_iter().collect();
            self
        }

        pub(crate) fn include_value(&self) -> &[String] {
            &self.include
        }
        pub(crate) fn exclude_value(&self) -> &[String] {
            &self.exclude
        }
        pub(crate) fn order_value(&self) -> &[PropertyPosition] {
            &self.order
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PropertyPosition {
        Property(String),
        Remaining,
    }

    macro_rules! container {
        ($name:ident, $field:ident: Vec<$item:ty>) => {
            #[derive(Debug, Clone, PartialEq, Serialize)]
            pub struct $name {
                #[serde(flatten)]
                meta: ElementMeta,
                $field: Vec<$item>,
            }

            impl $name {
                pub fn new(values: impl IntoIterator<Item = $item>) -> Self {
                    Self {
                        meta: ElementMeta::default(),
                        $field: values.into_iter().collect(),
                    }
                }

                pub fn meta(mut self, meta: ElementMeta) -> Self {
                    self.meta = meta;
                    self
                }

                #[allow(dead_code)]
                pub(crate) fn meta_value(&self) -> &ElementMeta {
                    &self.meta
                }
                pub fn $field(&self) -> &[$item] {
                    &self.$field
                }
            }
        };
    }

    container!(Stack, children: Vec<Element>);
    container!(Grid, cells: Vec<GridCell>);
    container!(Tabs, panels: Vec<TabPanel>);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(try_from = "u8", into = "u8")]
    pub struct GridSpan(u8);

    impl GridSpan {
        pub fn new(value: u8) -> Result<Self, &'static str> {
            Self::try_from(value)
        }

        pub fn get(self) -> u8 {
            self.0
        }
    }

    impl TryFrom<u8> for GridSpan {
        type Error = &'static str;

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            (1..=12)
                .contains(&value)
                .then_some(Self(value))
                .ok_or("grid spans must be from 1 through 12")
        }
    }

    impl From<GridSpan> for u8 {
        fn from(value: GridSpan) -> Self {
            value.get()
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct GridCell {
        compact_span: GridSpan,
        wide_span: Option<GridSpan>,
        child: Element,
    }

    impl GridCell {
        pub fn new(compact_span: GridSpan, child: Element) -> Self {
            Self {
                compact_span,
                wide_span: None,
                child,
            }
        }

        pub fn wide_span(mut self, span: GridSpan) -> Self {
            self.wide_span = Some(span);
            self
        }

        pub fn compact_span(&self) -> GridSpan {
            self.compact_span
        }

        pub fn effective_wide_span(&self) -> GridSpan {
            self.wide_span.unwrap_or(self.compact_span)
        }

        pub fn child(&self) -> &Element {
            &self.child
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct Group {
        #[serde(flatten)]
        meta: ElementMeta,
        title: TextReference,
        child: Box<Element>,
    }

    impl Group {
        pub fn new(title: TextReference, child: Element) -> Self {
            Self {
                meta: ElementMeta::default(),
                title,
                child: Box::new(child),
            }
        }

        pub fn meta(mut self, meta: ElementMeta) -> Self {
            self.meta = meta;
            self
        }

        pub fn title(&self) -> &TextReference {
            &self.title
        }
        pub fn child(&self) -> &Element {
            &self.child
        }
        pub(crate) fn meta_value(&self) -> &ElementMeta {
            &self.meta
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct TabPanel {
        title: TextReference,
        child: Element,
    }

    impl TabPanel {
        pub fn new(title: TextReference, child: Element) -> Self {
            Self { title, child }
        }

        pub fn title(&self) -> &TextReference {
            &self.title
        }

        pub fn child(&self) -> &Element {
            &self.child
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct Text {
        #[serde(flatten)]
        meta: ElementMeta,
        content: TextReference,
    }

    impl Text {
        pub fn new(content: TextReference) -> Self {
            Self {
                meta: ElementMeta::default(),
                content,
            }
        }

        pub fn meta(mut self, meta: ElementMeta) -> Self {
            self.meta = meta;
            self
        }

        pub fn content(&self) -> &TextReference {
            &self.content
        }
        pub(crate) fn meta_value(&self) -> &ElementMeta {
            &self.meta
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireUiSchema {
        version: Version,
        #[serde(default)]
        required_extensions: Vec<ExtensionNamespace>,
        root: Element,
    }

    impl<'de> Deserialize<'de> for UiSchema {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let wire = WireUiSchema::deserialize(deserializer)?;
            Ok(Self {
                version: wire.version,
                required_extensions: wire.required_extensions,
                root: wire.root,
            })
        }
    }

    #[derive(Deserialize)]
    #[serde(
        tag = "type",
        content = "value",
        rename_all = "snake_case",
        deny_unknown_fields
    )]
    enum WireElement {
        Control(WireControl),
        Auto(WireAuto),
        Stack(WireStack),
        Grid(WireGrid),
        Group(WireGroup),
        Tabs(WireTabs),
        Text(WireText),
    }

    impl<'de> Deserialize<'de> for Element {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            Ok(match WireElement::deserialize(deserializer)? {
                WireElement::Control(value) => Self::Control(value.into()),
                WireElement::Auto(value) => Self::Auto(value.into()),
                WireElement::Stack(value) => Self::Stack(value.into()),
                WireElement::Grid(value) => Self::Grid(value.into()),
                WireElement::Group(value) => Self::Group(value.into()),
                WireElement::Tabs(value) => Self::Tabs(value.into()),
                WireElement::Text(value) => Self::Text(value.into()),
            })
        }
    }

    fn meta(id: Option<String>, extensions: BTreeMap<ExtensionNamespace, Value>) -> ElementMeta {
        ElementMeta { id, extensions }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireControl {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
        binding: Binding,
        #[serde(default)]
        label: TextSetting,
        #[serde(default)]
        help: TextSetting,
        #[serde(default)]
        widget: Option<WidgetSymbol>,
        #[serde(default)]
        item_template: Option<Box<Element>>,
        #[serde(default)]
        item_label: Option<TextReference>,
    }

    impl From<WireControl> for Control {
        fn from(value: WireControl) -> Self {
            Self {
                meta: meta(value.id, value.extensions),
                binding: value.binding,
                label: value.label,
                help: value.help,
                widget: value.widget,
                item_template: value.item_template,
                item_label: value.item_label,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireAuto {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
        binding: Binding,
        #[serde(default)]
        properties: PropertySelection,
    }
    impl From<WireAuto> for Auto {
        fn from(value: WireAuto) -> Self {
            Self {
                meta: meta(value.id, value.extensions),
                binding: value.binding,
                properties: value.properties,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireStack {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
        children: Vec<Element>,
    }
    impl From<WireStack> for Stack {
        fn from(value: WireStack) -> Self {
            Self {
                meta: meta(value.id, value.extensions),
                children: value.children,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireGrid {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
        cells: Vec<WireGridCell>,
    }
    impl From<WireGrid> for Grid {
        fn from(value: WireGrid) -> Self {
            Self {
                meta: meta(value.id, value.extensions),
                cells: value.cells.into_iter().map(Into::into).collect(),
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireGridCell {
        compact_span: GridSpan,
        #[serde(default)]
        wide_span: Option<GridSpan>,
        child: Element,
    }
    impl From<WireGridCell> for GridCell {
        fn from(value: WireGridCell) -> Self {
            Self {
                compact_span: value.compact_span,
                wide_span: value.wide_span,
                child: value.child,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireGroup {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
        title: TextReference,
        child: Box<Element>,
    }
    impl From<WireGroup> for Group {
        fn from(value: WireGroup) -> Self {
            Self {
                meta: meta(value.id, value.extensions),
                title: value.title,
                child: value.child,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireTabs {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
        panels: Vec<WireTabPanel>,
    }
    impl From<WireTabs> for Tabs {
        fn from(value: WireTabs) -> Self {
            Self {
                meta: meta(value.id, value.extensions),
                panels: value.panels.into_iter().map(Into::into).collect(),
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireTabPanel {
        title: TextReference,
        child: Element,
    }
    impl From<WireTabPanel> for TabPanel {
        fn from(value: WireTabPanel) -> Self {
            Self {
                title: value.title,
                child: value.child,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireText {
        #[serde(default)]
        id: Option<String>,
        #[serde(default, deserialize_with = "deserialize_extension_map")]
        extensions: BTreeMap<ExtensionNamespace, Value>,
        content: TextReference,
    }
    impl From<WireText> for Text {
        fn from(value: WireText) -> Self {
            Self {
                meta: meta(value.id, value.extensions),
                content: value.content,
            }
        }
    }

    macro_rules! strict_deserialize {
        ($public:ty, $wire:ty) => {
            impl<'de> Deserialize<'de> for $public {
                fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: serde::Deserializer<'de>,
                {
                    <$wire>::deserialize(deserializer).map(Into::into)
                }
            }
        };
    }

    strict_deserialize!(Control, WireControl);
    strict_deserialize!(Auto, WireAuto);
    strict_deserialize!(Stack, WireStack);
    strict_deserialize!(Grid, WireGrid);
    strict_deserialize!(GridCell, WireGridCell);
    strict_deserialize!(Group, WireGroup);
    strict_deserialize!(Tabs, WireTabs);
    strict_deserialize!(TabPanel, WireTabPanel);
    strict_deserialize!(Text, WireText);
}
