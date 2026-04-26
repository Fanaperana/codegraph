use std::collections::HashSet;

use neo4rs::{query, ConfigBuilder, Graph};
use tracing::info;

use codegraph_core::config::Neo4jConfig;
use codegraph_core::entity::CodeEntity;
use codegraph_core::error::Error;
use codegraph_core::relationship::Relationship;
use codegraph_core::Result;

pub use crate::queries;
use crate::schema;

/// High-level interface to the Neo4j graph store.
pub struct GraphStore {
    graph: Graph,
    embedding_dimensions: usize,
}

impl GraphStore {
    /// Connect to Neo4j using the provided config.
    pub async fn connect(config: &Neo4jConfig, embedding_dimensions: usize) -> Result<Self> {
        let neo_config = ConfigBuilder::default()
            .uri(&config.uri)
            .user(&config.username)
            .password(&config.password)
            .db(config.database.as_str())
            .max_connections(config.max_connections)
            .build()
            .map_err(|e| Error::Neo4j(e.to_string()))?;

        let graph = Graph::connect(neo_config)
            .await
            .map_err(|e| Error::Neo4j(e.to_string()))?;

        Ok(Self {
            graph,
            embedding_dimensions,
        })
    }

    /// Set up the graph schema (constraints, indexes, vector indexes).
    pub async fn setup_schema(&self) -> Result<()> {
        schema::setup_schema(&self.graph, self.embedding_dimensions).await
    }

    /// Upsert a batch of entities into Neo4j using UNWIND for efficiency.
    pub async fn upsert_entities(&self, entities: &[CodeEntity]) -> Result<usize> {
        if entities.is_empty() {
            return Ok(0);
        }

        let mut count = 0;

        // Group entities by kind (label) since MERGE needs a specific label
        let mut by_label: std::collections::HashMap<&str, Vec<&CodeEntity>> =
            std::collections::HashMap::new();

        for entity in entities {
            by_label
                .entry(entity.neo4j_label())
                .or_default()
                .push(entity);
        }

        for (label, group) in &by_label {
            // Build parameter list for UNWIND
            let params: Vec<neo4rs::BoltType> = group
                .iter()
                .map(|e| {
                    let mut map = neo4rs::BoltMap::new();
                    map.put(
                        neo4rs::BoltString::from("qualified_name"),
                        neo4rs::BoltType::String(e.qualified_name.clone().into()),
                    );
                    map.put(
                        neo4rs::BoltString::from("name"),
                        neo4rs::BoltType::String(e.name.clone().into()),
                    );
                    map.put(
                        neo4rs::BoltString::from("file_path"),
                        neo4rs::BoltType::String(e.file_path.clone().into()),
                    );
                    map.put(
                        neo4rs::BoltString::from("visibility"),
                        neo4rs::BoltType::String(e.visibility.to_string().into()),
                    );
                    map.put(
                        neo4rs::BoltString::from("line_start"),
                        neo4rs::BoltType::Integer(neo4rs::BoltInteger::new(e.line_start as i64)),
                    );
                    map.put(
                        neo4rs::BoltString::from("line_end"),
                        neo4rs::BoltType::Integer(neo4rs::BoltInteger::new(e.line_end as i64)),
                    );
                    map.put(
                        neo4rs::BoltString::from("source_hash"),
                        neo4rs::BoltType::String(e.source_hash.clone().into()),
                    );
                    map.put(
                        neo4rs::BoltString::from("is_async"),
                        neo4rs::BoltType::Boolean(neo4rs::BoltBoolean::new(e.is_async)),
                    );

                    if let Some(ref doc) = e.doc_comment {
                        map.put(
                            neo4rs::BoltString::from("doc_comment"),
                            neo4rs::BoltType::String(doc.clone().into()),
                        );
                    }
                    if let Some(ref sig) = e.signature {
                        map.put(
                            neo4rs::BoltString::from("signature"),
                            neo4rs::BoltType::String(sig.clone().into()),
                        );
                    }
                    if let Some(ref module) = e.module_path {
                        map.put(
                            neo4rs::BoltString::from("module_path"),
                            neo4rs::BoltType::String(module.clone().into()),
                        );
                    }

                    neo4rs::BoltType::Map(map)
                })
                .collect();

            let cypher = format!(
                r#"UNWIND $batch AS props
                MERGE (n:{label} {{qualified_name: props.qualified_name}})
                SET n.name = props.name,
                    n.file_path = props.file_path,
                    n.visibility = props.visibility,
                    n.line_start = props.line_start,
                    n.line_end = props.line_end,
                    n.source_hash = props.source_hash,
                    n.is_async = props.is_async,
                    n.doc_comment = props.doc_comment,
                    n.signature = props.signature,
                    n.module_path = props.module_path"#
            );

            let bolt_list = neo4rs::BoltType::List(neo4rs::BoltList::from(params));
            self.graph
                .run(query(&cypher).param("batch", bolt_list))
                .await
                .map_err(|e| Error::Neo4j(e.to_string()))?;

            count += group.len();
        }

        // Set embeddings separately (vector property needs special handling)
        for entity in entities {
            if let Some(ref embedding) = entity.embedding {
                let embedding_f64: Vec<f64> = embedding.iter().map(|&x| x as f64).collect();
                self.graph
                    .run(
                        query(
                            "MATCH (n {qualified_name: $qn}) \
                             CALL db.create.setNodeVectorProperty(n, 'embedding', $vec)",
                        )
                        .param("qn", entity.qualified_name.as_str())
                        .param("vec", embedding_f64),
                    )
                    .await
                    .map_err(|e| Error::Neo4j(e.to_string()))?;
            }
        }

        info!("Upserted {count} entities");
        Ok(count)
    }

    /// Upsert relationships into Neo4j.
    pub async fn upsert_relationships(&self, relationships: &[Relationship]) -> Result<usize> {
        if relationships.is_empty() {
            return Ok(0);
        }

        let mut count = 0;

        // Group by relationship kind
        let mut by_type: std::collections::HashMap<&str, Vec<&Relationship>> =
            std::collections::HashMap::new();
        for rel in relationships {
            by_type
                .entry(rel.kind.neo4j_type())
                .or_default()
                .push(rel);
        }

        for (rel_type, group) in &by_type {
            let params: Vec<neo4rs::BoltType> = group
                .iter()
                .map(|r| {
                    let mut map = neo4rs::BoltMap::new();
                    map.put(
                        neo4rs::BoltString::from("from_qn"),
                        neo4rs::BoltType::String(r.from.clone().into()),
                    );
                    map.put(
                        neo4rs::BoltString::from("to_qn"),
                        neo4rs::BoltType::String(r.to.clone().into()),
                    );
                    if let Some(line) = r.metadata.line {
                        map.put(
                            neo4rs::BoltString::from("line"),
                            neo4rs::BoltType::Integer(neo4rs::BoltInteger::new(line as i64)),
                        );
                    }
                    if let Some(ref field_name) = r.metadata.field_name {
                        map.put(
                            neo4rs::BoltString::from("field_name"),
                            neo4rs::BoltType::String(field_name.clone().into()),
                        );
                    }
                    if let Some(ref field_type) = r.metadata.field_type {
                        map.put(
                            neo4rs::BoltString::from("field_type"),
                            neo4rs::BoltType::String(field_type.clone().into()),
                        );
                    }
                    if let Some(ref variant_name) = r.metadata.variant_name {
                        map.put(
                            neo4rs::BoltString::from("variant_name"),
                            neo4rs::BoltType::String(variant_name.clone().into()),
                        );
                    }
                    if let Some(ref dep_kind) = r.metadata.dep_kind {
                        map.put(
                            neo4rs::BoltString::from("dep_kind"),
                            neo4rs::BoltType::String(dep_kind.clone().into()),
                        );
                    }
                    neo4rs::BoltType::Map(map)
                })
                .collect();

            // Use MATCH for both ends — if either doesn't exist, the row is silently skipped
            let cypher = format!(
                r#"UNWIND $batch AS props
                MATCH (a {{qualified_name: props.from_qn}})
                MATCH (b {{qualified_name: props.to_qn}})
                MERGE (a)-[r:{rel_type}]->(b)
                SET r.line = props.line,
                    r.field_name = props.field_name,
                    r.field_type = props.field_type,
                    r.variant_name = props.variant_name,
                    r.dep_kind = props.dep_kind"#
            );

            let bolt_list = neo4rs::BoltType::List(neo4rs::BoltList::from(params));
            self.graph
                .run(query(&cypher).param("batch", bolt_list))
                .await
                .map_err(|e| Error::Neo4j(e.to_string()))?;

            count += group.len();
        }

        info!("Upserted {count} relationships");
        Ok(count)
    }

    /// Remove entities whose source_hash is NOT in the provided set.
    pub async fn delete_stale(&self, current_hashes: &HashSet<String>) -> Result<usize> {
        if current_hashes.is_empty() {
            return Ok(0);
        }

        let hashes: Vec<&str> = current_hashes.iter().map(|s| s.as_str()).collect();
        let cypher = r#"
            MATCH (n)
            WHERE n.source_hash IS NOT NULL
              AND NOT n.source_hash IN $hashes
            DETACH DELETE n
            RETURN count(n) AS deleted
        "#;

        let mut result = self
            .graph
            .execute(query(cypher).param("hashes", hashes))
            .await
            .map_err(|e| Error::Neo4j(e.to_string()))?;

        let deleted: i64 = if let Some(row) =
            result.next().await.map_err(|e| Error::Neo4j(e.to_string()))?
        {
            row.get("deleted").unwrap_or(0)
        } else {
            0
        };

        if deleted > 0 {
            info!("Deleted {deleted} stale entities");
        }
        Ok(deleted as usize)
    }

    /// Query graph neighbors.
    pub async fn query_neighbors(
        &self,
        qualified_name: &str,
        depth: u32,
        direction: &str,
    ) -> Result<Vec<queries::NeighborResult>> {
        queries::query_neighbors(&self.graph, qualified_name, depth, direction).await
    }

    /// Vector similarity search.
    pub async fn query_vector(
        &self,
        embedding: &[f32],
        label: &str,
        top_k: usize,
    ) -> Result<Vec<queries::VectorResult>> {
        queries::query_vector(&self.graph, embedding, label, top_k).await
    }

    /// Shortest path between two entities.
    pub async fn shortest_path(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<queries::PathNode>> {
        queries::shortest_path(&self.graph, from, to).await
    }

    /// Get graph statistics.
    pub async fn stats(&self) -> Result<Vec<(String, i64)>> {
        queries::graph_stats(&self.graph).await
    }

    /// Get the underlying Neo4j graph handle.
    pub fn inner(&self) -> &Graph {
        &self.graph
    }
}
