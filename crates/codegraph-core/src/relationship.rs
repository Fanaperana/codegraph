use serde::{Deserialize, Serialize};

/// A relationship between two code entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Qualified name of the source entity
    pub from: String,
    /// Qualified name of the target entity
    pub to: String,
    /// Kind of relationship
    pub kind: RelationshipKind,
    /// Additional metadata
    pub metadata: RelationshipMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationshipKind {
    /// File/Module contains an entity
    Contains,
    /// Module defines an entity
    Defines,
    /// Function/method calls another function/method
    Calls,
    /// File/module depends on another (use/mod/extern)
    DependsOn,
    /// Impl implements a trait
    Implements,
    /// Impl is for a type
    ImplFor,
    /// Impl has a method
    HasMethod,
    /// Trait extends another trait (supertrait)
    Extends,
    /// Function returns a type
    Returns,
    /// Function accepts a parameter of a type
    AcceptsParam,
    /// Struct has a field of a type
    HasField,
    /// Enum has a variant
    HasVariant,
    /// Entity uses a type (generic, reference, etc.)
    UsesType,
    /// File/module imports an entity
    Imports,
}

impl RelationshipKind {
    pub fn neo4j_type(&self) -> &'static str {
        match self {
            Self::Contains => "CONTAINS",
            Self::Defines => "DEFINES",
            Self::Calls => "CALLS",
            Self::DependsOn => "DEPENDS_ON",
            Self::Implements => "IMPLEMENTS",
            Self::ImplFor => "IMPL_FOR",
            Self::HasMethod => "HAS_METHOD",
            Self::Extends => "EXTENDS",
            Self::Returns => "RETURNS",
            Self::AcceptsParam => "ACCEPTS_PARAM",
            Self::HasField => "HAS_FIELD",
            Self::HasVariant => "HAS_VARIANT",
            Self::UsesType => "USES_TYPE",
            Self::Imports => "IMPORTS",
        }
    }
}

/// Optional metadata attached to a relationship.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelationshipMeta {
    /// Line number where the relationship occurs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Parameter name (for AcceptsParam)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_name: Option<String>,
    /// Parameter position (for AcceptsParam)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    /// Field name (for HasField)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_name: Option<String>,
    /// Field type as string (for HasField)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    /// Variant name (for HasVariant)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_name: Option<String>,
    /// Import alias (for Imports)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Dependency kind (for DependsOn): "use", "mod", "extern"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dep_kind: Option<String>,
}
