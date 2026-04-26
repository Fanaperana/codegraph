use std::path::Path;

use codegraph_core::entity::{EntityKind, Visibility};
use codegraph_core::{CodeGraph, Result};
use tree_sitter::{Node, Parser};

use crate::helpers::*;
use crate::LanguageParser;

pub struct CssParser;

impl CssParser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CssParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for CssParser {
    fn extensions(&self) -> &[&str] {
        &["css"]
    }

    fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_css::LANGUAGE.into())
            .map_err(|e| codegraph_core::Error::Parser(e.to_string()))?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| codegraph_core::Error::Parser("Failed to parse CSS file".into()))?;

        let file_path = path.to_string_lossy().to_string();
        let mut graph = CodeGraph::new();

        graph.add_entity(make_file_entity(&file_path, path, source));
        extract_rules(tree.root_node(), source, &file_path, &file_path, &mut graph);

        Ok(graph)
    }
}

fn extract_rules(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "rule_set" => {
                extract_rule_set(child, source, file_path, parent_qualified, graph);
            }
            "import_statement" => {
                extract_css_import(child, source, parent_qualified, graph);
            }
            "media_statement" => {
                extract_media_rule(child, source, file_path, parent_qualified, graph);
            }
            "keyframes_statement" => {
                extract_keyframes(child, source, file_path, parent_qualified, graph);
            }
            _ => {
                extract_rules(child, source, file_path, parent_qualified, graph);
            }
        }
    }
}

fn extract_rule_set(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Get the selector(s)
    if let Some(selectors) = child_by_kind(node, "selectors") {
        let selector_text = node_text(selectors, source);
        let _line = node.start_position().row + 1;

        let entity = make_entity(
            node,
            source,
            selector_text,
            EntityKind::CssRule,
            file_path,
            parent_qualified,
            Visibility::Public,
        );
        let qn = entity.qualified_name.clone();
        graph.add_entity(entity);
        add_contains(graph, parent_qualified, &qn);

        // Extract individual selectors as CssSelector entities
        let mut cursor = selectors.walk();
        for sel in selectors.children(&mut cursor) {
            if sel.kind() != "," {
                let sel_text = node_text(sel, source).trim().to_string();
                if !sel_text.is_empty() && sel_text != selector_text {
                    let sel_entity = make_entity(
                        sel,
                        source,
                        &sel_text,
                        EntityKind::CssSelector,
                        file_path,
                        &qn,
                        Visibility::Public,
                    );
                    let sq = sel_entity.qualified_name.clone();
                    graph.add_entity(sel_entity);
                    add_contains(graph, &qn, &sq);
                }
            }
        }
    }
}

fn extract_css_import(node: Node, source: &str, parent_qualified: &str, graph: &mut CodeGraph) {
    let line = node.start_position().row as u32 + 1;
    // Extract the URL or string from @import
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_value" | "call_expression" => {
                let import_path = node_text(child, source)
                    .trim_matches(|c| c == '"' || c == '\'' || c == ' ')
                    .to_string();
                // Strip url() wrapper if present
                let clean = if import_path.starts_with("url(") && import_path.ends_with(')') {
                    import_path[4..import_path.len() - 1]
                        .trim_matches(|c| c == '"' || c == '\'')
                        .to_string()
                } else {
                    import_path
                };
                if !clean.is_empty() {
                    add_import(graph, parent_qualified, &clean, line);
                }
            }
            _ => {}
        }
    }
}

fn extract_media_rule(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Create an entity for the @media rule
    let line = node.start_position().row + 1;
    let media_name = format!("@media@L{line}");

    let entity = make_entity(
        node,
        source,
        &media_name,
        EntityKind::CssRule,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Extract nested rules
    if let Some(body) = child_by_kind(node, "block") {
        extract_rules(body, source, file_path, &qn, graph);
    }
}

fn extract_keyframes(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, source),
        None => {
            // Try to find keyframes_name child
            let mut cursor = node.walk();
            let mut found = None;
            for child in node.children(&mut cursor) {
                if child.kind() == "keyframes_name" {
                    found = Some(node_text(child, source));
                    break;
                }
            }
            match found {
                Some(f) => f,
                None => return,
            }
        }
    };

    let entity = make_entity(
        node,
        source,
        &format!("@keyframes {name}"),
        EntityKind::CssRule,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_core::relationship::RelationshipKind;

    #[test]
    fn test_parse_css_rules() {
        let parser = CssParser::new();
        let source = r#"
body {
    margin: 0;
    padding: 0;
}

.container {
    max-width: 1200px;
}

#header {
    background: blue;
}
"#;
        let graph = parser.parse_file(Path::new("style.css"), source).unwrap();
        let rules: Vec<_> = graph
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::CssRule)
            .collect();
        assert_eq!(rules.len(), 3); // body, .container, #header
    }

    #[test]
    fn test_parse_css_import() {
        let parser = CssParser::new();
        let source = r#"
@import "reset.css";
@import url("fonts.css");

body {
    font-family: sans-serif;
}
"#;
        let graph = parser.parse_file(Path::new("style.css"), source).unwrap();
        let imports: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert!(!imports.is_empty());
    }

    #[test]
    fn test_parse_css_media_query() {
        let parser = CssParser::new();
        let source = r#"
@media (max-width: 768px) {
    .container {
        padding: 10px;
    }
}
"#;
        let graph = parser.parse_file(Path::new("style.css"), source).unwrap();
        let rules: Vec<_> = graph
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::CssRule)
            .collect();
        // @media rule + nested .container rule
        assert!(rules.len() >= 2);
    }
}
