use std::path::Path;

use codegraph_core::entity::{EntityKind, Visibility};
use codegraph_core::relationship::{Relationship, RelationshipKind, RelationshipMeta};
use codegraph_core::{CodeGraph, Result};
use tree_sitter::{Node, Parser};

use crate::helpers::*;
use crate::LanguageParser;

pub struct CParser {
    cpp: bool,
}

impl CParser {
    pub fn c() -> Self {
        Self { cpp: false }
    }

    pub fn cpp() -> Self {
        Self { cpp: true }
    }

    fn create_parser(&self) -> Result<Parser> {
        let mut parser = Parser::new();
        let lang = if self.cpp {
            tree_sitter_cpp::LANGUAGE.into()
        } else {
            tree_sitter_c::LANGUAGE.into()
        };
        parser
            .set_language(&lang)
            .map_err(|e| codegraph_core::Error::Parser(e.to_string()))?;
        Ok(parser)
    }
}

impl LanguageParser for CParser {
    fn extensions(&self) -> &[&str] {
        if self.cpp {
            &["cpp", "cxx", "cc", "hpp", "hxx", "hh"]
        } else {
            &["c", "h"]
        }
    }

    fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph> {
        let mut parser = self.create_parser()?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| codegraph_core::Error::Parser("Failed to parse C/C++ file".into()))?;

        let file_path = path.to_string_lossy().to_string();
        let mut graph = CodeGraph::new();

        graph.add_entity(make_file_entity(&file_path, path, source));
        extract_items(
            tree.root_node(),
            source,
            &file_path,
            &file_path,
            &mut graph,
            self.cpp,
        );

        Ok(graph)
    }
}

fn extract_items(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
    is_cpp: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                extract_function(child, source, file_path, parent_qualified, graph);
            }
            "declaration" => {
                extract_declaration(child, source, file_path, parent_qualified, graph);
            }
            "struct_specifier" => {
                extract_struct_or_class(
                    child,
                    source,
                    file_path,
                    parent_qualified,
                    graph,
                    EntityKind::Struct,
                );
            }
            "union_specifier" => {
                extract_struct_or_class(
                    child,
                    source,
                    file_path,
                    parent_qualified,
                    graph,
                    EntityKind::Union,
                );
            }
            "enum_specifier" => {
                extract_enum(child, source, file_path, parent_qualified, graph);
            }
            "type_definition" => {
                extract_typedef(child, source, file_path, parent_qualified, graph);
            }
            "preproc_include" => {
                extract_include(child, source, parent_qualified, graph);
            }
            "preproc_def" | "preproc_function_def" => {
                extract_preproc(child, source, file_path, parent_qualified, graph);
            }
            // C++ specific
            "class_specifier" if is_cpp => {
                extract_struct_or_class(
                    child,
                    source,
                    file_path,
                    parent_qualified,
                    graph,
                    EntityKind::Class,
                );
            }
            "namespace_definition" if is_cpp => {
                extract_namespace(child, source, file_path, parent_qualified, graph);
            }
            _ => {
                extract_items(child, source, file_path, parent_qualified, graph, is_cpp);
            }
        }
    }
}

fn extract_function(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = node
        .child_by_field_name("declarator")
        .and_then(|d| {
            // Could be a function_declarator wrapping an identifier
            if d.kind() == "function_declarator" {
                d.child_by_field_name("declarator")
            } else {
                Some(d)
            }
        })
        .map(|n| node_text(n, source))
        .unwrap_or_default();

    if name.is_empty() {
        return;
    }

    let mut entity = make_entity(
        node,
        source,
        name,
        EntityKind::Function,
        file_path,
        parent_qualified,
        Visibility::Public,
    );

    // Build signature from everything before the body
    if let Some(body) = child_by_kind(node, "compound_statement") {
        entity.signature = Some(
            source[node.start_byte()..body.start_byte()]
                .trim()
                .to_string(),
        );

        // Extract calls from body
        let fn_qn = entity.qualified_name.clone();
        graph.add_entity(entity);
        add_contains(graph, parent_qualified, &fn_qn);
        extract_calls_generic(
            body,
            source,
            &fn_qn,
            graph,
            &["call_expression"],
            &["identifier", "field_expression", "qualified_identifier"],
        );
    } else {
        let fn_qn = entity.qualified_name.clone();
        graph.add_entity(entity);
        add_contains(graph, parent_qualified, &fn_qn);
    }
}

fn extract_declaration(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Check if it's a function declaration (prototype)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_declarator" {
            if let Some(id) = child.child_by_field_name("declarator") {
                let name = node_text(id, source);
                let entity = make_entity(
                    node,
                    source,
                    name,
                    EntityKind::Function,
                    file_path,
                    parent_qualified,
                    Visibility::Public,
                );
                let qn = entity.qualified_name.clone();
                graph.add_entity(entity);
                add_contains(graph, parent_qualified, &qn);
            }
            return;
        }
    }

    // Otherwise it's a variable/constant declaration — extract if global scope
    if let Some(declarator) = node.child_by_field_name("declarator") {
        let id = match declarator.kind() {
            "init_declarator" => declarator.child_by_field_name("declarator"),
            "identifier" => Some(declarator),
            _ => None,
        };
        if let Some(id_node) = id
            && id_node.kind() == "identifier" {
                let name = node_text(id_node, source);
                let text = node_text(node, source);
                let kind = if text.contains("const ") {
                    EntityKind::Constant
                } else {
                    EntityKind::Variable
                };
                let entity = make_entity(
                    node,
                    source,
                    name,
                    kind,
                    file_path,
                    parent_qualified,
                    Visibility::Public,
                );
                let qn = entity.qualified_name.clone();
                graph.add_entity(entity);
                add_contains(graph, parent_qualified, &qn);
            }
    }
}

fn extract_struct_or_class(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
    kind: EntityKind,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source));

    let name = match name {
        Some(n) if !n.is_empty() => n,
        _ => return, // anonymous struct/union
    };

    let entity = make_entity(
        node,
        source,
        name,
        kind,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Extract base classes (C++ class inheritance)
    if let Some(bases) = child_by_kind(node, "base_class_clause") {
        let mut cursor = bases.walk();
        for base in bases.children(&mut cursor) {
            if base.kind() == "type_identifier" || base.kind() == "qualified_identifier" {
                add_extends(graph, &qn, node_text(base, source));
            }
        }
    }

    // Extract fields and methods from body
    if let Some(body) = child_by_kind(node, "field_declaration_list") {
        let mut body_cursor = body.walk();
        for member in body.children(&mut body_cursor) {
            match member.kind() {
                "function_definition" => {
                    extract_method(member, source, file_path, &qn, graph);
                }
                "field_declaration" => {
                    if let Some(decl) = member.child_by_field_name("declarator") {
                        if decl.kind() == "function_declarator" {
                            // Method declaration
                            if let Some(id) = decl.child_by_field_name("declarator") {
                                let method_name = node_text(id, source);
                                let method = make_entity(
                                    member,
                                    source,
                                    method_name,
                                    EntityKind::Method,
                                    file_path,
                                    &qn,
                                    Visibility::Public,
                                );
                                let mq = method.qualified_name.clone();
                                graph.add_entity(method);
                                add_contains(graph, &qn, &mq);
                            }
                        } else if decl.kind() == "field_identifier" {
                            let field_name = node_text(decl, source);
                            let field_type = member
                                .child_by_field_name("type")
                                .map(|t| node_text(t, source).to_string());
                            if let Some(ref ft) = field_type {
                                graph.add_relationship(Relationship {
                                    from: qn.clone(),
                                    to: ft.clone(),
                                    kind: RelationshipKind::HasField,
                                    metadata: RelationshipMeta {
                                        field_name: Some(field_name.to_string()),
                                        field_type,
                                        ..Default::default()
                                    },
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn extract_method(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = node
        .child_by_field_name("declarator")
        .and_then(|d| {
            if d.kind() == "function_declarator" {
                d.child_by_field_name("declarator")
            } else {
                Some(d)
            }
        })
        .map(|n| node_text(n, source))
        .unwrap_or_default();

    if name.is_empty() {
        return;
    }

    let mut entity = make_entity(
        node,
        source,
        name,
        EntityKind::Method,
        file_path,
        parent_qualified,
        Visibility::Public,
    );

    if let Some(body) = child_by_kind(node, "compound_statement") {
        entity.signature = Some(
            source[node.start_byte()..body.start_byte()]
                .trim()
                .to_string(),
        );
        let mq = entity.qualified_name.clone();
        graph.add_entity(entity);
        add_contains(graph, parent_qualified, &mq);
        extract_calls_generic(
            body,
            source,
            &mq,
            graph,
            &["call_expression"],
            &["identifier", "field_expression", "qualified_identifier"],
        );
    } else {
        let mq = entity.qualified_name.clone();
        graph.add_entity(entity);
        add_contains(graph, parent_qualified, &mq);
    }
}

fn extract_enum(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source));

    let name = match name {
        Some(n) if !n.is_empty() => n,
        _ => return,
    };

    let entity = make_entity(
        node,
        source,
        name,
        EntityKind::Enum,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Extract enumerators
    if let Some(body) = child_by_kind(node, "enumerator_list") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "enumerator"
                && let Some(id) = child.child_by_field_name("name") {
                    let variant_name = node_text(id, source).to_string();
                    graph.add_relationship(Relationship {
                        from: qn.clone(),
                        to: format!("{qn}::{variant_name}"),
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

fn extract_typedef(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = node
        .child_by_field_name("declarator")
        .and_then(|d| {
            if d.kind() == "type_identifier" {
                Some(d)
            } else {
                child_by_kind(d, "type_identifier")
            }
        })
        .map(|n| node_text(n, source));

    if let Some(name) = name {
        let entity = make_entity(
            node,
            source,
            name,
            EntityKind::Typedef,
            file_path,
            parent_qualified,
            Visibility::Public,
        );
        let qn = entity.qualified_name.clone();
        graph.add_entity(entity);
        add_contains(graph, parent_qualified, &qn);
    }
}

fn extract_include(node: Node, source: &str, parent_qualified: &str, graph: &mut CodeGraph) {
    if let Some(path_node) =
        child_by_kind(node, "string_literal").or_else(|| child_by_kind(node, "system_lib_string"))
    {
        let path = node_text(path_node, source);
        add_import(
            graph,
            parent_qualified,
            path,
            node.start_position().row as u32 + 1,
        );
    }
}

fn extract_preproc(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source));

    if let Some(name) = name {
        let entity = make_entity(
            node,
            source,
            name,
            EntityKind::Preprocessor,
            file_path,
            parent_qualified,
            Visibility::Public,
        );
        let qn = entity.qualified_name.clone();
        graph.add_entity(entity);
        add_contains(graph, parent_qualified, &qn);
    }
}

fn extract_namespace(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source))
        .unwrap_or("anonymous");

    let entity = make_entity(
        node,
        source,
        name,
        EntityKind::Namespace,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Recurse into namespace body
    if let Some(body) = child_by_kind(node, "declaration_list") {
        extract_items(body, source, file_path, &qn, graph, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_c_function() {
        let parser = CParser::c();
        let source = r#"
int add(int a, int b) {
    return a + b;
}
"#;
        let graph = parser.parse_file(Path::new("test.c"), source).unwrap();
        assert!(graph.entities.contains_key("test.c::add"));
        let entity = &graph.entities["test.c::add"];
        assert_eq!(entity.kind, EntityKind::Function);
    }

    #[test]
    fn test_parse_c_struct() {
        let parser = CParser::c();
        let source = r#"
struct Point {
    int x;
    int y;
};
"#;
        let graph = parser.parse_file(Path::new("test.c"), source).unwrap();
        assert!(graph.entities.contains_key("test.c::Point"));
    }

    #[test]
    fn test_parse_cpp_class() {
        let parser = CParser::cpp();
        let source = r#"
class Animal {
public:
    virtual void speak() {
    }
};

class Dog : public Animal {
public:
    void speak() {
    }
};
"#;
        let graph = parser.parse_file(Path::new("test.cpp"), source).unwrap();
        assert!(graph.entities.contains_key("test.cpp::Animal"));
        assert!(graph.entities.contains_key("test.cpp::Dog"));

        let extends: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Extends)
            .collect();
        assert!(!extends.is_empty());
    }

    #[test]
    fn test_parse_c_enum() {
        let parser = CParser::c();
        let source = r#"
enum Color {
    RED,
    GREEN,
    BLUE
};
"#;
        let graph = parser.parse_file(Path::new("test.c"), source).unwrap();
        assert!(graph.entities.contains_key("test.c::Color"));
        let variants: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::HasVariant)
            .collect();
        assert_eq!(variants.len(), 3);
    }

    #[test]
    fn test_parse_cpp_namespace() {
        let parser = CParser::cpp();
        let source = r#"
namespace math {
    int add(int a, int b) {
        return a + b;
    }
}
"#;
        let graph = parser.parse_file(Path::new("test.cpp"), source).unwrap();
        assert!(graph.entities.contains_key("test.cpp::math"));
        assert!(graph.entities.contains_key("test.cpp::math::add"));
    }
}
