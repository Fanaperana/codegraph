pub mod c;
pub mod css;
pub mod helpers;
pub mod html;
pub mod js;
pub mod python;
pub mod rust;
pub mod walker;

use codegraph_core::{CodeGraph, Result};
use std::path::Path;

/// Trait for language-specific parsers.
pub trait LanguageParser: Send + Sync {
    /// File extensions this parser handles.
    fn extensions(&self) -> &[&str];

    /// Parse a single file and return the extracted graph.
    fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph>;
}

/// Registry of all available language parsers.
pub struct ParserRegistry {
    parsers: Vec<Box<dyn LanguageParser>>,
}

impl ParserRegistry {
    pub fn new() -> Self {
        Self {
            parsers: vec![
                Box::new(rust::RustParser::new()),
                Box::new(c::CParser::c()),
                Box::new(c::CParser::cpp()),
                Box::new(python::PythonParser::new()),
                Box::new(js::JsParser::javascript()),
                Box::new(js::JsParser::typescript()),
                Box::new(js::JsParser::tsx()),
                Box::new(html::HtmlParser::new()),
                Box::new(css::CssParser::new()),
            ],
        }
    }

    /// Find a parser for the given file extension.
    pub fn parser_for(&self, extension: &str) -> Option<&dyn LanguageParser> {
        self.parsers
            .iter()
            .find(|p| p.extensions().contains(&extension))
            .map(|p| p.as_ref())
    }

    /// Parse a single file, auto-detecting the language.
    pub fn parse_file(&self, path: &Path, source: &str) -> Result<CodeGraph> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match self.parser_for(ext) {
            Some(parser) => parser.parse_file(path, source),
            None => Ok(CodeGraph::new()),
        }
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}
