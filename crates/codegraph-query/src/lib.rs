use std::collections::HashMap;

use codegraph_embed::Embedder;
use codegraph_graph::GraphStore;
use serde::Serialize;
use tracing::debug;

use codegraph_core::Result;

/// A single result from a hybrid query, combining vector + graph signals.
#[derive(Debug, Clone, Serialize)]
pub struct HybridResult {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    /// Vector similarity score (0.0–1.0, higher = more similar)
    pub vector_score: f64,
    /// Graph distance from the nearest vector match (0 = direct match)
    pub graph_distance: u32,
    /// Combined relevance score
    pub relevance: f64,
    /// How this result was found
    pub source: ResultSource,
    /// Related entities discovered via graph traversal
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedEntity>,
}

#[derive(Debug, Clone, Serialize)]
pub enum ResultSource {
    /// Found directly via vector similarity
    Vector,
    /// Found via graph expansion from a vector match
    GraphExpansion,
    /// Found via exact name match
    ExactMatch,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedEntity {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub relationship: String,
    pub direction: String,
}

/// Entity context returned by `explain`.
#[derive(Debug, Serialize)]
pub struct EntityContext {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub callers: Vec<RelatedEntity>,
    pub callees: Vec<RelatedEntity>,
    pub dependencies: Vec<RelatedEntity>,
    pub dependents: Vec<RelatedEntity>,
    pub similar: Vec<HybridResult>,
}

/// The hybrid query engine combining graph traversal with vector similarity.
pub struct HybridEngine<'a> {
    store: &'a GraphStore,
    embedder: &'a Embedder,
}

impl<'a> HybridEngine<'a> {
    pub fn new(store: &'a GraphStore, embedder: &'a Embedder) -> Self {
        Self { store, embedder }
    }

    /// Hybrid search: embed the query, find vector matches, then expand via graph.
    pub async fn search(&self, query_text: &str, top_k: usize) -> Result<Vec<HybridResult>> {
        // Step 1: Embed the query text
        let embeddings = self
            .embedder
            .embed(&[query_text.to_string()])
            .await?;
        let query_embedding = &embeddings[0];

        // Detect whether the embedder produced a meaningful vector. The Noop
        // provider returns all zeros, in which case vector search is useless
        // and we fall back to text-based name matching.
        let has_vector = query_embedding.iter().any(|&x| x != 0.0);

        let mut vector_results = Vec::new();

        if has_vector {
            // Step 2a: Vector search across embeddable entity types
            let labels = ["Function", "Method", "Struct", "Enum", "Trait", "Module"];
            for label in labels {
                match self
                    .store
                    .query_vector(query_embedding, label, top_k)
                    .await
                {
                    Ok(results) => vector_results.extend(results),
                    Err(e) => {
                        debug!("Vector search for {label} failed (index may not exist): {e}");
                    }
                }
            }
        } else {
            // Step 2b: Fallback — text search by name/qualified_name
            debug!("No embedding available; falling back to name search");
            match self.store.query_by_name(query_text, top_k).await {
                Ok(results) => vector_results.extend(results),
                Err(e) => debug!("Name search failed: {e}"),
            }
        }

        // Sort by score descending and take top_k
        vector_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        vector_results.truncate(top_k);

        // Step 3: Build initial results from vector matches
        let mut results_map: HashMap<String, HybridResult> = HashMap::new();

        for vr in &vector_results {
            results_map.insert(
                vr.qualified_name.clone(),
                HybridResult {
                    name: vr.name.clone(),
                    qualified_name: vr.qualified_name.clone(),
                    kind: vr.kind.clone(),
                    vector_score: vr.score,
                    graph_distance: 0,
                    relevance: vr.score, // Will be recalculated
                    source: ResultSource::Vector,
                    related: vec![],
                },
            );
        }

        // Step 4: Graph expansion — for each vector match, find 1-hop neighbors
        for vr in &vector_results {
            if let Ok(neighbors) = self
                .store
                .query_neighbors(&vr.qualified_name, 1, "both")
                .await
            {
                for neighbor in &neighbors {
                    let related = RelatedEntity {
                        name: neighbor.name.clone(),
                        qualified_name: neighbor.qualified_name.clone(),
                        kind: neighbor.kind.clone(),
                        relationship: neighbor.rel_type.clone(),
                        direction: neighbor.direction.clone(),
                    };

                    // Add to the vector match's related list
                    if let Some(result) = results_map.get_mut(&vr.qualified_name) {
                        result.related.push(related.clone());
                    }

                    // Also add the neighbor as a graph-expansion result if not already present
                    results_map
                        .entry(neighbor.qualified_name.clone())
                        .or_insert_with(|| HybridResult {
                            name: neighbor.name.clone(),
                            qualified_name: neighbor.qualified_name.clone(),
                            kind: neighbor.kind.clone(),
                            vector_score: vr.score * 0.5, // Decay for graph expansion
                            graph_distance: 1,
                            relevance: 0.0,
                            source: ResultSource::GraphExpansion,
                            related: vec![],
                        });
                }
            }
        }

        // Step 5: Compute combined relevance scores
        let mut results: Vec<HybridResult> = results_map.into_values().collect();
        for result in &mut results {
            result.relevance = compute_relevance(result);
        }

        // Sort by relevance descending
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);

        Ok(results)
    }

    /// Explain an entity: show its full context (callers, callees, deps, similar entities).
    pub async fn explain(&self, qualified_name: &str) -> Result<EntityContext> {
        // Get callers (who calls this)
        let callers = self
            .store
            .query_neighbors(qualified_name, 1, "in")
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.rel_type == "CALLS")
            .map(|n| RelatedEntity {
                name: n.name,
                qualified_name: n.qualified_name,
                kind: n.kind,
                relationship: n.rel_type,
                direction: "in".into(),
            })
            .collect();

        // Get callees (what this calls)
        let callees = self
            .store
            .query_neighbors(qualified_name, 1, "out")
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.rel_type == "CALLS")
            .map(|n| RelatedEntity {
                name: n.name,
                qualified_name: n.qualified_name,
                kind: n.kind,
                relationship: n.rel_type,
                direction: "out".into(),
            })
            .collect();

        // Get dependencies (what this depends on)
        let dependencies = self
            .store
            .query_neighbors(qualified_name, 1, "out")
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.rel_type == "DEPENDS_ON" || n.rel_type == "IMPORTS" || n.rel_type == "USES_TYPE")
            .map(|n| RelatedEntity {
                name: n.name,
                qualified_name: n.qualified_name,
                kind: n.kind,
                relationship: n.rel_type,
                direction: "out".into(),
            })
            .collect();

        // Get dependents (what depends on this)
        let dependents = self
            .store
            .query_neighbors(qualified_name, 1, "in")
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.rel_type == "DEPENDS_ON" || n.rel_type == "IMPORTS" || n.rel_type == "USES_TYPE")
            .map(|n| RelatedEntity {
                name: n.name,
                qualified_name: n.qualified_name,
                kind: n.kind,
                relationship: n.rel_type,
                direction: "in".into(),
            })
            .collect();

        // Find similar entities via vector search
        let similar = self.search(qualified_name, 5).await.unwrap_or_default();

        // Determine the kind by looking up the entity directly. Fall back to
        // neighbor inspection if it's not present (e.g. wrong qualified name).
        let kind = match self.store.get_entity(qualified_name).await {
            Ok(Some(entity)) => entity.kind,
            _ => self
                .store
                .query_neighbors(qualified_name, 1, "both")
                .await
                .ok()
                .and_then(|n| n.first().map(|r| r.kind.clone()))
                .unwrap_or_else(|| "Unknown".into()),
        };

        let name = qualified_name
            .rsplit("::")
            .next()
            .unwrap_or(qualified_name)
            .to_string();

        Ok(EntityContext {
            name,
            qualified_name: qualified_name.to_string(),
            kind,
            callers,
            callees,
            dependencies,
            dependents,
            similar,
        })
    }

    /// Find the shortest path between two entities.
    pub async fn path(&self, from: &str, to: &str) -> Result<Vec<RelatedEntity>> {
        let nodes = self.store.shortest_path(from, to).await?;
        Ok(nodes
            .into_iter()
            .map(|n| RelatedEntity {
                name: n.name,
                qualified_name: n.qualified_name,
                kind: n.kind,
                relationship: String::new(),
                direction: String::new(),
            })
            .collect())
    }
}

/// Compute a combined relevance score from vector similarity and graph distance.
fn compute_relevance(result: &HybridResult) -> f64 {
    // Vector similarity is the primary signal
    let vector_weight = 0.7;
    // Graph proximity is the secondary signal (closer = more relevant)
    let graph_weight = 0.3;

    let graph_score = 1.0 / (1.0 + result.graph_distance as f64);

    result.vector_score * vector_weight + graph_score * graph_weight
}
