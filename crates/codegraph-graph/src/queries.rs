use neo4rs::{query, Graph};
use serde::Deserialize;

use codegraph_core::Result;
use codegraph_core::error::Error;

/// Result from a graph neighbor query.
#[derive(Debug, Deserialize)]
pub struct NeighborResult {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub rel_type: String,
    pub direction: String,
}

/// Result from a vector similarity query.
#[derive(Debug, Deserialize)]
pub struct VectorResult {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub score: f64,
}

/// Result from a shortest path query.
#[derive(Debug, Deserialize)]
pub struct PathNode {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
}

/// Query the graph for neighbors of an entity.
pub async fn query_neighbors(
    graph: &Graph,
    qualified_name: &str,
    depth: u32,
    direction: &str, // "in", "out", "both"
) -> Result<Vec<NeighborResult>> {
    let cypher = match direction {
        "in" => format!(
            r#"MATCH (target)<-[r*1..{depth}]-(neighbor)
            WHERE target.qualified_name = $qn
            UNWIND r AS rel
            RETURN neighbor.name AS name,
                   neighbor.qualified_name AS qualified_name,
                   labels(neighbor)[0] AS kind,
                   type(rel) AS rel_type,
                   'in' AS direction"#
        ),
        "out" => format!(
            r#"MATCH (target)-[r*1..{depth}]->(neighbor)
            WHERE target.qualified_name = $qn
            UNWIND r AS rel
            RETURN neighbor.name AS name,
                   neighbor.qualified_name AS qualified_name,
                   labels(neighbor)[0] AS kind,
                   type(rel) AS rel_type,
                   'out' AS direction"#
        ),
        _ => format!(
            r#"MATCH (target)-[r*1..{depth}]-(neighbor)
            WHERE target.qualified_name = $qn
            UNWIND r AS rel
            RETURN neighbor.name AS name,
                   neighbor.qualified_name AS qualified_name,
                   labels(neighbor)[0] AS kind,
                   type(rel) AS rel_type,
                   'both' AS direction"#
        ),
    };

    let mut result = graph
        .execute(query(&cypher).param("qn", qualified_name))
        .await
        .map_err(|e| Error::Neo4j(e.to_string()))?;

    let mut neighbors = Vec::new();
    while let Some(row) = result.next().await.map_err(|e| Error::Neo4j(e.to_string()))? {
        neighbors.push(NeighborResult {
            name: row.get("name").unwrap_or_default(),
            qualified_name: row.get("qualified_name").unwrap_or_default(),
            kind: row.get("kind").unwrap_or_default(),
            rel_type: row.get("rel_type").unwrap_or_default(),
            direction: row.get("direction").unwrap_or_default(),
        });
    }

    Ok(neighbors)
}

/// Perform vector similarity search against a specific entity label.
pub async fn query_vector(
    graph: &Graph,
    embedding: &[f32],
    label: &str,
    top_k: usize,
) -> Result<Vec<VectorResult>> {
    let index_name = format!("{}_embedding_idx", label.to_lowercase());
    let embedding_f64: Vec<f64> = embedding.iter().map(|&x| x as f64).collect();

    let cypher = format!(
        r#"CALL db.index.vector.queryNodes('{index_name}', $topk, $embedding)
        YIELD node, score
        RETURN node.name AS name,
               node.qualified_name AS qualified_name,
               labels(node)[0] AS kind,
               score"#
    );

    let mut result = graph
        .execute(
            query(&cypher)
                .param("topk", top_k as i64)
                .param("embedding", embedding_f64),
        )
        .await
        .map_err(|e| Error::Neo4j(e.to_string()))?;

    let mut results = Vec::new();
    while let Some(row) = result.next().await.map_err(|e| Error::Neo4j(e.to_string()))? {
        results.push(VectorResult {
            name: row.get("name").unwrap_or_default(),
            qualified_name: row.get("qualified_name").unwrap_or_default(),
            kind: row.get("kind").unwrap_or_default(),
            score: row.get("score").unwrap_or_default(),
        });
    }

    Ok(results)
}

/// Find the shortest path between two entities.
pub async fn shortest_path(
    graph: &Graph,
    from_qn: &str,
    to_qn: &str,
) -> Result<Vec<PathNode>> {
    let cypher = r#"
        MATCH (a {qualified_name: $from_qn}), (b {qualified_name: $to_qn}),
              p = shortestPath((a)-[*..15]-(b))
        RETURN [n IN nodes(p) | {
            name: n.name,
            qualified_name: n.qualified_name,
            kind: labels(n)[0]
        }] AS path
    "#;

    let mut result = graph
        .execute(
            query(cypher)
                .param("from_qn", from_qn)
                .param("to_qn", to_qn),
        )
        .await
        .map_err(|e| Error::Neo4j(e.to_string()))?;

    if let Some(row) = result.next().await.map_err(|e| Error::Neo4j(e.to_string()))? {
        // The path is returned as a list of maps
        let path: Vec<PathNode> = row.get("path").unwrap_or_default();
        Ok(path)
    } else {
        Ok(vec![])
    }
}

/// Look up a single entity by qualified_name. Returns None if not found.
pub async fn get_entity(graph: &Graph, qualified_name: &str) -> Result<Option<VectorResult>> {
    let cypher = r#"
        MATCH (n {qualified_name: $qn})
        RETURN n.name AS name,
               n.qualified_name AS qualified_name,
               labels(n)[0] AS kind
        LIMIT 1
    "#;

    let mut result = graph
        .execute(query(cypher).param("qn", qualified_name))
        .await
        .map_err(|e| Error::Neo4j(e.to_string()))?;

    if let Some(row) = result.next().await.map_err(|e| Error::Neo4j(e.to_string()))? {
        Ok(Some(VectorResult {
            name: row.get("name").unwrap_or_default(),
            qualified_name: row.get("qualified_name").unwrap_or_default(),
            kind: row.get("kind").unwrap_or_default(),
            score: 1.0,
        }))
    } else {
        Ok(None)
    }
}

/// Text-based search over entity names and qualified names (case-insensitive substring match).
/// Used as a fallback when vector search is unavailable (e.g. provider = "none").
pub async fn query_by_name(
    graph: &Graph,
    text: &str,
    top_k: usize,
) -> Result<Vec<VectorResult>> {
    let cypher = r#"
        MATCH (n)
        WHERE (n.name IS NOT NULL AND toLower(n.name) CONTAINS toLower($text))
           OR (n.qualified_name IS NOT NULL AND toLower(n.qualified_name) CONTAINS toLower($text))
        RETURN n.name AS name,
               n.qualified_name AS qualified_name,
               labels(n)[0] AS kind,
               CASE
                 WHEN toLower(n.name) = toLower($text) THEN 1.0
                 WHEN toLower(n.name) STARTS WITH toLower($text) THEN 0.8
                 ELSE 0.5
               END AS score
        ORDER BY score DESC, n.qualified_name ASC
        LIMIT $topk
    "#;

    let mut result = graph
        .execute(
            query(cypher)
                .param("text", text)
                .param("topk", top_k as i64),
        )
        .await
        .map_err(|e| Error::Neo4j(e.to_string()))?;

    let mut results = Vec::new();
    while let Some(row) = result.next().await.map_err(|e| Error::Neo4j(e.to_string()))? {
        results.push(VectorResult {
            name: row.get("name").unwrap_or_default(),
            qualified_name: row.get("qualified_name").unwrap_or_default(),
            kind: row.get("kind").unwrap_or_default(),
            score: row.get("score").unwrap_or_default(),
        });
    }

    Ok(results)
}

/// Get statistics about the graph.
pub async fn graph_stats(graph: &Graph) -> Result<Vec<(String, i64)>> {
    let cypher = r#"
        CALL db.labels() YIELD label
        CALL {
            WITH label
            MATCH (n)
            WHERE label IN labels(n)
            RETURN count(n) AS cnt
        }
        RETURN label, cnt
        ORDER BY cnt DESC
    "#;

    let mut result = graph
        .execute(query(cypher))
        .await
        .map_err(|e| Error::Neo4j(e.to_string()))?;

    let mut stats = Vec::new();
    while let Some(row) = result.next().await.map_err(|e| Error::Neo4j(e.to_string()))? {
        let label: String = row.get("label").unwrap_or_default();
        let cnt: i64 = row.get("cnt").unwrap_or_default();
        stats.push((label, cnt));
    }

    Ok(stats)
}
