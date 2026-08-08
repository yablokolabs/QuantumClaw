use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use quantumclaw_core::{AgentTask, SolverBackend};
use quantumclaw_planner::{HybridPlanner, Plan, PlannerMode, PlannerRequest, PlannerTelemetry};
use quantumclaw_policy::{DeterministicPolicyEngine, PolicyDecision};
use quantumclaw_solvers_classical::GreedySolver;
use quantumclaw_solvers_qinspired::QuantumInspiredSolver;

const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_LISTED_FILES: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RequestedToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl AssistantMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain("system", content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::plain("user", content)
    }

    pub fn assistant(content: Option<String>, tool_calls: Vec<RequestedToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    fn plain(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantRequest {
    pub messages: Vec<AssistantMessage>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantReply {
    pub content: Option<String>,
    pub tool_calls: Vec<RequestedToolCall>,
    pub usage: Option<AssistantUsage>,
}

impl AssistantReply {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            tool_calls: Vec::new(),
            usage: None,
        }
    }
}

#[async_trait]
pub trait AssistantProvider: Send + Sync {
    async fn complete(&self, request: AssistantRequest) -> Result<AssistantReply>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExecution {
    pub tool_name: String,
    pub tool_call_id: String,
    pub success: bool,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantReport {
    pub turns: usize,
    pub summary: String,
    pub tool_results: Vec<ToolExecution>,
    pub usage: AssistantUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AssistantEvent {
    ModelReply {
        turn: usize,
        content: Option<String>,
        tool_calls: Vec<RequestedToolCall>,
        usage: Option<AssistantUsage>,
    },
    ToolResult {
        turn: usize,
        result: ToolExecution,
    },
    Finished {
        turn: usize,
        summary: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssistantConfig {
    pub max_turns: usize,
    pub max_total_tokens: u64,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            max_turns: 30,
            max_total_tokens: 180_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantumPlanContext {
    pub plan: Plan,
    pub policy: PolicyDecision,
    pub telemetry: PlannerTelemetry,
}

pub async fn quantum_plan_context(task: &str) -> Result<QuantumPlanContext> {
    let backends: Vec<Arc<dyn SolverBackend>> = vec![
        Arc::new(GreedySolver),
        Arc::new(QuantumInspiredSolver::default()),
    ];
    let request = PlannerRequest::new(AgentTask::new(task))
        .with_mode(PlannerMode::QuantumInspiredPreferred)
        .with_backend(backends[0].clone())
        .with_backend(backends[1].clone());
    let response = HybridPlanner::default().plan(request).await?;
    let plan = response.primary_plan().clone();
    let policy = DeterministicPolicyEngine::default()
        .evaluate_plan(&plan)
        .await?;
    if !policy.allowed {
        bail!(
            "QuantumClaw policy denied plan: {}",
            policy.reasons.join("; ")
        );
    }
    if policy.required_confirmation {
        bail!(
            "QuantumClaw policy requires human confirmation: {}",
            policy.reasons.join("; ")
        );
    }
    Ok(QuantumPlanContext {
        plan,
        policy,
        telemetry: response.telemetry,
    })
}

#[derive(Debug, Clone)]
pub struct WorkspaceTools {
    root: PathBuf,
    allowed_programs: BTreeSet<String>,
}

impl WorkspaceTools {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = fs::canonicalize(root.as_ref())
            .with_context(|| format!("workspace does not exist: {}", root.as_ref().display()))?;
        if !root.is_dir() {
            bail!("workspace is not a directory: {}", root.display());
        }
        Ok(Self {
            root,
            allowed_programs: ["cargo", "rustc"].into_iter().map(String::from).collect(),
        })
    }

    pub fn definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "read_file".into(),
                description: "Read a UTF-8 text file inside the workspace.".into(),
                parameters: json!({
                    "type":"object",
                    "properties":{
                        "path":{"type":"string"},
                        "offset":{"type":"integer","minimum":1},
                        "limit":{"type":"integer","minimum":1,"maximum":2000}
                    },
                    "required":["path"],
                    "additionalProperties":false
                }),
            },
            ToolDefinition {
                name: "write_file".into(),
                description: "Create or completely replace a UTF-8 file inside the workspace.".into(),
                parameters: json!({
                    "type":"object",
                    "properties":{
                        "path":{"type":"string"},
                        "content":{"type":"string","maxLength":2097152}
                    },
                    "required":["path","content"],
                    "additionalProperties":false
                }),
            },
            ToolDefinition {
                name: "list_files".into(),
                description: "List files recursively under a workspace-relative directory.".into(),
                parameters: json!({
                    "type":"object",
                    "properties":{"path":{"type":"string"}},
                    "additionalProperties":false
                }),
            },
            ToolDefinition {
                name: "run_command".into(),
                description: "Run an allowlisted executable directly in the workspace without a shell. Allowed programs: cargo, rustc.".into(),
                parameters: json!({
                    "type":"object",
                    "properties":{
                        "program":{"type":"string","enum":["cargo","rustc"]},
                        "args":{"type":"array","items":{"type":"string"}},
                        "timeout_secs":{"type":"integer","minimum":1,"maximum":600}
                    },
                    "required":["program","args"],
                    "additionalProperties":false
                }),
            },
            ToolDefinition {
                name: "finish".into(),
                description: "Finish only after the requested artifact and all required verification gates are complete.".into(),
                parameters: json!({
                    "type":"object",
                    "properties":{"summary":{"type":"string"}},
                    "required":["summary"],
                    "additionalProperties":false
                }),
            },
        ]
    }

    pub async fn execute(&self, name: &str, arguments: Value) -> Result<ToolExecution> {
        let result = match name {
            "read_file" => self.read_file(&arguments),
            "write_file" => self.write_file(&arguments),
            "list_files" => self.list_files(&arguments),
            "run_command" => self.run_command(&arguments).await,
            "finish" => bail!("finish is handled by the assistant runner"),
            _ => bail!("unknown tool '{name}'"),
        }?;

        Ok(ToolExecution {
            tool_name: name.into(),
            tool_call_id: String::new(),
            success: result.0,
            output: truncate_output(result.1),
        })
    }

    fn read_file(&self, arguments: &Value) -> Result<(bool, String)> {
        let path = required_str(arguments, "path")?;
        let path = self.resolve_existing(path)?;
        let metadata = fs::metadata(&path)?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            bail!("file exceeds {MAX_FILE_BYTES} byte read limit");
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("file is not readable UTF-8: {}", path.display()))?;
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(1) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(2000)
            .min(2000) as usize;
        if offset == 0 {
            bail!("offset must be at least 1");
        }
        let selected = text
            .lines()
            .skip(offset - 1)
            .take(limit)
            .collect::<Vec<_>>()
            .join("\n");
        Ok((true, selected))
    }

    fn write_file(&self, arguments: &Value) -> Result<(bool, String)> {
        let relative = required_str(arguments, "path")?;
        let content = required_str(arguments, "content")?;
        if content.len() > MAX_FILE_BYTES {
            bail!("content exceeds {MAX_FILE_BYTES} byte write limit");
        }
        let path = self.resolve_for_write(relative)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;
        Ok((true, format!("wrote {} bytes to {relative}", content.len())))
    }

    fn list_files(&self, arguments: &Value) -> Result<(bool, String)> {
        let relative = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = self.resolve_existing(relative)?;
        if !path.is_dir() {
            bail!("workspace path is not a directory: {relative}");
        }
        let mut files = Vec::new();
        self.collect_files(&path, &mut files)?;
        files.sort();
        files.truncate(MAX_LISTED_FILES);
        Ok((true, files.join("\n")))
    }

    fn collect_files(&self, directory: &Path, files: &mut Vec<String>) -> Result<()> {
        if files.len() >= MAX_LISTED_FILES {
            return Ok(());
        }
        let mut entries = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if files.len() >= MAX_LISTED_FILES {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            if name == OsStr::new(".git") || name == OsStr::new("target") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                self.collect_files(&path, files)?;
            } else if metadata.is_file() {
                let relative = path.strip_prefix(&self.root)?;
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(())
    }

    async fn run_command(&self, arguments: &Value) -> Result<(bool, String)> {
        let program = required_str(arguments, "program")?;
        if !self.allowed_programs.contains(program) {
            bail!("program '{program}' is not allowed");
        }
        let args = arguments
            .get("args")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("'args' must be an array"))?
            .iter()
            .map(|arg| {
                arg.as_str()
                    .map(String::from)
                    .ok_or_else(|| anyhow!("command arguments must be strings"))
            })
            .collect::<Result<Vec<_>>>()?;
        validate_command(program, &args)?;
        let timeout_secs = arguments
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(300)
            .clamp(1, 600);

        let mut command = tokio::process::Command::new(program);
        command
            .args(&args)
            .current_dir(&self.root)
            .kill_on_drop(true)
            .env_clear();
        for key in [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "LANG",
            "LC_ALL",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "TMPDIR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("CARGO_TERM_COLOR", "never");

        let output = tokio::time::timeout(Duration::from_secs(timeout_secs), command.output())
            .await
            .map_err(|_| anyhow!("command timed out after {timeout_secs} seconds"))??;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        Ok((output.status.success(), text))
    }

    fn resolve_existing(&self, relative: &str) -> Result<PathBuf> {
        let relative = validate_relative_path(relative)?;
        let path = self.root.join(relative);
        let canonical = fs::canonicalize(&path)
            .with_context(|| format!("workspace path does not exist: {relative:?}"))?;
        self.ensure_in_workspace(canonical)
    }

    fn resolve_for_write(&self, relative: &str) -> Result<PathBuf> {
        let relative_path = validate_relative_path(relative)?;
        let mut current = self.root.clone();
        for component in relative_path.components() {
            if let Component::Normal(segment) = component {
                current.push(segment);
                if fs::symlink_metadata(&current).is_ok() {
                    let canonical = fs::canonicalize(&current).with_context(|| {
                        format!("workspace path cannot be resolved: {}", current.display())
                    })?;
                    self.ensure_in_workspace(canonical)?;
                }
            }
        }
        Ok(self.root.join(relative_path))
    }

    fn ensure_in_workspace(&self, path: PathBuf) -> Result<PathBuf> {
        if !path.starts_with(&self.root) {
            bail!("path escapes workspace: {}", path.display());
        }
        Ok(path)
    }
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("'{key}' must be a string"))
}

fn validate_relative_path(path: &str) -> Result<&Path> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("path must be workspace-relative");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => bail!("path escapes workspace"),
        }
    }
    Ok(path)
}

fn validate_command(program: &str, args: &[String]) -> Result<()> {
    if program == "cargo" {
        if let Some(command) = args.first().map(String::as_str) {
            if ["install", "login", "owner", "publish", "search", "yank"].contains(&command) {
                bail!("cargo subcommand '{command}' is not allowed");
            }
        }
    }
    for arg in args {
        if arg.contains('\0') || arg.contains("..") {
            bail!("command argument may escape workspace: {arg}");
        }
        let candidate = arg.split_once('=').map(|(_, value)| value).unwrap_or(arg);
        if Path::new(candidate).is_absolute() {
            bail!("absolute command path is not allowed: {arg}");
        }
    }
    Ok(())
}

fn truncate_output(mut output: String) -> String {
    if output.len() > MAX_TOOL_OUTPUT_BYTES {
        output.truncate(MAX_TOOL_OUTPUT_BYTES);
        output.push_str("\n[output truncated]");
    }
    output
}

pub struct AssistantRunner<P> {
    provider: P,
    tools: WorkspaceTools,
    config: AssistantConfig,
}

impl<P> AssistantRunner<P>
where
    P: AssistantProvider,
{
    pub fn new(provider: P, tools: WorkspaceTools, config: AssistantConfig) -> Self {
        Self {
            provider,
            tools,
            config,
        }
    }

    pub async fn run(&self, task: &str, plan_context: &str) -> Result<AssistantReport> {
        self.run_observed(task, plan_context, |_| {}).await
    }

    pub async fn run_observed<F>(
        &self,
        task: &str,
        plan_context: &str,
        mut observe: F,
    ) -> Result<AssistantReport>
    where
        F: FnMut(&AssistantEvent),
    {
        if self.config.max_turns == 0 {
            bail!("turn limit must be greater than zero");
        }
        let mut messages = vec![
            AssistantMessage::system(
                "You are QuantumClaw's bounded coding assistant. Work only through the provided tools. Inspect before editing, use exact workspace-relative paths, run required Cargo gates, recover from tool failures, and call finish only after verification. Batch independent tool calls in the same response and do not repeat a successful read, write, or check without a concrete reason. Never request or reveal credentials. Never claim an action happened unless its tool result confirms it.",
            ),
            AssistantMessage::user(format!(
                "TASK\n{task}\n\nQUANTUMCLAW PLAN CONTEXT\n{plan_context}"
            )),
        ];
        let definitions = WorkspaceTools::definitions();
        let mut tool_results = Vec::new();
        let mut usage = AssistantUsage::default();

        for turn in 1..=self.config.max_turns {
            let reply = self
                .provider
                .complete(AssistantRequest {
                    messages: messages.clone(),
                    tools: definitions.clone(),
                })
                .await?;
            observe(&AssistantEvent::ModelReply {
                turn,
                content: reply.content.clone(),
                tool_calls: reply.tool_calls.clone(),
                usage: reply.usage,
            });
            if let Some(reply_usage) = reply.usage {
                usage.prompt_tokens = usage
                    .prompt_tokens
                    .saturating_add(reply_usage.prompt_tokens);
                usage.completion_tokens = usage
                    .completion_tokens
                    .saturating_add(reply_usage.completion_tokens);
                usage.total_tokens = usage.total_tokens.saturating_add(reply_usage.total_tokens);
            }
            if usage.total_tokens > self.config.max_total_tokens {
                bail!(
                    "assistant exceeded cumulative token budget ({} > {})",
                    usage.total_tokens,
                    self.config.max_total_tokens
                );
            }
            messages.push(AssistantMessage::assistant(
                reply.content.clone(),
                reply.tool_calls.clone(),
            ));

            if reply.tool_calls.is_empty() {
                messages.push(AssistantMessage::user(
                    "No tool call was made. Continue with a provided tool or call finish if and only if verification is complete.",
                ));
                continue;
            }

            for call in reply.tool_calls {
                if call.name == "finish" {
                    let summary = required_str(&call.arguments, "summary")?.to_string();
                    observe(&AssistantEvent::Finished {
                        turn,
                        summary: summary.clone(),
                    });
                    return Ok(AssistantReport {
                        turns: turn,
                        summary,
                        tool_results,
                        usage,
                    });
                }

                let mut execution = match self.tools.execute(&call.name, call.arguments).await {
                    Ok(execution) => execution,
                    Err(error) => ToolExecution {
                        tool_name: call.name.clone(),
                        tool_call_id: call.id.clone(),
                        success: false,
                        output: error.to_string(),
                    },
                };
                execution.tool_call_id = call.id.clone();
                let observation = serde_json::to_string(&execution)?;
                messages.push(AssistantMessage::tool(call.id, observation));
                observe(&AssistantEvent::ToolResult {
                    turn,
                    result: execution.clone(),
                });
                tool_results.push(execution);
            }
        }

        bail!(
            "assistant reached turn limit ({}) without finishing",
            self.config.max_turns
        )
    }
}

pub fn sarvam_request_body(
    model: &str,
    messages: &[AssistantMessage],
    tools: &[ToolDefinition],
    max_tokens: u32,
) -> Value {
    let messages = messages.iter().map(sarvam_message).collect::<Vec<_>>();
    let tools = tools
        .iter()
        .map(|tool| {
            json!({
                "type":"function",
                "function":{
                    "name":tool.name,
                    "description":tool.description,
                    "parameters":tool.parameters
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "model":model,
        "messages":messages,
        "tools":tools,
        "tool_choice":"auto",
        "temperature":0.2,
        "max_tokens":max_tokens,
        "stream":false
    })
}

fn sarvam_message(message: &AssistantMessage) -> Value {
    let mut value = json!({"role":message.role});
    if let Some(content) = &message.content {
        value["content"] = Value::String(content.clone());
    } else if message.tool_calls.is_empty() {
        value["content"] = Value::String(String::new());
    }
    if !message.tool_calls.is_empty() {
        value["tool_calls"] = Value::Array(
            message
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id":call.id,
                        "type":"function",
                        "function":{
                            "name":call.name,
                            "arguments":serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into())
                        }
                    })
                })
                .collect(),
        );
    }
    if let Some(tool_call_id) = &message.tool_call_id {
        value["tool_call_id"] = Value::String(tool_call_id.clone());
    }
    value
}

#[derive(Debug, Clone)]
pub struct SarvamProvider {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
    model: String,
    max_tokens: u32,
    max_attempts: usize,
}

fn load_sarvam_api_key(
    environment_key: Option<String>,
    credentials_directory: Option<&Path>,
) -> Result<String> {
    let key = match environment_key {
        Some(key) => key,
        None => {
            let directory = credentials_directory
                .context("SARVAM_API_KEY is not set and no systemd credential directory exists")?;
            std::fs::read_to_string(directory.join("SARVAM_API_KEY"))
                .context("failed to read SARVAM_API_KEY systemd credential")?
        }
    };
    let key = key.trim_end_matches(['\r', '\n']).to_string();
    if key.trim().is_empty() {
        bail!("SARVAM_API_KEY is empty");
    }
    Ok(key)
}

impl SarvamProvider {
    pub fn from_env() -> Result<Self> {
        let credentials_directory = std::env::var_os("CREDENTIALS_DIRECTORY").map(PathBuf::from);
        let api_key = load_sarvam_api_key(
            std::env::var("SARVAM_API_KEY").ok(),
            credentials_directory.as_deref(),
        )?;
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(180))
                .build()?,
            api_key,
            endpoint: "https://api.sarvam.ai/v1/chat/completions".into(),
            model: "sarvam-105b".into(),
            max_tokens: 4096,
            max_attempts: 5,
        })
    }
}

#[async_trait]
impl AssistantProvider for SarvamProvider {
    async fn complete(&self, request: AssistantRequest) -> Result<AssistantReply> {
        let body = sarvam_request_body(
            &self.model,
            &request.messages,
            &request.tools,
            self.max_tokens,
        );
        for attempt in 1..=self.max_attempts {
            let response = self
                .client
                .post(&self.endpoint)
                .header("api-subscription-key", &self.api_key)
                .json(&body)
                .send()
                .await?;
            let status = response.status();
            let response_body = response.text().await?;
            if status.is_success() {
                return parse_sarvam_reply(&response_body);
            }
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < self.max_attempts {
                tokio::time::sleep(Duration::from_secs(1_u64 << (attempt - 1))).await;
                continue;
            }
            bail!(
                "Sarvam request failed with HTTP {}: {}",
                status.as_u16(),
                truncate_output(response_body)
            );
        }
        bail!("Sarvam request exhausted retries")
    }
}

fn parse_sarvam_reply(body: &str) -> Result<AssistantReply> {
    let value: Value = serde_json::from_str(body).context("invalid Sarvam JSON response")?;
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| anyhow!("Sarvam response contained no choices"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow!("Sarvam response contained no message"))?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .map(String::from);
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    let function = call
                        .get("function")
                        .ok_or_else(|| anyhow!("tool call omitted function"))?;
                    let raw_arguments = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("tool call arguments were not a string"))?;
                    Ok(RequestedToolCall {
                        id: call
                            .get("id")
                            .and_then(Value::as_str)
                            .ok_or_else(|| anyhow!("tool call omitted id"))?
                            .into(),
                        name: function
                            .get("name")
                            .and_then(Value::as_str)
                            .ok_or_else(|| anyhow!("tool call omitted name"))?
                            .into(),
                        arguments: serde_json::from_str(raw_arguments)
                            .context("tool call arguments were invalid JSON")?,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let usage = value.get("usage").map(|usage| AssistantUsage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    });
    Ok(AssistantReply {
        content,
        tool_calls,
        usage,
    })
}

#[cfg(test)]
mod credential_tests {
    use super::load_sarvam_api_key;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn loads_systemd_credential_when_environment_is_absent() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("SARVAM_API_KEY"),
            "credential-value\n",
        )
        .unwrap();

        let key = load_sarvam_api_key(None, Some(directory.path())).unwrap();

        assert_eq!(key, "credential-value");
    }

    #[test]
    fn environment_key_takes_precedence_over_systemd_credential() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("SARVAM_API_KEY"),
            "credential-value\n",
        )
        .unwrap();

        let key =
            load_sarvam_api_key(Some("environment-value".into()), Some(directory.path())).unwrap();

        assert_eq!(key, "environment-value");
    }
}
