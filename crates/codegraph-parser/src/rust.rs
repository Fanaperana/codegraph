use std::path::Path;

use codegraph_core::entity::{CodeEntity, EntityKind, Visibility};
use codegraph_core::relationship::{Relationship, RelationshipKind, RelationshipMeta};
use codegraph_core::{CodeGraph, Result};
use tree_sitter::{Node, Parser};

use crate::LanguageParser;

pub struct RustParser {
    _private: (),
}

impl RustParser {
    pub fn new() -> Self {
        Self { _private: () }
    }

    fn create_parser() -> Result<Parser> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| codegraph_core::Error::Parser(e.to_string()))?;
        Ok(parser)
    }
}

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for RustParser {
    fn extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph> {
        let mut parser = Self::create_parser()?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| codegraph_core::Error::Parser("Failed to parse file".into()))?;

        let file_path = path.to_string_lossy().to_string();
        let mut graph = CodeGraph::new();

        // Create the File entity
        let file_hash = blake3::hash(source.as_bytes()).to_hex().to_string();
        let file_entity = CodeEntity {
            qualified_name: file_path.clone(),
            name: path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default(),
            kind: EntityKind::File,
            file_path: file_path.clone(),
            module_path: None,
            visibility: Visibility::Public,
            line_start: 1,
            line_end: source.lines().count() as u32,
            doc_comment: None,
            source_text: String::new(), // Don't store entire file source
            source_hash: file_hash,
            signature: None,
            is_async: false,
            embedding: None,
        };
        graph.add_entity(file_entity);

        // Walk the AST
        let root = tree.root_node();
        extract_entities(
            root,
            source,
            &file_path,
            &file_path,
            &mut graph,
        );

        Ok(graph)
    }
}

/// Recursively extract entities and relationships from the AST.
fn extract_entities(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                extract_function(child, source, file_path, parent_qualified, graph, false);
            }
            "struct_item" => {
                extract_struct(child, source, file_path, parent_qualified, graph);
            }
            "enum_item" => {
                extract_enum(child, source, file_path, parent_qualified, graph);
            }
            "trait_item" => {
                extract_trait(child, source, file_path, parent_qualified, graph);
            }
            "impl_item" => {
                extract_impl(child, source, file_path, parent_qualified, graph);
            }
            "type_item" => {
                extract_type_alias(child, source, file_path, parent_qualified, graph);
            }
            "macro_definition" => {
                extract_macro(child, source, file_path, parent_qualified, graph);
            }
            "mod_item" => {
                extract_module(child, source, file_path, parent_qualified, graph);
            }
            "use_declaration" => {
                extract_use(child, source, file_path, parent_qualified, graph);
            }
            "const_item" => {
                extract_const(child, source, file_path, parent_qualified, graph, EntityKind::Constant);
            }
            "static_item" => {
                extract_const(child, source, file_path, parent_qualified, graph, EntityKind::Static);
            }
            _ => {
                // Recurse into other nodes
                extract_entities(child, source, file_path, parent_qualified, graph);
            }
        }
    }
}

fn get_node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn get_visibility(node: Node, source: &str) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = get_node_text(child, source);
            return match text {
                "pub" => Visibility::Public,
                "pub(crate)" => Visibility::PublicCrate,
                "pub(super)" => Visibility::PublicSuper,
                _ if text.starts_with("pub") => Visibility::Public,
                _ => Visibility::Private,
            };
        }
    }
    Visibility::Private
}

fn get_doc_comment(node: Node, source: &str) -> Option<String> {
    let mut comments = Vec::new();
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "line_comment" {
            let text = get_node_text(sib, source).trim();
            if text.starts_with("///") || text.starts_with("//!") {
                comments.push(text.trim_start_matches("///").trim_start_matches("//!").trim().to_string());
            } else {
                break;
            }
        } else if sib.kind() == "block_comment" {
            let text = get_node_text(sib, source).trim();
            if text.starts_with("/**") {
                comments.push(text.trim_start_matches("/**").trim_end_matches("*/").trim().to_string());
            }
            break;
        } else {
            break;
        }
        sibling = sib.prev_sibling();
    }
    comments.reverse();
    if comments.is_empty() {
        None
    } else {
        Some(comments.join("\n"))
    }
}

fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

fn make_entity(
    node: Node,
    source: &str,
    name: &str,
    kind: EntityKind,
    file_path: &str,
    parent_qualified: &str,
) -> CodeEntity {
    let source_text = get_node_text(node, source).to_string();
    let hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let qualified_name = format!("{parent_qualified}::{name}");

    CodeEntity {
        qualified_name,
        name: name.to_string(),
        kind,
        file_path: file_path.to_string(),
        module_path: Some(parent_qualified.to_string()),
        visibility: get_visibility(node, source),
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        doc_comment: get_doc_comment(node, source),
        source_text,
        source_hash: hash,
        signature: None,
        is_async: false,
        embedding: None,
    }
}

fn extract_function(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
    is_method: bool,
) {
    let name_node = match find_child_by_kind(node, "identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let kind = if is_method { EntityKind::Method } else { EntityKind::Function };

    let mut entity = make_entity(node, source, name, kind, file_path, parent_qualified);

    // Extract signature (everything before the block)
    if let Some(body) = find_child_by_kind(node, "block") {
        let sig_end = body.start_byte();
        entity.signature = Some(source[node.start_byte()..sig_end].trim().to_string());
    }

    // Check if async
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "async" {
            entity.is_async = true;
            break;
        }
    }

    let fn_qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    // CONTAINS relationship from parent
    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: fn_qualified.clone(),
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });

    // Extract CALLS relationships from function body
    if let Some(body) = find_child_by_kind(node, "block") {
        extract_calls(body, source, file_path, &fn_qualified, graph);
    }
}

/// Walk a node looking for call expressions and record CALLS relationships.
fn extract_calls(
    node: Node,
    source: &str,
    _file_path: &str,
    caller_qualified: &str,
    graph: &mut CodeGraph,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func) = find_child_by_kind(child, "identifier") {
                let callee = get_node_text(func, source);
                graph.add_relationship(Relationship {
                    from: caller_qualified.to_string(),
                    to: callee.to_string(), // Unresolved — will be resolved during graph storage
                    kind: RelationshipKind::Calls,
                    metadata: RelationshipMeta {
                        line: Some(child.start_position().row as u32 + 1),
                        ..Default::default()
                    },
                });
            } else if let Some(field) = find_child_by_kind(child, "field_expression") {
                // method call: obj.method()
                if let Some(method_name) = find_child_by_kind(field, "field_identifier") {
                    let callee = get_node_text(method_name, source);
                    graph.add_relationship(Relationship {
                        from: caller_qualified.to_string(),
                        to: callee.to_string(),
                        kind: RelationshipKind::Calls,
                        metadata: RelationshipMeta {
                            line: Some(child.start_position().row as u32 + 1),
                            ..Default::default()
                        },
                    });
                }
            } else if let Some(scoped) = find_child_by_kind(child, "scoped_identifier") {
                let callee = get_node_text(scoped, source);
                graph.add_relationship(Relationship {
                    from: caller_qualified.to_string(),
                    to: callee.to_string(),
                    kind: RelationshipKind::Calls,
                    metadata: RelationshipMeta {
                        line: Some(child.start_position().row as u32 + 1),
                        ..Default::default()
                    },
                });
            }
        }
        // Recurse
        extract_calls(child, source, _file_path, caller_qualified, graph);
    }
}

fn extract_struct(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name_node = match find_child_by_kind(node, "type_identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let entity = make_entity(node, source, name, EntityKind::Struct, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified.clone(),
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });

    // Extract fields
    if let Some(field_list) = find_child_by_kind(node, "field_declaration_list") {
        let mut cursor = field_list.walk();
        for field in field_list.children(&mut cursor) {
            if field.kind() == "field_declaration"
                && let Some(fname) = find_child_by_kind(field, "field_identifier") {
                    let field_name = get_node_text(fname, source).to_string();
                    let field_type = find_child_by_kind(field, "type_identifier")
                        .map(|t| get_node_text(t, source).to_string());

                    if let Some(ref ft) = field_type {
                        graph.add_relationship(Relationship {
                            from: qualified.clone(),
                            to: ft.clone(),
                            kind: RelationshipKind::HasField,
                            metadata: RelationshipMeta {
                                field_name: Some(field_name),
                                field_type,
                                ..Default::default()
                            },
                        });
                    }
                }
        }
    }
}

fn extract_enum(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name_node = match find_child_by_kind(node, "type_identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let entity = make_entity(node, source, name, EntityKind::Enum, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified.clone(),
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });

    // Extract variants
    if let Some(variant_list) = find_child_by_kind(node, "enum_variant_list") {
        let mut cursor = variant_list.walk();
        for variant in variant_list.children(&mut cursor) {
            if variant.kind() == "enum_variant"
                && let Some(vname) = find_child_by_kind(variant, "identifier") {
                    let variant_name = get_node_text(vname, source).to_string();
                    graph.add_relationship(Relationship {
                        from: qualified.clone(),
                        to: format!("{qualified}::{variant_name}"),
                        kind: RelationshipKind::HasVariant,
                        metadata: RelationshipMeta {
                            variant_name: Some(variant_name),
                            ..Default::default()
                        },
                    });
                }
        }
    }
}

fn extract_trait(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name_node = match find_child_by_kind(node, "type_identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let entity = make_entity(node, source, name, EntityKind::Trait, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified.clone(),
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });

    // Extract trait methods (in declaration_list)
    if let Some(body) = find_child_by_kind(node, "declaration_list") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_item" {
                extract_function(child, source, file_path, &qualified, graph, true);
            }
        }
    }

    // Extract supertraits from trait_bounds
    if let Some(bounds) = find_child_by_kind(node, "trait_bounds") {
        let mut cursor = bounds.walk();
        for child in bounds.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                let supertrait = get_node_text(child, source);
                graph.add_relationship(Relationship {
                    from: qualified.clone(),
                    to: supertrait.to_string(),
                    kind: RelationshipKind::Extends,
                    metadata: RelationshipMeta::default(),
                });
            }
        }
    }
}

fn extract_impl(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Use tree-sitter field names to extract the type and trait
    // `impl Type { ... }` -> type field = "Type", trait field = None
    // `impl Trait for Type { ... }` -> type field = "Type", trait field = "Trait"
    let impl_type = node
        .child_by_field_name("type")
        .and_then(|n| {
            if n.kind() == "type_identifier" {
                Some(get_node_text(n, source).to_string())
            } else {
                None
            }
        });

    let trait_name = node
        .child_by_field_name("trait")
        .and_then(|n| {
            if n.kind() == "type_identifier" {
                Some(get_node_text(n, source).to_string())
            } else {
                // Could be a scoped identifier like std::fmt::Display
                find_child_by_kind(n, "type_identifier")
                    .map(|inner| get_node_text(inner, source).to_string())
                    .or_else(|| Some(get_node_text(n, source).to_string()))
            }
        });

    let impl_name = match &impl_type {
        Some(t) => match &trait_name {
            Some(tr) => format!("impl {tr} for {t}"),
            None => format!("impl {t}"),
        },
        None => return,
    };

    let entity = make_entity(node, source, &impl_name, EntityKind::Impl, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified.clone(),
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });

    // IMPL_FOR relationship
    if let Some(ref target_type) = impl_type {
        graph.add_relationship(Relationship {
            from: qualified.clone(),
            to: target_type.clone(),
            kind: RelationshipKind::ImplFor,
            metadata: RelationshipMeta::default(),
        });
    }

    // IMPLEMENTS relationship (if trait impl)
    if let Some(ref tr) = trait_name {
        graph.add_relationship(Relationship {
            from: qualified.clone(),
            to: tr.clone(),
            kind: RelationshipKind::Implements,
            metadata: RelationshipMeta::default(),
        });
    }

    // Extract methods from impl body
    if let Some(body) = find_child_by_kind(node, "declaration_list") {
        let impl_parent = match &impl_type {
            Some(t) => format!("{parent_qualified}::{t}"),
            None => qualified.clone(),
        };

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_item" {
                extract_function(child, source, file_path, &impl_parent, graph, true);
            }
        }
    }
}

fn extract_type_alias(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name_node = match find_child_by_kind(node, "type_identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let entity = make_entity(node, source, name, EntityKind::TypeAlias, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified,
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });
}

fn extract_macro(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name_node = match find_child_by_kind(node, "identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let entity = make_entity(node, source, name, EntityKind::Macro, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified,
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });
}

fn extract_module(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name_node = match find_child_by_kind(node, "identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let entity = make_entity(node, source, name, EntityKind::Module, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified.clone(),
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });

    // If inline module (has declaration_list), recurse into it
    if let Some(body) = find_child_by_kind(node, "declaration_list") {
        extract_entities(body, source, file_path, &qualified, graph);
    }
}

fn extract_use(
    node: Node,
    source: &str,
    _file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Extract the full use path
    let use_text = get_node_text(node, source);
    let path = use_text
        .trim_start_matches("use ")
        .trim_end_matches(';')
        .trim()
        .to_string();

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: path,
        kind: RelationshipKind::Imports,
        metadata: RelationshipMeta {
            line: Some(node.start_position().row as u32 + 1),
            ..Default::default()
        },
    });
}

fn extract_const(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
    kind: EntityKind,
) {
    let name_node = match find_child_by_kind(node, "identifier") {
        Some(n) => n,
        None => return,
    };
    let name = get_node_text(name_node, source);
    let entity = make_entity(node, source, name, kind, file_path, parent_qualified);
    let qualified = entity.qualified_name.clone();
    graph.add_entity(entity);

    graph.add_relationship(Relationship {
        from: parent_qualified.to_string(),
        to: qualified,
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_function() {
        let parser = RustParser::new();
        let source = r#"
/// Does something cool.
pub fn hello(name: &str) -> String {
    format!("Hello, {name}!")
}
"#;
        let graph = parser
            .parse_file(Path::new("test.rs"), source)
            .unwrap();

        assert!(graph.entities.contains_key("test.rs::hello"));
        let entity = &graph.entities["test.rs::hello"];
        assert_eq!(entity.kind, EntityKind::Function);
        assert_eq!(entity.visibility, Visibility::Public);
        assert!(entity.doc_comment.is_some());
    }

    #[test]
    fn test_parse_struct_with_fields() {
        let parser = RustParser::new();
        let source = r#"
pub struct Config {
    pub name: String,
    pub value: i32,
}
"#;
        let graph = parser
            .parse_file(Path::new("test.rs"), source)
            .unwrap();

        assert!(graph.entities.contains_key("test.rs::Config"));
        let entity = &graph.entities["test.rs::Config"];
        assert_eq!(entity.kind, EntityKind::Struct);

        // Should have HasField relationships
        let field_rels: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::HasField)
            .collect();
        assert!(!field_rels.is_empty());
    }

    #[test]
    fn test_parse_impl_block() {
        let parser = RustParser::new();
        let source = r#"
struct Foo;

impl Foo {
    pub fn bar(&self) -> i32 {
        42
    }
}
"#;
        let graph = parser
            .parse_file(Path::new("test.rs"), source)
            .unwrap();

        // Method should be under Foo
        assert!(graph.entities.contains_key("test.rs::Foo::bar"));
        let method = &graph.entities["test.rs::Foo::bar"];
        assert_eq!(method.kind, EntityKind::Method);
    }

    #[test]
    fn test_parse_trait_impl() {
        let parser = RustParser::new();
        let source = r#"
trait Greet {
    fn greet(&self) -> String;
}

struct Person;

impl Greet for Person {
    fn greet(&self) -> String {
        "Hello".to_string()
    }
}
"#;
        let graph = parser
            .parse_file(Path::new("test.rs"), source)
            .unwrap();

        // Should have IMPLEMENTS relationship
        let impls: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Implements)
            .collect();
        assert!(!impls.is_empty());
    }

    #[test]
    fn test_parse_enum() {
        let parser = RustParser::new();
        let source = r#"
pub enum Color {
    Red,
    Green,
    Blue,
}
"#;
        let graph = parser
            .parse_file(Path::new("test.rs"), source)
            .unwrap();

        assert!(graph.entities.contains_key("test.rs::Color"));
        let variants: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::HasVariant)
            .collect();
        assert_eq!(variants.len(), 3);
    }
}
