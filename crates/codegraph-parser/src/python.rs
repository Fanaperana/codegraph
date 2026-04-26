use std::path::Path;

use codegraph_core::entity::{EntityKind, Visibility};
use codegraph_core::relationship::{Relationship, RelationshipKind, RelationshipMeta};
use codegraph_core::{CodeGraph, Result};
use tree_sitter::{Node, Parser};

use crate::helpers::*;
use crate::LanguageParser;

pub struct PythonParser;

impl PythonParser {
    pub fn new() -> Self {
        Self
    }

    fn create_parser() -> Result<Parser> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|e| codegraph_core::Error::Parser(e.to_string()))?;
        Ok(parser)
    }
}

impl Default for PythonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for PythonParser {
    fn extensions(&self) -> &[&str] {
        &["py", "pyi"]
    }

    fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph> {
        let mut parser = Self::create_parser()?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| codegraph_core::Error::Parser("Failed to parse Python file".into()))?;

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
            "function_definition" => {
                extract_function(child, source, file_path, parent_qualified, graph);
            }
            "class_definition" => {
                extract_class(child, source, file_path, parent_qualified, graph);
            }
            "import_statement" | "import_from_statement" => {
                extract_import_stmt(child, source, parent_qualified, graph);
            }
            "decorated_definition" => {
                // Extract the actual definition inside the decorator
                extract_decorated(child, source, file_path, parent_qualified, graph);
            }
            "assignment" => {
                // Module-level assignments are variables/constants
                extract_assignment(child, source, file_path, parent_qualified, graph);
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

    let is_method = name == "__init__"
        || parent_qualified.contains("::")
            && !parent_qualified.ends_with(&Path::new(file_path).to_string_lossy().to_string());

    let kind = if name == "__init__" {
        EntityKind::Constructor
    } else if is_method {
        EntityKind::Method
    } else {
        EntityKind::Function
    };

    let mut entity = make_entity(
        node,
        source,
        name,
        kind,
        file_path,
        parent_qualified,
        Visibility::Public,
    );

    // Check for async
    let full_text = node_text(node, source);
    if full_text.starts_with("async ") {
        entity.is_async = true;
    }

    // Check visibility from naming convention
    if name.starts_with("__") && !name.ends_with("__") {
        entity.visibility = Visibility::Private;
    } else if name.starts_with('_') {
        entity.visibility = Visibility::PublicCrate; // "protected" in Python convention
    }

    // Build signature
    if let Some(params) = node.child_by_field_name("parameters") {
        let return_type = node
            .child_by_field_name("return_type")
            .map(|r| node_text(r, source));

        let sig = match return_type {
            Some(rt) => format!("def {name}{} -> {rt}", node_text(params, source)),
            None => format!("def {name}{}", node_text(params, source)),
        };
        entity.signature = Some(sig);
    }

    // Extract docstring
    if let Some(body) = node.child_by_field_name("body")
        && let Some(first_stmt) = child_by_kind(body, "expression_statement")
            && let Some(string_node) = child_by_kind(first_stmt, "string") {
                entity.doc_comment = Some(node_text(string_node, source).to_string());
            }

    let fn_qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &fn_qn);

    // Extract calls from function body
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_generic(
            body,
            source,
            &fn_qn,
            graph,
            &["call"],
            &["identifier", "attribute"],
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
        node,
        source,
        name,
        EntityKind::Class,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let class_qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &class_qn);

    // Extract base classes (inheritance)
    if let Some(bases) = node.child_by_field_name("superclasses") {
        let mut cursor = bases.walk();
        for child in bases.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    add_extends(graph, &class_qn, node_text(child, source));
                }
                "attribute" => {
                    add_extends(graph, &class_qn, node_text(child, source));
                }
                _ => {}
            }
        }
    }

    // Extract class body
    if let Some(body) = node.child_by_field_name("body") {
        extract_items(body, source, file_path, &class_qn, graph);
    }
}

fn extract_import_stmt(node: Node, source: &str, parent_qualified: &str, graph: &mut CodeGraph) {
    let import_text = node_text(node, source);
    let line = node.start_position().row as u32 + 1;

    // Extract module names from various import patterns
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                add_import(graph, parent_qualified, node_text(child, source), line);
            }
            "aliased_import" => {
                if let Some(name) = child.child_by_field_name("name") {
                    add_import(graph, parent_qualified, node_text(name, source), line);
                }
            }
            _ => {}
        }
    }

    // Fallback: if no structured imports found, use the full text
    if graph
        .relationships
        .iter()
        .all(|r| !(r.from == parent_qualified && r.kind == RelationshipKind::Imports))
        || import_text.starts_with("from ")
    {
        // Already handled via dotted_name above in most cases
    }
}

fn extract_decorated(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Extract decorators
    let mut decorators = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let deco_text = node_text(child, source).trim_start_matches('@').to_string();
            decorators.push(deco_text);
        }
    }

    // Extract the definition itself
    if let Some(definition) = child_by_kind(node, "function_definition") {
        extract_function(definition, source, file_path, parent_qualified, graph);

        // Attach decorators as relationships
        if let Some(name) = definition.child_by_field_name("name") {
            let fn_qn = format!("{parent_qualified}::{}", node_text(name, source));
            for deco in &decorators {
                graph.add_relationship(Relationship {
                    from: fn_qn.clone(),
                    to: deco.clone(),
                    kind: RelationshipKind::UsesType,
                    metadata: RelationshipMeta::default(),
                });
            }
        }
    } else if let Some(definition) = child_by_kind(node, "class_definition") {
        extract_class(definition, source, file_path, parent_qualified, graph);
    }
}

fn extract_assignment(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    if let Some(left) = node.child_by_field_name("left")
        && left.kind() == "identifier" {
            let name = node_text(left, source);
            // Convention: UPPER_CASE = constant
            let kind = if name.chars().all(|c| c.is_uppercase() || c == '_') {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_python_function() {
        let parser = PythonParser::new();
        let source = r#"
def greet(name: str) -> str:
    """Say hello."""
    return f"Hello, {name}!"
"#;
        let graph = parser.parse_file(Path::new("test.py"), source).unwrap();
        assert!(graph.entities.contains_key("test.py::greet"));
        let entity = &graph.entities["test.py::greet"];
        assert_eq!(entity.kind, EntityKind::Function);
        assert!(entity.signature.is_some());
    }

    #[test]
    fn test_parse_python_class() {
        let parser = PythonParser::new();
        let source = r#"
class Animal:
    def __init__(self, name: str):
        self.name = name

    def speak(self) -> str:
        return "..."

class Dog(Animal):
    def speak(self) -> str:
        return "Woof!"
"#;
        let graph = parser.parse_file(Path::new("test.py"), source).unwrap();
        assert!(graph.entities.contains_key("test.py::Animal"));
        assert!(graph.entities.contains_key("test.py::Dog"));
        assert!(graph.entities.contains_key("test.py::Animal::__init__"));
        assert!(graph.entities.contains_key("test.py::Animal::speak"));

        let extends: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Extends)
            .collect();
        assert!(!extends.is_empty());
        assert_eq!(extends[0].to, "Animal");
    }

    #[test]
    fn test_parse_python_imports() {
        let parser = PythonParser::new();
        let source = r#"
import os
from pathlib import Path
from typing import List, Optional
"#;
        let graph = parser.parse_file(Path::new("test.py"), source).unwrap();
        let imports: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert!(!imports.is_empty());
    }

    #[test]
    fn test_parse_python_async() {
        let parser = PythonParser::new();
        let source = r#"
async def fetch_data(url: str) -> dict:
    pass
"#;
        let graph = parser.parse_file(Path::new("test.py"), source).unwrap();
        let entity = &graph.entities["test.py::fetch_data"];
        assert!(entity.is_async);
    }

    #[test]
    fn test_parse_python_constants() {
        let parser = PythonParser::new();
        let source = r#"
MAX_RETRIES = 3
DEFAULT_NAME = "world"
"#;
        let graph = parser.parse_file(Path::new("test.py"), source).unwrap();
        assert!(graph.entities.contains_key("test.py::MAX_RETRIES"));
        assert_eq!(
            graph.entities["test.py::MAX_RETRIES"].kind,
            EntityKind::Constant
        );
    }
}
