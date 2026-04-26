use std::path::Path;

use codegraph_core::entity::{EntityKind, Visibility};
use codegraph_core::{CodeGraph, Result};
use tree_sitter::{Node, Parser};

use crate::helpers::*;
use crate::LanguageParser;

pub struct HtmlParser;

impl HtmlParser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HtmlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for HtmlParser {
    fn extensions(&self) -> &[&str] {
        &["html", "htm"]
    }

    fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .map_err(|e| codegraph_core::Error::Parser(e.to_string()))?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| codegraph_core::Error::Parser("Failed to parse HTML file".into()))?;

        let file_path = path.to_string_lossy().to_string();
        let mut graph = CodeGraph::new();

        graph.add_entity(make_file_entity(&file_path, path, source));
        extract_elements(tree.root_node(), source, &file_path, &file_path, &mut graph);

        Ok(graph)
    }
}

fn extract_elements(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "element" | "self_closing_tag" => {
                extract_element(child, source, file_path, parent_qualified, graph);
            }
            "script_element" | "style_element" => {
                extract_special_element(child, source, file_path, parent_qualified, graph);
            }
            _ => {
                extract_elements(child, source, file_path, parent_qualified, graph);
            }
        }
    }
}

fn extract_element(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // Get the tag name from start_tag or self_closing_tag
    let tag_name = if node.kind() == "self_closing_tag" {
        node.child_by_field_name("tag_name")
            .or_else(|| child_by_kind(node, "tag_name"))
            .map(|n| node_text(n, source))
    } else {
        child_by_kind(node, "start_tag")
            .and_then(|start| child_by_kind(start, "tag_name"))
            .map(|n| node_text(n, source))
    };

    let tag_name = match tag_name {
        Some(name) => name,
        None => return,
    };

    // Only track significant elements (not every <div>/<span>)
    let significant = matches!(
        tag_name,
        "html"
            | "head"
            | "body"
            | "main"
            | "nav"
            | "header"
            | "footer"
            | "section"
            | "article"
            | "aside"
            | "form"
            | "table"
            | "script"
            | "style"
            | "link"
            | "meta"
            | "template"
    );

    if !significant {
        // Still recurse for nested significant elements
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_elements(child, source, file_path, parent_qualified, graph);
        }
        return;
    }

    // Use line number to disambiguate repeated tags
    let line = node.start_position().row + 1;

    // Check for id attribute to make the name more specific
    let id_attr = get_attribute(node, source, "id");
    let display_name = match &id_attr {
        Some(id) => format!("<{tag_name}#{id}>"),
        None => format!("<{tag_name}>@L{line}"),
    };

    let _qualified_name = format!("{parent_qualified}::{display_name}");

    let entity = make_entity(
        node,
        source,
        &display_name,
        EntityKind::HtmlElement,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Track script/style source references
    if tag_name == "script" || tag_name == "link" {
        let src_attr =
            get_attribute(node, source, "src").or_else(|| get_attribute(node, source, "href"));
        if let Some(src) = src_attr {
            add_import(graph, &qn, &src, line as u32);
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_elements(child, source, file_path, &qn, graph);
    }
}

fn extract_special_element(
    node: Node,
    source: &str,
    file_path: &str,
    parent_qualified: &str,
    graph: &mut CodeGraph,
) {
    // script_element and style_element have a start_tag child
    let tag_name = if node.kind() == "script_element" {
        "script"
    } else {
        "style"
    };

    let line = node.start_position().row + 1;
    let display_name = format!("<{tag_name}>@L{line}");

    let entity = make_entity(
        node,
        source,
        &display_name,
        EntityKind::HtmlElement,
        file_path,
        parent_qualified,
        Visibility::Public,
    );
    let qn = entity.qualified_name.clone();
    graph.add_entity(entity);
    add_contains(graph, parent_qualified, &qn);

    // Track script src references
    if tag_name == "script"
        && let Some(start_tag) = child_by_kind(node, "start_tag") {
            let src = get_attribute_from_tag(start_tag, source, "src");
            if let Some(src) = src {
                add_import(graph, &qn, &src, line as u32);
            }
        }
}

fn get_attribute(node: Node, source: &str, attr_name: &str) -> Option<String> {
    // Look in start_tag or self_closing_tag for attributes
    let tag_node = if node.kind() == "self_closing_tag" {
        Some(node)
    } else {
        child_by_kind(node, "start_tag")
    };

    tag_node.and_then(|tag| get_attribute_from_tag(tag, source, attr_name))
}

fn get_attribute_from_tag(tag: Node, source: &str, attr_name: &str) -> Option<String> {
    let mut cursor = tag.walk();
    for child in tag.children(&mut cursor) {
        if child.kind() == "attribute"
            && let Some(name) = child_by_kind(child, "attribute_name")
                && node_text(name, source) == attr_name {
                    let value_node = child_by_kind(child, "quoted_attribute_value")
                        .or_else(|| child_by_kind(child, "attribute_value"));
                    return value_node.map(|v| {
                        node_text(v, source)
                            .trim_matches(|c| c == '"' || c == '\'')
                            .to_string()
                    });
                }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_core::relationship::RelationshipKind;

    #[test]
    fn test_parse_html_basic() {
        let parser = HtmlParser::new();
        let source = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Test</title>
</head>
<body>
    <main id="app">
        <p>Hello</p>
    </main>
</body>
</html>"#;
        let graph = parser.parse_file(Path::new("index.html"), source).unwrap();
        // Should find html, head, body, meta, main elements
        assert!(!graph.entities.is_empty());
        let html_elements: Vec<_> = graph
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::HtmlElement)
            .collect();
        assert!(html_elements.len() >= 3);
    }

    #[test]
    fn test_parse_html_script_src() {
        let parser = HtmlParser::new();
        let source = r#"<html>
<body>
    <script src="app.js"></script>
</body>
</html>"#;
        let graph = parser.parse_file(Path::new("index.html"), source).unwrap();
        eprintln!("Entities: {:#?}", graph.entities.keys().collect::<Vec<_>>());
        eprintln!("Relationships: {:#?}", graph.relationships);
        let imports: Vec<_> = graph
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert!(!imports.is_empty());
    }

    #[test]
    fn test_parse_html_id_attribute() {
        let parser = HtmlParser::new();
        let source = r#"<html>
<body>
    <main id="content">
        <section id="intro">Hello</section>
    </main>
</body>
</html>"#;
        let graph = parser.parse_file(Path::new("index.html"), source).unwrap();
        let has_id_entity = graph
            .entities
            .values()
            .any(|e| e.name.contains("#content") || e.name.contains("#intro"));
        assert!(has_id_entity);
    }
}
