use codegraph_core::entity::EntityKind;
use neo4rs::Graph;
use tracing::info;

/// Set up the Neo4j schema: constraints, indexes, and vector indexes.
pub async fn setup_schema(graph: &Graph, embedding_dimensions: usize) -> codegraph_core::Result<()> {
    info!("Setting up Neo4j schema...");

    // Create uniqueness constraints for each entity label
    for label in EntityKind::all_labels() {
        let cypher = format!(
            "CREATE CONSTRAINT IF NOT EXISTS FOR (n:{label}) REQUIRE n.qualified_name IS UNIQUE"
        );
        graph
            .run(neo4rs::query(&cypher))
            .await
            .map_err(|e| codegraph_core::Error::Neo4j(e.to_string()))?;
    }

    // Create indexes on common query fields
    for label in EntityKind::all_labels() {
        let cypher = format!(
            "CREATE INDEX IF NOT EXISTS FOR (n:{label}) ON (n.name)"
        );
        graph
            .run(neo4rs::query(&cypher))
            .await
            .map_err(|e| codegraph_core::Error::Neo4j(e.to_string()))?;

        let cypher = format!(
            "CREATE INDEX IF NOT EXISTS FOR (n:{label}) ON (n.file_path)"
        );
        graph
            .run(neo4rs::query(&cypher))
            .await
            .map_err(|e| codegraph_core::Error::Neo4j(e.to_string()))?;
    }

    // Create vector indexes for entity types that carry embeddings
    let embeddable_labels = [
        "Function", "Method", "Struct", "Enum", "Trait", "Module", "File",
    ];
    for label in embeddable_labels {
        let index_name = format!("{}_embedding_idx", label.to_lowercase());
        let cypher = format!(
            r#"CREATE VECTOR INDEX {index_name} IF NOT EXISTS
            FOR (n:{label})
            ON n.embedding
            OPTIONS {{
                indexConfig: {{
                    `vector.dimensions`: {embedding_dimensions},
                    `vector.similarity_function`: 'cosine'
                }}
            }}"#
        );
        graph
            .run(neo4rs::query(&cypher))
            .await
            .map_err(|e| codegraph_core::Error::Neo4j(e.to_string()))?;
    }

    info!("Schema setup complete");
    Ok(())
}
