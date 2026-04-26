#![allow(clippy::format_in_format_args)]

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use colored::{ColoredString, Colorize};
use tracing::info;

use codegraph_core::Config;
use codegraph_embed::Embedder;
use codegraph_graph::GraphStore;
use codegraph_parser::{ParserRegistry, walker};
use codegraph_query::HybridEngine;

#[derive(Parser)]
#[command(name = "codegraph", version, about = "Codebase graph with hybrid retrieval (graph + vector)")]
struct Cli {
    /// Path to config file (default: auto-discover codegraph.toml)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Output format
    #[arg(short, long, global = true, default_value = "text")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new codegraph.toml config file
    Init,

    /// Index a codebase: parse, embed, and store in Neo4j
    Index {
        /// Path to the codebase to index
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        /// Re-index everything (ignore source_hash cache)
        #[arg(long)]
        full: bool,
    },

    /// Hybrid search: vector similarity + graph expansion
    Search {
        /// Natural language query or code snippet
        query: String,

        /// Number of results to return
        #[arg(short = 'k', long, default_value = "10")]
        top_k: usize,
    },

    /// Show full context for an entity (callers, callees, deps, similar)
    Explain {
        /// Qualified name of the entity
        entity: String,
    },

    /// Find shortest path between two entities
    Path {
        /// Qualified name of the source entity
        from: String,
        /// Qualified name of the target entity
        to: String,
    },

    /// Show graph schema and statistics
    Schema,
}

/// Colorize an entity kind label.
fn kind_color(kind: &str) -> ColoredString {
    match kind {
        "Function" | "Method" | "Constructor" => kind.bright_green(),
        "Struct" | "Class" | "Interface" | "Enum" | "Union" | "TypeAlias" | "Typedef" => {
            kind.bright_yellow()
        }
        "Trait" | "Impl" => kind.magenta(),
        "Module" | "Namespace" | "File" | "Header" => kind.bright_blue(),
        "Constant" | "Static" | "Variable" | "Property" => kind.cyan(),
        "Macro" | "Decorator" | "Preprocessor" => kind.bright_magenta(),
        "HtmlElement" | "CssRule" | "CssSelector" => kind.bright_cyan(),
        _ => kind.white(),
    }
}

/// Colorize a relevance score from red (low) to green (high).
fn relevance_color(score: f64) -> ColoredString {
    let formatted = format!("{score:.3}");
    if score >= 0.85 {
        formatted.bright_green().bold()
    } else if score >= 0.70 {
        formatted.green()
    } else if score >= 0.50 {
        formatted.yellow()
    } else {
        formatted.red()
    }
}

/// Colorize a relationship type.
fn rel_color(rel: &str) -> ColoredString {
    match rel {
        "CALLS" => rel.bright_green(),
        "CONTAINS" => rel.dimmed(),
        "IMPORTS" | "DEPENDS_ON" => rel.bright_blue(),
        "USES_TYPE" | "RETURNS" | "ACCEPTS_PARAM" => rel.cyan(),
        "IMPLEMENTS" | "EXTENDS" | "IMPL_FOR" => rel.magenta(),
        _ => rel.white(),
    }
}

/// Direction arrow + color.
fn direction_arrow(direction: &str) -> ColoredString {
    match direction {
        "in" => "<-".bright_red(),
        "out" => "->".bright_green(),
        _ => "--".dimmed(),
    }
}

/// Format a qualified name by splitting "path::ident" so the trailing
/// identifier stands out against the dimmed module path.
fn format_qname(qname: &str) -> String {
    if let Some(idx) = qname.rfind("::") {
        let (path, ident) = (&qname[..idx + 2], &qname[idx + 2..]);
        format!("{}{}", path.dimmed(), ident.bold())
    } else {
        qname.bold().to_string()
    }
}

fn header(text: &str) -> String {
    text.bright_white().bold().underline().to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("codegraph=info".parse()?),
        )
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Init => cmd_init()?,
        Commands::Index { path, full } => cmd_index(&cli, path, *full).await?,
        Commands::Search { query, top_k } => cmd_search(&cli, query, *top_k).await?,
        Commands::Explain { entity } => cmd_explain(&cli, entity).await?,
        Commands::Path { from, to } => cmd_path(&cli, from, to).await?,
        Commands::Schema => cmd_schema(&cli).await?,
    }

    Ok(())
}

fn load_config(cli: &Cli) -> anyhow::Result<Config> {
    let config_path = match &cli.config {
        Some(p) => p.clone(),
        None => Config::discover(&std::env::current_dir()?)
            .context("No codegraph.toml found. Run `codegraph init` to create one.")?,
    };
    Config::from_file(&config_path).context("Failed to load config")
}

fn cmd_init() -> anyhow::Result<()> {
    let path = PathBuf::from("codegraph.toml");
    if path.exists() {
        anyhow::bail!("codegraph.toml already exists");
    }
    std::fs::write(&path, Config::default_toml())?;
    println!(
        "{} Created {}",
        "✓".bright_green().bold(),
        "codegraph.toml".bright_cyan().bold()
    );
    Ok(())
}

async fn cmd_index(cli: &Cli, path: &PathBuf, _full: bool) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let abs_path = std::fs::canonicalize(path)?;

    info!("Indexing {}", abs_path.display());

    // Step 1: Parse the codebase
    let registry = ParserRegistry::new();
    let graph = walker::walk_and_parse(&abs_path, &config.indexing.exclude, &registry)?;
    info!(
        "Parsed {} entities, {} relationships",
        graph.entity_count(),
        graph.relationship_count()
    );

    // Step 2: Generate embeddings (if enabled)
    let embedder = if config.indexing.embed {
        codegraph_embed::from_config(&config.embedding)?
    } else {
        Embedder::Noop(codegraph_embed::NoopProvider::new(config.embedding.dimensions))
    };

    let mut entities: Vec<_> = graph.entities.into_values().collect();

    if config.indexing.embed {
        info!("Generating embeddings...");
        let texts: Vec<String> = entities.iter().map(|e| e.embedding_text()).collect();

        // Batch embedding
        for chunk in texts.chunks(config.indexing.batch_size) {
            let embeddings = embedder.embed(chunk).await?;
            let offset = entities.len() - texts.len() + (chunk.as_ptr() as usize - texts.as_ptr() as usize) / std::mem::size_of::<String>();
            for (i, emb) in embeddings.into_iter().enumerate() {
                if offset + i < entities.len() {
                    entities[offset + i].embedding = Some(emb);
                }
            }
        }
        info!("Embeddings generated");
    }

    // Step 3: Store in Neo4j
    let store = GraphStore::connect(&config.neo4j, config.embedding.dimensions).await?;
    store.setup_schema().await?;

    // Batch upsert entities
    for chunk in entities.chunks(config.indexing.batch_size) {
        store.upsert_entities(chunk).await?;
    }

    // Upsert relationships
    for chunk in graph.relationships.chunks(config.indexing.batch_size) {
        store.upsert_relationships(chunk).await?;
    }

    // Clean up stale entities
    let current_hashes: HashSet<String> = entities.iter().map(|e| e.source_hash.clone()).collect();
    let deleted = store.delete_stale(&current_hashes).await?;

    let stale_msg = if deleted > 0 {
        format!(", removed {deleted} stale").yellow().to_string()
    } else {
        String::new()
    };
    println!(
        "{} {} {}, {}{}",
        "✓".bright_green().bold(),
        "Indexed".bright_white().bold(),
        format!("{} entities", entities.len()).bright_cyan(),
        format!("{} relationships", graph.relationships.len()).bright_cyan(),
        stale_msg
    );

    Ok(())
}

async fn cmd_search(cli: &Cli, query: &str, top_k: usize) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let store = GraphStore::connect(&config.neo4j, config.embedding.dimensions).await?;
    let embedder = codegraph_embed::from_config(&config.embedding)?;

    let engine = HybridEngine::new(&store, &embedder);
    let results = engine.search(query, top_k).await?;

    match cli.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        OutputFormat::Text => {
            if results.is_empty() {
                println!("{}", "No results found.".yellow());
                return Ok(());
            }
            println!(
                "{} {} {}",
                header("Search results for"),
                format!("\"{query}\"").bright_cyan(),
                format!("({} matches)", results.len()).dimmed()
            );
            println!();
            for (i, result) in results.iter().enumerate() {
                println!(
                    "{} {}  {}  {} {}",
                    format!("{:>2}.", i + 1).bright_white().bold(),
                    format_qname(&result.qualified_name),
                    format!("[{}]", kind_color(&result.kind)),
                    "relevance:".dimmed(),
                    relevance_color(result.relevance)
                );
                if !result.related.is_empty() {
                    for rel in &result.related {
                        println!(
                            "      {} {}  {}  {}",
                            direction_arrow(&rel.direction),
                            format_qname(&rel.qualified_name),
                            format!("[{}]", kind_color(&rel.kind)),
                            rel_color(&rel.relationship)
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

async fn cmd_explain(cli: &Cli, entity: &str) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let store = GraphStore::connect(&config.neo4j, config.embedding.dimensions).await?;
    let embedder = codegraph_embed::from_config(&config.embedding)?;

    let engine = HybridEngine::new(&store, &embedder);
    let ctx = engine.explain(entity).await?;

    match cli.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&ctx)?);
        }
        OutputFormat::Text => {
            println!(
                "{} {}  {}",
                header("Entity"),
                format_qname(&ctx.qualified_name),
                format!("[{}]", kind_color(&ctx.kind))
            );
            println!();

            if !ctx.callers.is_empty() {
                println!("{}", header("Called by"));
                for c in &ctx.callers {
                    println!(
                        "  {} {}  {}",
                        "<-".bright_red(),
                        format_qname(&c.qualified_name),
                        format!("[{}]", kind_color(&c.kind))
                    );
                }
                println!();
            }

            if !ctx.callees.is_empty() {
                println!("{}", header("Calls"));
                for c in &ctx.callees {
                    println!(
                        "  {} {}  {}",
                        "->".bright_green(),
                        format_qname(&c.qualified_name),
                        format!("[{}]", kind_color(&c.kind))
                    );
                }
                println!();
            }

            if !ctx.dependencies.is_empty() {
                println!("{}", header("Depends on"));
                for d in &ctx.dependencies {
                    println!(
                        "  {} {}  {}  {}",
                        "->".bright_green(),
                        format_qname(&d.qualified_name),
                        format!("[{}]", kind_color(&d.kind)),
                        rel_color(&d.relationship)
                    );
                }
                println!();
            }

            if !ctx.dependents.is_empty() {
                println!("{}", header("Depended on by"));
                for d in &ctx.dependents {
                    println!(
                        "  {} {}  {}  {}",
                        "<-".bright_red(),
                        format_qname(&d.qualified_name),
                        format!("[{}]", kind_color(&d.kind)),
                        rel_color(&d.relationship)
                    );
                }
                println!();
            }

            if !ctx.similar.is_empty() {
                println!("{}", header("Similar entities"));
                for s in &ctx.similar {
                    println!(
                        "  {} {}  {}  {} {}",
                        "~".bright_magenta(),
                        format_qname(&s.qualified_name),
                        format!("[{}]", kind_color(&s.kind)),
                        "relevance:".dimmed(),
                        relevance_color(s.relevance)
                    );
                }
            }
        }
    }

    Ok(())
}

async fn cmd_path(cli: &Cli, from: &str, to: &str) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let store = GraphStore::connect(&config.neo4j, config.embedding.dimensions).await?;
    let embedder = codegraph_embed::from_config(&config.embedding)?;

    let engine = HybridEngine::new(&store, &embedder);
    let path = engine.path(from, to).await?;

    match cli.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&path)?);
        }
        OutputFormat::Text => {
            if path.is_empty() {
                println!(
                    "{} {} {} {}",
                    "No path found between".yellow(),
                    from.bold(),
                    "and".yellow(),
                    to.bold()
                );
                return Ok(());
            }
            println!(
                "{} {}",
                header("Path"),
                format!("({} hops)", path.len() - 1).dimmed()
            );
            for (i, node) in path.iter().enumerate() {
                if i > 0 {
                    println!("    {}", "│".dimmed());
                    println!("    {}", "▼".bright_blue());
                }
                println!(
                    "  {} {}  {}",
                    "●".bright_blue(),
                    format_qname(&node.qualified_name),
                    format!("[{}]", kind_color(&node.kind))
                );
            }
        }
    }

    Ok(())
}

async fn cmd_schema(cli: &Cli) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let store = GraphStore::connect(&config.neo4j, config.embedding.dimensions).await?;

    let stats = store.stats().await?;

    match cli.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        OutputFormat::Text => {
            println!("{}", header("Graph Statistics"));
            let rule = "─".repeat(40).dimmed();
            println!("{rule}");
            let mut total = 0i64;
            for (label, count) in &stats {
                let count_str = if *count == 0 {
                    format!("{count:>8}").dimmed()
                } else {
                    format!("{count:>8}").bright_white().bold()
                };
                println!("  {:<22} {}", kind_color(label), count_str);
                total += count;
            }
            println!("{rule}");
            println!(
                "  {:<22} {}",
                "Total".bright_white().bold(),
                format!("{total:>8}").bright_cyan().bold()
            );
        }
    }

    Ok(())
}
