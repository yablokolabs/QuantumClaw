use crate::quantumclaw_core::{Result, SkillStore};
use crate::quantumclaw_memory::{ProceduralMemory, StoredProcedure};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub template: SkillTemplate,
    pub recipe: PlanningRecipe,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillTemplate {
    pub variables: Vec<String>,
    pub steps: Vec<String>,
}

impl SkillTemplate {
    pub fn new(steps: Vec<String>) -> Self {
        Self {
            variables: Vec::new(),
            steps,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillExecutionRecord {
    pub task: String,
    pub plan_summary: String,
    pub outcome: String,
    pub why_it_worked: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanningRecipe {
    pub task_pattern: String,
    pub recommended_mode: String,
    pub tool_sequence: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkillLearningPipeline<M> {
    pub procedural_memory: M,
}

impl<M> SkillLearningPipeline<M>
where
    M: ProceduralMemory + Clone,
{
    pub fn new(procedural_memory: M) -> Self {
        Self { procedural_memory }
    }

    pub async fn learn_from_success(&self, record: SkillExecutionRecord) -> Result<Skill> {
        let skill = self.capture_successful_execution(&record);
        let procedure = StoredProcedure {
            id: skill.id.clone(),
            title: skill.title.clone(),
            summary: format!(
                "{} Why it worked: {}",
                skill.description, record.why_it_worked
            ),
            keywords: skill.tags.clone(),
            template: skill.template.steps.join("\n"),
            metadata: BTreeMap::new(),
        };
        self.procedural_memory.store_procedure(procedure).await?;
        Ok(skill)
    }

    pub fn capture_successful_execution(&self, record: &SkillExecutionRecord) -> Skill {
        let id = stable_skill_id(&record.task);
        Skill {
            id: id.clone(),
            title: format!("Reusable procedure: {}", record.task),
            description: summarize_success(record),
            tags: record.tags.clone(),
            template: SkillTemplate::new(vec![
                "Retrieve relevant prior procedures".into(),
                "Encode the task into backend-neutral decision IR".into(),
                "Select a solver backend under policy and latency constraints".into(),
                "Execute or simulate with policy-controlled tools".into(),
                "Validate deterministic outcomes and capture learning".into(),
            ]),
            recipe: PlanningRecipe {
                task_pattern: record.task.clone(),
                recommended_mode: "Auto".into(),
                tool_sequence: vec![
                    "filesystem".into(),
                    "code_edit".into(),
                    "shell".into(),
                    "memory".into(),
                ],
            },
            metadata: BTreeMap::new(),
        }
    }
}

#[async_trait]
pub trait SkillRetriever: Send + Sync {
    async fn retrieve(&self, query: &str, limit: usize) -> Result<Vec<Skill>>;
}

#[derive(Debug, Clone)]
pub struct KeywordSkillRetriever<M> {
    pub procedural_memory: M,
}

impl<M> KeywordSkillRetriever<M> {
    pub fn new(procedural_memory: M) -> Self {
        Self { procedural_memory }
    }
}

#[async_trait]
impl<M> SkillRetriever for KeywordSkillRetriever<M>
where
    M: ProceduralMemory + Clone,
{
    async fn retrieve(&self, query: &str, limit: usize) -> Result<Vec<Skill>> {
        let procedures = self
            .procedural_memory
            .retrieve_similar(query, limit)
            .await?;
        Ok(procedures
            .into_iter()
            .map(|procedure| Skill {
                id: procedure.id,
                title: procedure.title,
                description: procedure.summary,
                tags: procedure.keywords,
                template: SkillTemplate::new(
                    procedure.template.lines().map(str::to_string).collect(),
                ),
                recipe: PlanningRecipe {
                    task_pattern: query.into(),
                    recommended_mode: "Auto".into(),
                    tool_sequence: Vec::new(),
                },
                metadata: procedure.metadata,
            })
            .collect())
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemorySkillStore {
    skills: Arc<RwLock<HashMap<String, Skill>>>,
}

#[async_trait]
impl SkillStore for InMemorySkillStore {
    type Skill = Skill;

    async fn save_skill(&self, skill: Self::Skill) -> Result<()> {
        self.skills
            .write()
            .expect("skill store lock")
            .insert(skill.id.clone(), skill);
        Ok(())
    }

    async fn find_skills(&self, query: &str, limit: usize) -> Result<Vec<Self::Skill>> {
        let query = query.to_lowercase();
        Ok(self
            .skills
            .read()
            .expect("skill store lock")
            .values()
            .filter(|skill| {
                format!("{} {} {:?}", skill.title, skill.description, skill.tags)
                    .to_lowercase()
                    .contains(&query)
            })
            .take(limit)
            .cloned()
            .collect())
    }
}

fn summarize_success(record: &SkillExecutionRecord) -> String {
    format!(
        "{} Outcome: {}. Reuse because {}",
        record.plan_summary, record.outcome, record.why_it_worked
    )
}

fn stable_skill_id(task: &str) -> String {
    let slug = task
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("skill-{}", slug.chars().take(48).collect::<String>())
}
