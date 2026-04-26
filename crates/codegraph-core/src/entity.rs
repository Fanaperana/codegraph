use serde::{Deserialize, Serialize};

/// A code entity extracted from source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeEntity {
    /// Unique identifier: typically `file_path::module::name`
    pub qualified_name: String,
    /// Short name of the entity
    pub name: String,
    /// What kind of entity this is
    pub kind: EntityKind,
    /// File path where this entity is defined
    pub file_path: String,
    /// Module path (e.g. `crate::config`)
    pub module_path: Option<String>,
    /// Visibility (pub, pub(crate), private)
    pub visibility: Visibility,
    /// Line where the entity starts (1-based)
    pub line_start: u32,
    /// Line where the entity ends (1-based)
    pub line_end: u32,
    /// Documentation comment, if any
    pub doc_comment: Option<String>,
    /// The full source text of the entity
    pub source_text: String,
    /// Blake3 hash of `source_text` for incremental indexing
    pub source_hash: String,
    /// Function/method signature (for Function/Method kinds)
    pub signature: Option<String>,
    /// Whether the function is async (for Function/Method kinds)
    pub is_async: bool,
    /// Embedding vector (populated by codegraph-embed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl CodeEntity {
    /// Compute and set the `source_hash` from `source_text`.
    pub fn compute_hash(&mut self) {
        self.source_hash = blake3::hash(self.source_text.as_bytes())
            .to_hex()
            .to_string();
    }

    /// Create a text representation suitable for embedding.
    /// Includes the kind, name, signature, and doc comment for semantic richness.
    pub fn embedding_text(&self) -> String {
        let mut parts = vec![format!("{:?}: {}", self.kind, self.name)];
        if let Some(sig) = &self.signature {
            parts.push(sig.clone());
        }
        if let Some(doc) = &self.doc_comment {
            parts.push(doc.clone());
        }
        parts.push(self.source_text.clone());
        parts.join("\n")
    }

    /// Neo4j label string for this entity's kind.
    pub fn neo4j_label(&self) -> &'static str {
        self.kind.neo4j_label()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityKind {
    // Universal
    File,
    Module,
    Function,
    Method,
    Constant,
    Static,
    Variable,
    TypeAlias,

    // Rust-specific
    Struct,
    Enum,
    Trait,
    Impl,
    Macro,

    // OOP (C++, Python, JS/TS)
    Class,
    Interface,
    Constructor,
    Property,
    Decorator,

    // C/C++
    Header,
    Namespace,
    Union,
    Typedef,
    Preprocessor,

    // HTML/CSS
    HtmlElement,
    CssRule,
    CssSelector,
}

impl EntityKind {
    pub fn neo4j_label(&self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Module => "Module",
            Self::Function => "Function",
            Self::Method => "Method",
            Self::Constant => "Constant",
            Self::Static => "Static",
            Self::Variable => "Variable",
            Self::TypeAlias => "TypeAlias",
            Self::Struct => "Struct",
            Self::Enum => "Enum",
            Self::Trait => "Trait",
            Self::Impl => "Impl",
            Self::Macro => "Macro",
            Self::Class => "Class",
            Self::Interface => "Interface",
            Self::Constructor => "Constructor",
            Self::Property => "Property",
            Self::Decorator => "Decorator",
            Self::Header => "Header",
            Self::Namespace => "Namespace",
            Self::Union => "Union",
            Self::Typedef => "Typedef",
            Self::Preprocessor => "Preprocessor",
            Self::HtmlElement => "HtmlElement",
            Self::CssRule => "CssRule",
            Self::CssSelector => "CssSelector",
        }
    }

    pub fn all_labels() -> &'static [&'static str] {
        &[
            "File",
            "Module",
            "Function",
            "Method",
            "Constant",
            "Static",
            "Variable",
            "TypeAlias",
            "Struct",
            "Enum",
            "Trait",
            "Impl",
            "Macro",
            "Class",
            "Interface",
            "Constructor",
            "Property",
            "Decorator",
            "Header",
            "Namespace",
            "Union",
            "Typedef",
            "Preprocessor",
            "HtmlElement",
            "CssRule",
            "CssSelector",
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum Visibility {
    Public,
    PublicCrate,
    PublicSuper,
    #[default]
    Private,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public => write!(f, "pub"),
            Self::PublicCrate => write!(f, "pub(crate)"),
            Self::PublicSuper => write!(f, "pub(super)"),
            Self::Private => write!(f, "private"),
        }
    }
}
