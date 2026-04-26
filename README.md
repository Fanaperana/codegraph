# codegraph

A Rust-native codebase graph with hybrid retrieval — combining **Neo4j graph traversal** with **vector similarity search** for AI agents and humans navigating large codebases.

## Architecture

```
codegraph/
├── crates/
│   ├── codegraph-core/       # Core types: entities, relationships, config
│   ├── codegraph-parser/     # tree-sitter parsing (Rust, extensible)
│   ├── codegraph-graph/      # Neo4j storage, schema, queries
│   ├── codegraph-embed/      # Embedding providers (OpenAI, local ONNX)
│   └── codegraph-query/      # Hybrid retrieval engine
│   └── codegraph-cli/        # CLI binary
├── codegraph.toml            # Default config
└── docker-compose.yml        # Neo4j setup
```

## Quick Start

```bash
# 1. Start Neo4j
docker compose up -d

# 2. Initialize config
codegraph init

# 3. Index your codebase
codegraph index --path /path/to/project

# 4. Search (hybrid: vector + graph)
codegraph search "function that parses configuration"

# 5. Explain an entity
codegraph explain "src/config.rs::Config::from_file"

# 6. Find path between entities
codegraph path "src/main.rs::main" "src/config.rs::Config"
```

## Hybrid Retrieval

The key insight: **graph** and **vector** retrieval are complementary.

| Approach | Finds | Example |
|----------|-------|---------|
| **Graph** (relationships) | Who calls what, what depends on what | "Show me everything that calls `parse_config`" |
| **Vector** (semantic similarity) | Things like this, fuzzy intent | "Find functions related to error handling" |
| **Hybrid** (both) | Fast + smart | "Find the config parser and show its callers" |

### How it works

1. **Index**: Parse source → extract entities & relationships → generate embeddings → store in Neo4j
2. **Search**: Embed query → vector similarity search → expand results via graph neighbors → rank by combined score
3. **Explain**: Graph traversal for callers/callees/deps + vector similarity for related entities

## Graph Schema

### Nodes

`File`, `Module`, `Function`, `Method`, `Struct`, `Enum`, `Trait`, `Impl`, `TypeAlias`, `Macro`, `Constant`, `Static`

### Relationships

`CONTAINS`, `DEFINES`, `CALLS`, `DEPENDS_ON`, `IMPLEMENTS`, `IMPL_FOR`, `HAS_METHOD`, `EXTENDS`, `RETURNS`, `ACCEPTS_PARAM`, `HAS_FIELD`, `HAS_VARIANT`, `USES_TYPE`, `IMPORTS`

## Configuration

See `codegraph.toml` for all options. Key settings:

```toml
[embedding]
provider = "openai"     # or "local" or "none"
dimensions = 384

[embedding.openai]
model = "text-embedding-3-small"
# Set OPENAI_API_KEY env var

[embedding.local]
model_path = "models/all-MiniLM-L6-v2.onnx"
tokenizer_path = "models/tokenizer.json"
```

## Output Formats

```bash
# Human-readable (default)
codegraph search "config parser"

# JSON (for AI agents)
codegraph search "config parser" --format json
```

## Building

```bash
# Default (OpenAI embeddings only)
cargo build --release

# With local ONNX embeddings
cargo build --release --features local

# All embedding providers
cargo build --release --features "openai,local"
```

## Adding Language Support

Implement the `LanguageParser` trait in `codegraph-parser` and register it in `ParserRegistry::new()`. Tree-sitter grammars are available for 100+ languages.
