use async_trait::async_trait;
use quantumclaw_core::{CoreToolCall, CoreToolResult, Result};
pub use quantumclaw_core::{Tool, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub type ToolCall = CoreToolCall;
pub type ToolResult = CoreToolResult;

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

#[derive(Default, Clone)]
pub struct InMemoryToolRegistry {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
}

impl InMemoryToolRegistry {
    pub fn with_default_tools() -> Self {
        let registry = Self::default();
        registry.insert_sync(Arc::new(ShellTool));
        registry.insert_sync(Arc::new(FilesystemTool));
        registry.insert_sync(Arc::new(HttpTool));
        registry.insert_sync(Arc::new(SearchTool));
        registry.insert_sync(Arc::new(CodeEditTool));
        registry.insert_sync(Arc::new(SchedulerTool));
        registry.insert_sync(Arc::new(MemoryTool));
        registry.insert_sync(Arc::new(ExternalApiTool));
        registry
    }

    fn insert_sync(&self, tool: Arc<dyn Tool>) {
        self.tools
            .write()
            .expect("tool registry lock")
            .insert(tool.name().into(), tool);
    }
}

#[async_trait]
impl ToolRegistry for InMemoryToolRegistry {
    async fn register(&self, tool: Arc<dyn Tool>) -> Result<()> {
        self.insert_sync(tool);
        Ok(())
    }

    async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .read()
            .expect("tool registry lock")
            .get(name)
            .cloned()
    }

    async fn list(&self) -> Vec<String> {
        self.tools
            .read()
            .expect("tool registry lock")
            .keys()
            .cloned()
            .collect()
    }
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
        impl Tool for $name {
            fn name(&self) -> &'static str { $tool_name }
            fn description(&self) -> &'static str { $desc }

            async fn call(&self, call: CoreToolCall) -> Result<CoreToolResult> {
                Ok(CoreToolResult::simulated(format!("{} stub simulated action '{}'", $tool_name, call.action)))
            }
        }
    };
}

define_stub_tool!(
    ShellTool,
    "shell",
    "Policy-controlled shell execution stub",
    "tool.shell.execute",
    "high"
);
define_stub_tool!(
    FilesystemTool,
    "filesystem",
    "Policy-controlled file read/write stub",
    "tool.filesystem.access",
    "medium"
);
define_stub_tool!(
    HttpTool,
    "http",
    "Policy-controlled HTTP request stub",
    "tool.http.request",
    "medium"
);
define_stub_tool!(
    SearchTool,
    "search",
    "Policy-controlled search stub",
    "tool.search.query",
    "low"
);
define_stub_tool!(
    CodeEditTool,
    "code_edit",
    "Policy-controlled code edit stub",
    "tool.code_edit.modify",
    "high"
);
define_stub_tool!(
    SchedulerTool,
    "scheduler",
    "Policy-controlled scheduler stub",
    "tool.scheduler.manage",
    "medium"
);
define_stub_tool!(
    MemoryTool,
    "memory",
    "Policy-controlled memory access stub",
    "tool.memory.access",
    "medium"
);
define_stub_tool!(
    ExternalApiTool,
    "external_api",
    "Policy-controlled external API stub",
    "tool.external_api.call",
    "medium"
);
