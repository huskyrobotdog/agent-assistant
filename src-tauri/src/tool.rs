use anyhow::Result;
use serde::{Deserialize, Serialize};

/// MCP 工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// MCP 工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// MCP 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub result: String,
    pub is_error: bool,
}

/// MCP 工具执行器 trait
pub trait McpToolExecutor: Send + Sync {
    fn execute(&self, tool_call: &ToolCall) -> Result<ToolResult>;
    fn get_tools(&self) -> Vec<McpTool>;
}

/// 内置的 Echo 工具执行器（用于测试）
pub struct EchoToolExecutor;

impl McpToolExecutor for EchoToolExecutor {
    fn execute(&self, tool_call: &ToolCall) -> Result<ToolResult> {
        Ok(ToolResult {
            tool_name: tool_call.name.clone(),
            result: format!(
                "Echo: {}",
                serde_json::to_string(&tool_call.arguments).unwrap()
            ),
            is_error: false,
        })
    }

    fn get_tools(&self) -> Vec<McpTool> {
        vec![McpTool {
            name: "echo".to_string(),
            description: "回显输入的内容".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "要回显的消息"
                    }
                },
                "required": ["message"]
            }),
        }]
    }
}
