# Contributing to codegraph

Thanks for contributing. This project is a Rust monorepo for indexing codebases,
building a graph in Neo4j, and querying it with hybrid retrieval.

## Before You Start

- Read the [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Search existing issues and pull requests before opening a new one
- Prefer small, reviewable changes over broad rewrites

## Development Setup

1. Clone the repository.
2. Start Neo4j if your change needs graph storage:

```bash
docker compose up -d
```

3. Build the workspace:

```bash
cargo build --workspace
```

4. Run tests and linting before opening a pull request:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

## Project Layout

- `crates/codegraph-core`: shared types, config, errors, graph model
- `crates/codegraph-parser`: tree-sitter based language parsers
- `crates/codegraph-graph`: Neo4j schema, storage, and graph queries
- `crates/codegraph-embed`: embedding providers
- `crates/codegraph-query`: hybrid retrieval engine
- `crates/codegraph-cli`: command-line interface

## What Good Contributions Look Like

- Bug fixes with a focused reproduction and a regression test when practical
- New parser support with tests for real syntax cases
- Query or ranking improvements with measurable behavior changes
- Documentation updates that match the implemented behavior

## Pull Request Guidelines

- Keep PRs scoped to one concern
- Add or update tests for behavior changes
- Avoid unrelated refactors in the same PR
- Update docs or config examples when user-facing behavior changes
- Write a PR description that explains the problem, the change, and how you
  validated it

## Commit Style

Any clear commit message is acceptable. Short imperative messages work well, for
example:

- `Add TypeScript parser support`
- `Fix Neo4j schema setup for vector indexes`
- `Update README quick start`

## Reporting Bugs

Please include:

- What you expected to happen
- What actually happened
- Steps to reproduce
- Relevant logs, stack traces, or screenshots
- Your environment details when relevant

## Feature Requests

Feature requests are welcome. Please describe the use case, the expected user
experience, and any constraints around language support, storage, or embeddings.

## Questions

If you are unsure where to start, open an issue with context and the proposed
direction before implementing a large change.