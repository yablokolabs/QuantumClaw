use async_trait::async_trait;
use quantumclaw_assistant::{
    AssistantConfig, AssistantEvent, AssistantProvider, AssistantReply, AssistantRequest,
    AssistantRunner, AssistantUsage, RequestedToolCall, WorkspaceTools,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Mutex;
use tempfile::tempdir;

#[derive(Debug)]
struct ScriptedProvider {
    replies: Mutex<VecDeque<AssistantReply>>,
    requests: Mutex<Vec<AssistantRequest>>,
}

impl ScriptedProvider {
    fn new(replies: Vec<AssistantReply>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AssistantProvider for ScriptedProvider {
    async fn complete(&self, request: AssistantRequest) -> anyhow::Result<AssistantReply> {
        self.requests.lock().unwrap().push(request);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("scripted provider exhausted"))
    }
}

fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> AssistantReply {
    AssistantReply {
        content: None,
        tool_calls: vec![RequestedToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        }],
        usage: None,
    }
}

#[tokio::test]
async fn workspace_tools_write_read_and_list_real_files() {
    let dir = tempdir().unwrap();
    let tools = WorkspaceTools::new(dir.path()).unwrap();

    let written = tools
        .execute(
            "write_file",
            json!({"path":"src/lib.rs","content":"pub fn answer() -> u8 { 42 }\n"}),
        )
        .await
        .unwrap();
    assert!(written.success);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "pub fn answer() -> u8 { 42 }\n"
    );

    let read = tools
        .execute("read_file", json!({"path":"src/lib.rs"}))
        .await
        .unwrap();
    assert!(read.success);
    assert!(read.output.contains("pub fn answer"));

    let listed = tools
        .execute("list_files", json!({"path":"."}))
        .await
        .unwrap();
    assert!(listed.output.contains("src/lib.rs"));
}

#[tokio::test]
async fn workspace_rejects_paths_outside_root() {
    let dir = tempdir().unwrap();
    let tools = WorkspaceTools::new(dir.path()).unwrap();

    for path in ["../escape.txt", "/tmp/escape.txt"] {
        let error = tools
            .execute("write_file", json!({"path":path,"content":"no"}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("workspace"));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_rejects_symlink_escape() {
    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("outside-link")).unwrap();
    let tools = WorkspaceTools::new(root.path()).unwrap();

    let error = tools
        .execute(
            "write_file",
            json!({"path":"outside-link/pwned.txt","content":"no"}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("workspace"));
    assert!(!outside.path().join("pwned.txt").exists());
}

#[tokio::test]
async fn command_tool_is_allowlisted_and_shell_free() {
    let dir = tempdir().unwrap();
    let tools = WorkspaceTools::new(dir.path()).unwrap();

    let cargo = tools
        .execute(
            "run_command",
            json!({"program":"cargo","args":["--version"],"timeout_secs":30}),
        )
        .await
        .unwrap();
    assert!(cargo.success);
    assert!(cargo.output.contains("cargo"));

    let denied = tools
        .execute(
            "run_command",
            json!({"program":"sh","args":["-c","touch escaped"]}),
        )
        .await
        .unwrap_err();
    assert!(denied.to_string().contains("not allowed"));
    assert!(!dir.path().join("escaped").exists());
}

#[tokio::test]
async fn runner_executes_tool_calls_and_finishes() {
    let dir = tempdir().unwrap();
    let provider = ScriptedProvider::new(vec![
        tool_call(
            "call-write",
            "write_file",
            json!({"path":"RESULT.md","content":"real tool output\n"}),
        ),
        tool_call("call-read", "read_file", json!({"path":"RESULT.md"})),
        tool_call(
            "call-finish",
            "finish",
            json!({"summary":"verified complete"}),
        ),
    ]);
    let runner = AssistantRunner::new(
        provider,
        WorkspaceTools::new(dir.path()).unwrap(),
        AssistantConfig {
            max_turns: 5,
            ..AssistantConfig::default()
        },
    );

    let report = runner
        .run(
            "Create and verify RESULT.md",
            "one safe write, then read it",
        )
        .await
        .unwrap();

    assert_eq!(report.turns, 3);
    assert_eq!(report.summary, "verified complete");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("RESULT.md")).unwrap(),
        "real tool output\n"
    );
    assert!(report.tool_results.iter().all(|result| result.success));
}

#[tokio::test]
async fn runner_stops_at_turn_limit() {
    let dir = tempdir().unwrap();
    let provider = ScriptedProvider::new(vec![
        AssistantReply::text("still thinking"),
        AssistantReply::text("still thinking"),
    ]);
    let runner = AssistantRunner::new(
        provider,
        WorkspaceTools::new(dir.path()).unwrap(),
        AssistantConfig {
            max_turns: 2,
            ..AssistantConfig::default()
        },
    );

    let error = runner.run("never finish", "bounded").await.unwrap_err();
    assert!(error.to_string().contains("turn limit"));
}

#[test]
fn sarvam_request_uses_string_messages_and_openai_tools() {
    let body = quantumclaw_assistant::sarvam_request_body(
        "sarvam-105b",
        &[quantumclaw_assistant::AssistantMessage::user("build it")],
        &WorkspaceTools::definitions(),
        4096,
    );

    assert_eq!(body["model"], "sarvam-105b");
    assert!(body["messages"][0]["content"].is_string());
    assert_eq!(body["tool_choice"], "auto");
    assert!(body["tools"].as_array().unwrap().len() >= 5);
    assert_eq!(body["stream"], false);
}

#[tokio::test]
async fn quantum_plan_uses_qinspired_backend_and_policy() {
    let planned = quantumclaw_assistant::quantum_plan_context(
        "Build and verify a Rust library from a mathematical paper",
    )
    .await
    .unwrap();

    assert_eq!(planned.plan.backend, "quantum-inspired-hybrid");
    assert!(planned.policy.allowed);
    assert!(!planned.policy.required_confirmation);
    assert!(!planned.plan.steps.is_empty());
}

#[tokio::test]
async fn observed_run_emits_model_tool_and_finish_events() {
    let dir = tempdir().unwrap();
    let provider = ScriptedProvider::new(vec![
        tool_call(
            "call-write",
            "write_file",
            json!({"path":"EVENT.md","content":"event\n"}),
        ),
        tool_call("call-finish", "finish", json!({"summary":"event verified"})),
    ]);
    let runner = AssistantRunner::new(
        provider,
        WorkspaceTools::new(dir.path()).unwrap(),
        AssistantConfig {
            max_turns: 3,
            ..AssistantConfig::default()
        },
    );
    let mut events = Vec::new();

    let report = runner
        .run_observed("Create EVENT.md", "bounded", |event| {
            events.push(event.clone())
        })
        .await
        .unwrap();

    assert_eq!(report.summary, "event verified");
    assert!(matches!(
        events.first(),
        Some(AssistantEvent::ModelReply { turn: 1, .. })
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        AssistantEvent::ToolResult { result, .. } if result.tool_name == "write_file" && result.success
    )));
    assert!(matches!(
        events.last(),
        Some(AssistantEvent::Finished { turn: 2, summary }) if summary == "event verified"
    ));
}

#[tokio::test]
async fn runner_enforces_cumulative_token_budget_before_tools() {
    let dir = tempdir().unwrap();
    let provider = ScriptedProvider::new(vec![AssistantReply {
        content: None,
        tool_calls: vec![RequestedToolCall {
            id: "call-finish".into(),
            name: "finish".into(),
            arguments: json!({"summary":"must not finish over budget"}),
        }],
        usage: Some(AssistantUsage {
            prompt_tokens: 90,
            completion_tokens: 11,
            total_tokens: 101,
        }),
    }]);
    let runner = AssistantRunner::new(
        provider,
        WorkspaceTools::new(dir.path()).unwrap(),
        AssistantConfig {
            max_turns: 2,
            max_total_tokens: 100,
        },
    );

    let error = runner.run("bounded", "bounded").await.unwrap_err();
    assert!(error.to_string().contains("token budget"));
}
