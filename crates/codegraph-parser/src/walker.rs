use std::path::{Path, PathBuf};

use codegraph_core::{CodeGraph, Result};
use ignore::WalkBuilder;
use tracing::debug;

use crate::ParserRegistry;

/// Walk a directory tree, parse all supported files, and return a combined CodeGraph.
pub fn walk_and_parse(
    root: &Path,
    excludes: &[String],
    registry: &ParserRegistry,
) -> Result<CodeGraph> {
    let mut graph = CodeGraph::new();
    let files = collect_files(root, excludes)?;

    for file_path in &files {
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                debug!("Skipping {}: {e}", file_path.display());
                continue;
            }
        };

        // Use relative path from root for the graph
        let rel_path = file_path
            .strip_prefix(root)
            .unwrap_or(file_path);

        match registry.parse_file(rel_path, &source) {
            Ok(file_graph) => {
                let ec = file_graph.entity_count();
                let rc = file_graph.relationship_count();
                if ec > 0 {
                    debug!(
                        "Parsed {}: {ec} entities, {rc} relationships",
                        rel_path.display()
                    );
                }
                graph.merge(file_graph);
            }
            Err(e) => {
                debug!("Error parsing {}: {e}", rel_path.display());
            }
        }
    }

    Ok(graph)
}

/// Collect all files under `root`, respecting .gitignore and custom excludes.
fn collect_files(root: &Path, excludes: &[String]) -> Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true);

    // Add custom exclude globs
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    for pattern in excludes {
        // Prefix with ! to negate (exclude)
        let _ = overrides.add(&format!("!{pattern}"));
    }
    if let Ok(built) = overrides.build() {
        builder.overrides(built);
    }

    let mut files = Vec::new();
    for entry in builder.build() {
        match entry {
            Ok(entry) => {
                if entry.file_type().is_some_and(|ft| ft.is_file()) {
                    files.push(entry.into_path());
                }
            }
            Err(e) => {
                debug!("Walk error: {e}");
            }
        }
    }

    Ok(files)
}
