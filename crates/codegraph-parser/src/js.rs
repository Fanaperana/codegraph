use std::path::Path;

use codegraph_core::entity::{EntityKind, Visibility};
use codegraph_core::relationship::{RelationshipKind, RelationshipMeta, Relationship};
use codegraph_core::{CodeGraph, Result};
use tree_sitter::{Node, Parser};

use crate::helpers::*;
use crate::LanguageParser;

/// Parser for JavaScript, TypeScript, JSX, and TSX.
pub struct JsParser {
    variant: JsVariant,
}

enum JsVariant {
    JavaScript,
    TypeScript,
    Tsx,
}

impl JsParser {
    pub fn javascript() -> Self {
        Self { variant: JsVariant::JavaScript }
    }

    pub fn typescript() -> Self {
        Self { variant: JsVariant::TypeScript }
    }

    pub fn tsx() -> Self {
        Self { variant: JsVariant::Tsx }
    }

    fn create_parser(&self) -> Result<Parser> {
        let mut parser = Parser::new();
        let lang = match self.variant {
            JsVariant::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            JsVariant::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            JsVariant::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        };
        parser
            .set_language(&lang)
            .map_err(|e| codegraph_core::Error::Parser(e.to_string()))?;
        Ok(parser)
    }
}

impl LanguageParser for JsParser {
    fn extensions(&self) -> &[&str] {
        match self.variant {
            JsVariant::JavaScript => &["js", "mjs", "cjs", "jsx"],
            JsVariant::TypeScript => &["ts", "mts", "cts"],
            JsVariant::Tsx => &["tsx"],
        }
    }

    fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph> {
        let mut parser = self.create_parser()?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| codegraph_core::Error::Parser("Failed to parse JS/TS file".into()))?;

        let file_path = path.to_string_lossy().to_string();
        let mut graph = CodeGraph::new();

        graph.add_entity(make_file_entity(&file_path, path, source));
        extract_items(tree.root_node(), source, &file_path, &file_path, &mut graph);

        Ok(graph)
    }
}

fn extract_items(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                extract_function(child, source, file_path, parent_qualified, graph);
            }
            "class_declaration" => {
                extract_class(child, source, file_path, parent_qualified, graph);
            }
            "lexical_declaration" | "variable_declaration" => {
                extract_variable_decl(child, source, file_path, parent_qualified, graph);
            }
            "import_statement" => {
                extract_import_stmt(child, source, parent_qualified, graph);
            }
            "export_statement" => {
                // Recurse into exports to find the actual declaration
                extract_export(child, source, file_path, parent_qualified, graph);
            }
            // TypeScript-specific
            "interface_declaration" => {
                extract_interface(child, source, file_path, parent_qualified, graph);
            }
            "type_alias_declaration" => {
                extract_type_alias(child, source, file_path, parent_qualified, graph);
            }
            "enum_declaration" => {
                extract_enum(child, source, file_path, parent_qualified, graph);
            }
            _ => {
                extract_items(child, source, file_path, parent_qualified, graph);
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
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => return,
    };

    let mut entity = make_entity(
        node, source, name, EntityKind::Function, file_path, parent_qualified, Visibility::Public,
    );

    // Check async
    let text = node_text(node, source);
    if text.starts_with("async ") {
        entity.is_async = true;
    }

    // Build signature
    if let Some(params) = node.child_by_field_name("parameters") {
        let return_type = node
            .child_by_field_name("return_type")
            .map(|r| format!(": {}", node_text(r, source)));
        entity.signature = Some(format!(
            "function {name}{}{}",
            node_text(params, source),
            return_type.unwrap_or_default()
        ));
    }

    let fn_qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &fn_qn);

    // Extract calls from body
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_generic(
            body, source, &fn_qn, graph,
            &["call_expression"],
            &["identifier", "member_expression"],
        );
    }
}

fn extract_class(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => return,
    };

    let entity = make_entity(
        node, source, name, EntityKind::Class, file_path, parent_qualified, Visibility::Public,
    );
    let class_qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &class_qn);

    // Extract superclass
    if let Some(heritage) = child_by_kind(node, "class_heritage") {
        let mut cursor = heritage.walk();
        for child in heritage.children(&mut cursor) {
            if child.kind() == "identifier" {
                add_extends(graph, &class_qn, node_text(child, source));
            }
        }
    }

    // Extract implements (TypeScript)
    // This appears in the node as "implements_clause" in TS grammar

    // Extract class body
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "method_definition" => {
                    extract_method(child, source, file_path, &class_qn, graph);
                }
                "public_field_definition" | "field_definition" => {
                    extract_property(child, source, file_path, &class_qn, graph);
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
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => return,
    };

    let kind = if name == "constructor" {
        EntityKind::Constructor
    } else {
        EntityKind::Method
    };

    let mut entity = make_entity(
        node, source, name, kind, file_path, parent_qualified, Visibility::Public,
    );

    let text = node_text(node, source);
    if text.starts_with("async ") || text.contains("async ") {
        entity.is_async = true;
    }

    // Check for static/private
    if text.starts_with("static ") {
        entity.visibility = Visibility::Public;
    }
    if name.starts_with('#') {
        entity.visibility = Visibility::Private;
    }

    // Build signature
    if let Some(params) = node.child_by_field_name("parameters") {
        entity.signature = Some(format!("{name}{}", node_text(params, source)));
    }

    let mq = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &mq);

    // Extract calls
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_generic(
            body, source, &mq, graph,
            &["call_expression"],
            &["identifier", "member_expression"],
        );
    }
}

fn extract_property(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => return,
    };

    let entity = make_entity(
        node, source, name, EntityKind::Property, file_path, parent_qualified, Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);
}

fn extract_variable_decl(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = match child.child_by_field_name("name") {
                Some(n) if n.kind() == "identifier" => node_text(n, source),
                _ => continue,
            };

            // Check if it's an arrow function or function expression
            let value = child.child_by_field_name("value");
            let kind = match value.map(|v| v.kind()) {
                Some("arrow_function") | Some("function") => EntityKind::Function,
                _ => {
                    let decl_text = node_text(node, source);
                    if decl_text.starts_with("const ") {
                        EntityKind::Constant
                    } else {
                        EntityKind::Variable
                    }
                }
            };

            let mut entity = make_entity(
                child, source, name, kind, file_path, parent_qualified, Visibility::Public,
            );

            // If arrow function, check async and extract signature
            if let Some(val) = value
                && (val.kind() == "arrow_function" || val.kind() == "function") {
                    let val_text = node_text(val, source);
                    if val_text.starts_with("async ") {
                        entity.is_async = true;
                    }
                    if let Some(params) = val.child_by_field_name("parameters") {
                        entity.signature =
                            Some(format!("const {name} = {}", node_text(params, source)));
                    }

                    let fn_qn = entity.qualified_name.clone();
                    graph.add_entity(entity);
                    add_contains(graph, parent_qualified, &fn_qn);

                    // Extract calls from arrow function body
                    if let Some(body) = val.child_by_field_name("body") {
                        extract_calls_generic(
                            body, source, &fn_qn, graph,
                            &["call_expression"],
                            &["identifier", "member_expression"],
                        );
                    }
                    continue;
                }

            let qn = entity.qualified_name.clone();
            graph.add_entity(entity);
            add_contains(graph, parent_qualified, &qn);
        }
    }
}

fn extract_import_stmt(
    node: Node,
    source: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let line = node.start_position().row as u32 + 1;

    // Find the module source string
    if let Some(source_node) = node.child_by_field_name("source") {
        let module_path = node_text(source_node, source)
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string();
        add_import(graph, parent_qualified, &module_path, line);
    }
}

fn extract_export(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Recurse to find the actual declaration inside export
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                extract_function(child, source, file_path, parent_qualified, graph);
            }
            "class_declaration" => {
                extract_class(child, source, file_path, parent_qualified, graph);
            }
            "lexical_declaration" | "variable_declaration" => {
                extract_variable_decl(child, source, file_path, parent_qualified, graph);
            }
            "interface_declaration" => {
                extract_interface(child, source, file_path, parent_qualified, graph);
            }
            "type_alias_declaration" => {
                extract_type_alias(child, source, file_path, parent_qualified, graph);
            }
            "enum_declaration" => {
                extract_enum(child, source, file_path, parent_qualified, graph);
            }
            _ => {}
        }
    }
}

fn extract_interface(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => return,
    };

    let entity = make_entity(
        node, source, name, EntityKind::Interface, file_path, parent_qualified, Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Extract extends
    if let Some(heritage) = child_by_kind(node, "extends_type_clause") {
        let mut cursor = heritage.walk();
        for child in heritage.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                add_extends(graph, &qn, node_text(child, source));
            }
        }
    }

    // Extract method signatures from body
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if (child.kind() == "method_signature" || child.kind() == "property_signature")
                && let Some(name_node) = child.child_by_field_name("name") {
                    let method_name = node_text(name_node, source);
                    let mk = if child.kind() == "method_signature" {
                        EntityKind::Method
                    } else {
                        EntityKind::Property
                    };
                    let method = make_entity(
                        child, source, method_name, mk, file_path, &qn, Visibility::Public,
                    );
                    let mq = method.qualified_name.clone();
                    graph.add_entity(method);
                    add_contains(graph, &qn, &mq);
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
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => return,
    };

    let entity = make_entity(
        node, source, name, EntityKind::TypeAlias, file_path, parent_qualified, Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);
}

fn extract_enum(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => return,
    };

    let entity = make_entity(
        node, source, name, EntityKind::Enum, file_path, parent_qualified, Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Extract enum members
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "enum_assignment" || child.kind() == "property_identifier" {
                let variant_name = if child.kind() == "enum_assignment" {
                    child.child_by_field_name("name")
                        .map(|n| node_text(n, source).to_string())
                } else {
                    Some(node_text(child, source).to_string())
                };
                if let Some(vn) = variant_name {
                    graph.add_relationship(Relationship {
                        from: qn.clone(),
                        to: format!("{qn}::{vn}"),
                        kind: RelationshipKind::HasVariant,
                        metadata: RelationshipMeta {
                            variant_name: Some(vn),
                            ..Default::default()
                        },
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_js_function() {
        let parser = JsParser::javascript();
        let source = r#"
function greet(name) {
    return `Hello, ${name}!`;
}
"#;
        let graph = parser.parse_file(Path::new("test.js"), source).unwrap();
        assert!(graph.entities.contains_key("test.js::greet"));
        assert_eq!(graph.entities["test.js::greet"].kind, EntityKind::Function);
    }

    #[test]
    fn test_parse_js_arrow_function() {
        let parser = JsParser::javascript();
        let source = r#"
const add = (a, b) => a + b;
const fetchData = async (url) => {
    return fetch(url);
};
"#;
        let graph = parser.parse_file(Path::new("test.js"), source).unwrap();
        assert!(graph.entities.contains_key("test.js::add"));
        assert_eq!(graph.entities["test.js::add"].kind, EntityKind::Function);
        assert!(graph.entities.contains_key("test.js::fetchData"));
        assert!(graph.entities["test.js::fetchData"].is_async);
    }

    #[test]
    fn test_parse_js_class() {
        let parser = JsParser::javascript();
        let source = r#"
class Animal {
    constructor(name) {
        this.name = name;
    }
    speak() {
        return "...";
    }
}

class Dog extends Animal {
    speak() {
        return "Woof!";
    }
}
"#;
        let graph = parser.parse_file(Path::new("test.js"), source).unwrap();
        assert!(graph.entities.contains_key("test.js::Animal"));
        assert!(graph.entities.contains_key("test.js::Dog"));
        assert!(graph.entities.contains_key("test.js::Animal::constructor"));
        assert_eq!(
            graph.entities["test.js::Animal::constructor"].kind,
            EntityKind::Constructor
        );

        let extends: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Extends)
            .collect();
        assert!(!extends.is_empty());
    }

    #[test]
    fn test_parse_ts_interface() {
        let parser = JsParser::typescript();
        let source = r#"
interface Shape {
    area(): number;
    perimeter(): number;
}

interface Circle extends Shape {
    radius: number;
}
"#;
        let graph = parser.parse_file(Path::new("test.ts"), source).unwrap();
        assert!(graph.entities.contains_key("test.ts::Shape"));
        assert_eq!(graph.entities["test.ts::Shape"].kind, EntityKind::Interface);
        assert!(graph.entities.contains_key("test.ts::Circle"));
    }

    #[test]
    fn test_parse_js_imports() {
        let parser = JsParser::javascript();
        let source = r#"
import { useState, useEffect } from 'react';
import express from 'express';
"#;
        let graph = parser.parse_file(Path::new("test.js"), source).unwrap();
        let imports: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert!(imports.len() >= 2);
    }

    #[test]
    fn test_parse_ts_enum() {
        let parser = JsParser::typescript();
        let source = r#"
enum Direction {
    Up,
    Down,
    Left,
    Right
}
"#;
        let graph = parser.parse_file(Path::new("test.ts"), source).unwrap();
        assert!(graph.entities.contains_key("test.ts::Direction"));
        assert_eq!(graph.entities["test.ts::Direction"].kind, EntityKind::Enum);
    }
}
