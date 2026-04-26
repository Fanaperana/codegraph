//! Shared helper functions used across all language parsers.

use codegraph_core::entity::{CodeEntity, EntityKind, Visibility};
use codegraph_core::relationship::{Relationship, RelationshipKind, RelationshipMeta};
use codegraph_core::CodeGraph;
use tree_sitter::Node;

/// Extract the text content of a tree-sitter node from source.
pub fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Find the first direct child with the given kind.
pub fn child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

/// Create a file entity for the root of a parsed file.
pub fn make_file_entity(file_path: &str, path: &std::path::Path, source: &str) -> CodeEntity {
    let file_hash = blake3::hash(source.as_bytes()).to_hex().to_string();
    CodeEntity {
        qualified_name: file_path.to_string(),
        name: path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default(),
        kind: EntityKind::File,
        file_path: file_path.to_string(),
        module_path: None,
        visibility: Visibility::Public,
        line_start: 1,
        line_end: source.lines().count() as u32,
        doc_comment: None,
        source_text: String::new(),
        source_hash: file_hash,
        signature: None,
        is_async: false,
        embedding: None,
    }
}

/// Create a generic code entity from a tree-sitter node.
pub fn make_entity(
    node: Node,
    source: &str,
    name: &str,
    kind: EntityKind,
    file_path: &str,
    parent_qualified: &str,
    visibility: Visibility,
) -> CodeEntity {
    let source_text = node_text(node, source).to_string();
    let hash = blake3::hash(source_text.as_bytes()).to_hex().to_string();
    let qualified_name = format!("{parent_qualified}::{name}");

    CodeEntity {
        qualified_name,
        name: name.to_string(),
        kind,
        file_path: file_path.to_string(),
        module_path: Some(parent_qualified.to_string()),
        visibility,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        doc_comment: None,
        source_text,
        source_hash: hash,
        signature: None,
        is_async: false,
        embedding: None,
    }
}

/// Add a CONTAINS relationship from parent to child.
pub fn add_contains(graph: &mut CodeGraph, parent: &str, child: &str) {
    graph.add_relationship(Relationship {
        from: parent.to_string(),
        to: child.to_string(),
        kind: RelationshipKind::Contains,
        metadata: RelationshipMeta::default(),
    });
}

/// Add a CALLS relationship.
pub fn add_call(graph: &mut CodeGraph, caller: &str, callee: &str, line: u32) {
    graph.add_relationship(Relationship {
        from: caller.to_string(),
        to: callee.to_string(),
        kind: RelationshipKind::Calls,
        metadata: RelationshipMeta {
            line: Some(line),
            ..Default::default()
        },
    });
}

/// Add an IMPORTS relationship.
pub fn add_import(graph: &mut CodeGraph, importer: &str, imported: &str, line: u32) {
    graph.add_relationship(Relationship {
        from: importer.to_string(),
        to: imported.to_string(),
        kind: RelationshipKind::Imports,
        metadata: RelationshipMeta {
            line: Some(line),
            ..Default::default()
        },
    });
}

/// Add an EXTENDS (inheritance) relationship.
pub fn add_extends(graph: &mut CodeGraph, child: &str, parent: &str) {
    graph.add_relationship(Relationship {
        from: child.to_string(),
        to: parent.to_string(),
        kind: RelationshipKind::Extends,
        metadata: RelationshipMeta::default(),
    });
}

/// Add an IMPLEMENTS relationship.
pub fn add_implements(graph: &mut CodeGraph, implementor: &str, interface: &str) {
    graph.add_relationship(Relationship {
        from: implementor.to_string(),
        to: interface.to_string(),
        kind: RelationshipKind::Implements,
        metadata: RelationshipMeta::default(),
    });
}

/// Recursively find all call expressions within a node and record CALLS relationships.
pub fn extract_calls_generic(
    node: Node,
    source: &str,
    caller_qualified: &str,
    graph: &mut CodeGraph,
    call_kinds: &[&str],
    ident_kinds: &[&str],
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if call_kinds.contains(&child.kind()) {
            // Try to find the function name being called
            for ident_kind in ident_kinds {
                if let Some(id) = child_by_kind(child, ident_kind) {
                    let callee = node_text(id, source);
                    add_call(
                        graph,
                        caller_qualified,
                        callee,
                        child.start_position().row as u32 + 1,
                    );
                    break;
                }
            }
        }
        extract_calls_generic(
            child,
            source,
            caller_qualified,
            graph,
            call_kinds,
            ident_kinds,
        );
    }
}
