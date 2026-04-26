use std::collections::HashMap;

use crate::entity::CodeEntity;
use crate::relationship::Relationship;

/// In-memory representation of the code graph before persisting to Neo4j.
#[derive(Debug, Default)]
pub struct CodeGraph {
    /// Entities keyed by qualified_name
    pub entities: HashMap<String, CodeEntity>,
    /// All relationships
    pub relationships: Vec<Relationship>,
}

impl CodeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entity. If an entity with the same qualified_name exists, it is replaced.
    pub fn add_entity(&mut self, entity: CodeEntity) {
        self.entities.insert(entity.qualified_name.clone(), entity);
    }

    /// Add a relationship.
    pub fn add_relationship(&mut self, rel: Relationship) {
        self.relationships.push(rel);
    }

    /// Merge another graph into this one.
    pub fn merge(&mut self, other: CodeGraph) {
        for (k, v) in other.entities {
            self.entities.insert(k, v);
        }
        self.relationships.extend(other.relationships);
    }

    /// Get all entities as a Vec (consuming the map).
    pub fn into_entities(self) -> Vec<CodeEntity> {
        self.entities.into_values().collect()
    }

    /// Get all source hashes currently in the graph.
    pub fn source_hashes(&self) -> Vec<String> {
        self.entities.values().map(|e| e.source_hash.clone()).collect()
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn relationship_count(&self) -> usize {
        self.relationships.len()
    }
}
