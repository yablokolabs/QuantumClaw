use async_trait::async_trait;
use quantumclaw_core::{CoreToolCall, CoreToolResult, QuantumClawError, Result};
pub use quantumclaw_core::{Tool, ToolRegistry};
use quantumclaw_core::{Tool as CoreTool, ToolRegistry as CoreToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use zeroclaw::tools::{Tool as ZeroClawTool, ToolResult as ZeroClawToolResult};

pub type ToolCall = CoreToolCall;
pub type ToolResult = CoreToolResult;
pub type ZeroClawToolRef = Arc<dyn ZeroClawTool>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub permissions: Vec<ToolPermission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPermission {
    pub permission: String,
    pub risk_level: String,
}

impl ToolPermission {
    pub fn new(permission: impl Into<String>, risk_level: impl Into<String>) -> Self {
        Self {
            permission: permission.into(),
            risk_level: risk_level.into(),
        }
    }
}

#[derive(Clone)]
pub struct ZeroClawToolAdapter {
    inner: ZeroClawToolRef,
}

impl ZeroClawToolAdapter {
    pub fn new(inner: ZeroClawToolRef) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> ZeroClawToolRef {
        self.inner.clone()
    }
}

#[async_trait]
impl CoreTool for ZeroClawToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult> {
        let args = core_call_to_zeroclaw_args(call);
        let result = self
            .inner
            .execute(args)
            .await
            .map_err(|error| QuantumClawError::new(error.to_string()))?;

        Ok(CoreToolResult {
            success: result.success,
            output: decode_tool_output(result.output),
            metadata: result
                .error
                .map(|error| BTreeMap::from([("zeroclaw_error".into(), error)]))
                .unwrap_or_default(),
        })
    }
}

/// ZeroClaw tool output is text. Tools that produce structured results encode
/// them as JSON, so parse that back rather than handing callers a string that
/// merely looks like an object.
fn decode_tool_output(output: String) -> Value {
    match serde_json::from_str::<Value>(&output) {
        Ok(value) if value.is_object() || value.is_array() => value,
        _ => Value::String(output),
    }
}

#[derive(Clone)]
pub struct QuantumClawToolAdapter {
    inner: Arc<dyn CoreTool>,
}

impl QuantumClawToolAdapter {
    pub fn new(inner: Arc<dyn CoreTool>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ZeroClawTool for QuantumClawToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "additionalProperties": true })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ZeroClawToolResult> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_else(|| self.inner.name())
            .to_string();
        let input = args.get("input").cloned().unwrap_or(args);
        let mut call = CoreToolCall::new(self.inner.name(), action);
        call.input = input;

        let result = self
            .inner
            .call(call)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(ZeroClawToolResult {
            success: result.success,
            output: result.output.to_string(),
            error: if result.success {
                None
            } else {
                Some(result.output.to_string())
            },
        })
    }
}

#[derive(Default, Clone)]
pub struct InMemoryToolRegistry {
    tools: Arc<RwLock<HashMap<String, ZeroClawToolRef>>>,
}

impl InMemoryToolRegistry {
    pub fn with_default_tools() -> Self {
        let registry = Self::default();
        registry.insert_zeroclaw_sync(Arc::new(ShellTool));
        registry.insert_zeroclaw_sync(Arc::new(FilesystemTool));
        registry.insert_zeroclaw_sync(Arc::new(HttpTool));
        registry.insert_zeroclaw_sync(Arc::new(SearchTool));
        registry.insert_zeroclaw_sync(Arc::new(CodeEditTool));
        registry.insert_zeroclaw_sync(Arc::new(SchedulerTool));
        registry.insert_zeroclaw_sync(Arc::new(MemoryTool));
        registry.insert_zeroclaw_sync(Arc::new(ExternalApiTool));
        registry
    }

    fn insert_zeroclaw_sync(&self, tool: ZeroClawToolRef) {
        self.tools
            .write()
            .expect("tool registry lock")
            .insert(tool.name().into(), tool);
    }

    pub async fn register_zeroclaw(&self, tool: ZeroClawToolRef) -> Result<()> {
        self.insert_zeroclaw_sync(tool);
        Ok(())
    }

    pub async fn get_zeroclaw(&self, name: &str) -> Option<ZeroClawToolRef> {
        self.tools
            .read()
            .expect("tool registry lock")
            .get(name)
            .cloned()
    }

    pub async fn list_zeroclaw(&self) -> Vec<String> {
        self.tools
            .read()
            .expect("tool registry lock")
            .keys()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl CoreToolRegistry for InMemoryToolRegistry {
    async fn register(&self, tool: Arc<dyn CoreTool>) -> Result<()> {
        self.insert_zeroclaw_sync(Arc::new(QuantumClawToolAdapter::new(tool)));
        Ok(())
    }

    async fn get(&self, name: &str) -> Option<Arc<dyn CoreTool>> {
        self.get_zeroclaw(name)
            .await
            .map(|tool| Arc::new(ZeroClawToolAdapter::new(tool)) as Arc<dyn CoreTool>)
    }

    async fn list(&self) -> Vec<String> {
        self.list_zeroclaw().await
    }
}

fn core_call_to_zeroclaw_args(call: CoreToolCall) -> serde_json::Value {
    json!({
        "action": call.action,
        "input": call.input,
        "metadata": call.metadata,
    })
}

fn action_from_args(args: &serde_json::Value, fallback: &str) -> String {
    args.get("action")
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

macro_rules! define_stub_tool {
    ($name:ident, $tool_name:literal, $desc:literal, $permission:literal, $risk:literal) => {
        #[derive(Debug, Default, Clone)]
        pub struct $name;

        impl $name {
            pub fn schema() -> ToolSchema {
                ToolSchema {
                    name: $tool_name.into(),
                    description: $desc.into(),
                    input_schema: json!({ "type": "object", "additionalProperties": true }),
                    permissions: vec![ToolPermission::new($permission, $risk)],
                }
            }
        }

        #[async_trait]
        impl ZeroClawTool for $name {
            fn name(&self) -> &str {
                $tool_name
            }

            fn description(&self) -> &str {
                $desc
            }

            fn parameters_schema(&self) -> serde_json::Value {
                Self::schema().input_schema
            }

            async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ZeroClawToolResult> {
                let action = action_from_args(&args, "execute");
                Ok(ZeroClawToolResult {
                    success: true,
                    output: format!(
                        "ZeroClaw {} tool simulated action '{}' for QuantumClaw",
                        $tool_name, action
                    ),
                    error: None,
                })
            }
        }

        #[async_trait]
        impl CoreTool for $name {
            fn name(&self) -> &str {
                $tool_name
            }

            fn description(&self) -> &str {
                $desc
            }

            async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult> {
                let result = <Self as ZeroClawTool>::execute(self, core_call_to_zeroclaw_args(call))
                    .await
                    .map_err(|error| QuantumClawError::new(error.to_string()))?;
                Ok(CoreToolResult {
                    success: result.success,
                    output: Value::String(result.output),
                    metadata: result
                        .error
                        .map(|error| BTreeMap::from([("zeroclaw_error".into(), error)]))
                        .unwrap_or_default(),
                })
            }
        }
    };
}

define_stub_tool!(
    ShellTool,
    "shell",
    "ZeroClaw-backed policy-controlled shell execution stub",
    "tool.shell.execute",
    "high"
);
define_stub_tool!(
    FilesystemTool,
    "filesystem",
    "ZeroClaw-backed policy-controlled file read/write stub",
    "tool.filesystem.access",
    "medium"
);
define_stub_tool!(
    HttpTool,
    "http",
    "ZeroClaw-backed policy-controlled HTTP request stub",
    "tool.http.request",
    "medium"
);
define_stub_tool!(
    SearchTool,
    "search",
    "ZeroClaw-backed policy-controlled search stub",
    "tool.search.query",
    "low"
);
define_stub_tool!(
    CodeEditTool,
    "code_edit",
    "ZeroClaw-backed policy-controlled code edit stub",
    "tool.code_edit.modify",
    "high"
);
define_stub_tool!(
    SchedulerTool,
    "scheduler",
    "ZeroClaw-backed policy-controlled scheduler stub",
    "tool.scheduler.manage",
    "medium"
);
define_stub_tool!(
    MemoryTool,
    "memory",
    "ZeroClaw-backed policy-controlled memory access stub",
    "tool.memory.access",
    "medium"
);
define_stub_tool!(
    ExternalApiTool,
    "external_api",
    "ZeroClaw-backed policy-controlled external API stub",
    "tool.external_api.call",
    "medium"
);
