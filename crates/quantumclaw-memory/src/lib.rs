use async_trait::async_trait;
use quantumclaw_core::{MemoryStore, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub text: String,
    pub tags: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}

impl MemoryRecord {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            tags: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct WorkingMemory {
    records: Arc<RwLock<Vec<MemoryRecord>>>,
}

#[derive(Debug, Default, Clone)]
pub struct ShortTermMemory {
    records: Arc<RwLock<Vec<MemoryRecord>>>,
}

#[derive(Debug, Default, Clone)]
pub struct EpisodicMemory {
    records: Arc<RwLock<Vec<MemoryRecord>>>,
}

#[derive(Debug, Default, Clone)]
pub struct SemanticMemory {
    records: Arc<RwLock<Vec<MemoryRecord>>>,
}

#[derive(Debug, Default, Clone)]
pub struct ProceduralMemoryTier {
    records: Arc<RwLock<Vec<StoredProcedure>>>,
}

impl WorkingMemory {
    pub fn push(&self, record: MemoryRecord) {
        self.records
            .write()
            .expect("working memory lock")
            .push(record);
    }
    pub fn all(&self) -> Vec<MemoryRecord> {
        self.records.read().expect("working memory lock").clone()
    }
}

impl ShortTermMemory {
    pub fn push(&self, record: MemoryRecord) {
        self.records
            .write()
            .expect("short term memory lock")
            .push(record);
    }
    pub fn recent(&self, limit: usize) -> Vec<MemoryRecord> {
        self.records
            .read()
            .expect("short term memory lock")
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}

impl EpisodicMemory {
    pub fn push(&self, record: MemoryRecord) {
        self.records
            .write()
            .expect("episodic memory lock")
            .push(record);
    }
    pub fn all(&self) -> Vec<MemoryRecord> {
        self.records.read().expect("episodic memory lock").clone()
    }
}

impl SemanticMemory {
    pub fn push(&self, record: MemoryRecord) {
        self.records
            .write()
            .expect("semantic memory lock")
            .push(record);
    }
    pub fn query(&self, query: &str, limit: usize) -> Vec<MemoryRecord> {
        rank_records(
            &self.records.read().expect("semantic memory lock"),
            query,
            limit,
        )
    }
}

impl ProceduralMemoryTier {
    pub fn push(&self, procedure: StoredProcedure) {
        self.records
            .write()
            .expect("procedural memory lock")
            .push(procedure);
    }
    pub fn all(&self) -> Vec<StoredProcedure> {
        self.records.read().expect("procedural memory lock").clone()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredProcedure {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub keywords: Vec<String>,
    pub template: String,
    pub metadata: BTreeMap<String, String>,
}

impl StoredProcedure {
    pub fn new<I, S>(id: impl Into<String>, summary: impl Into<String>, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let id = id.into();
        Self {
            title: id.clone(),
            id,
            summary: summary.into(),
            keywords: keywords.into_iter().map(Into::into).collect(),
            template: String::new(),
            metadata: BTreeMap::new(),
        }
    }
}

#[async_trait]
pub trait ProceduralMemory: Send + Sync {
    async fn store_procedure(&self, procedure: StoredProcedure) -> Result<()>;
    async fn retrieve_similar(&self, query: &str, limit: usize) -> Result<Vec<StoredProcedure>>;
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryProceduralMemory {
    records: Arc<RwLock<Vec<StoredProcedure>>>,
}

#[async_trait]
impl ProceduralMemory for InMemoryProceduralMemory {
    async fn store_procedure(&self, procedure: StoredProcedure) -> Result<()> {
        self.records
            .write()
            .expect("procedural memory lock")
            .push(procedure);
        Ok(())
    }

    async fn retrieve_similar(&self, query: &str, limit: usize) -> Result<Vec<StoredProcedure>> {
        let records = self.records.read().expect("procedural memory lock");
        let mut scored = records
            .iter()
            .cloned()
            .map(|record| (procedure_score(&record, query), record))
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(scored
            .into_iter()
            .filter(|(score, _)| *score > 0)
            .take(limit)
            .map(|(_, record)| record)
            .collect())
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryMemoryStore {
    records: Arc<RwLock<Vec<Value>>>,
}

#[async_trait]
impl MemoryStore for InMemoryMemoryStore {
    type Record = Value;

    async fn put(&self, record: Self::Record) -> Result<()> {
        self.records
            .write()
            .expect("memory store lock")
            .push(record);
        Ok(())
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<Self::Record>> {
        let needle = query.to_lowercase();
        Ok(self
            .records
            .read()
            .expect("memory store lock")
            .iter()
            .filter(|record| record.to_string().to_lowercase().contains(&needle))
            .take(limit)
            .cloned()
            .collect())
    }
}

fn rank_records(records: &[MemoryRecord], query: &str, limit: usize) -> Vec<MemoryRecord> {
    let query = query.to_lowercase();
    let mut scored = records
        .iter()
        .cloned()
        .map(|record| {
            let haystack =
                format!("{} {} {:?}", record.id, record.text, record.tags).to_lowercase();
            let score = query
                .split_whitespace()
                .filter(|token| haystack.contains(token))
                .count();
            (score, record)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .filter(|(score, _)| *score > 0)
        .take(limit)
        .map(|(_, record)| record)
        .collect()
}

fn procedure_score(record: &StoredProcedure, query: &str) -> usize {
    let query = query.to_lowercase();
    let haystack = format!(
        "{} {} {} {:?}",
        record.id, record.title, record.summary, record.keywords
    )
    .to_lowercase();
    let keyword_score = record
        .keywords
        .iter()
        .filter(|keyword| query.contains(&keyword.to_lowercase()))
        .count()
        * 3;
    let text_score = query
        .split_whitespace()
        .filter(|token| haystack.contains(token))
        .count();
    keyword_score + text_score
}
