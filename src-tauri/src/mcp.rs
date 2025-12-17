use crate::agent::{AgentError, McpTool, McpToolExecutor, ToolCall, ToolResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// JSON-RPC 请求
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

/// JSON-RPC 响应
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<u64>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

/// JSON-RPC 错误
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

/// MCP 工具信息
#[derive(Debug, Deserialize)]
struct McpToolInfo {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    input_schema: Option<serde_json::Value>,
}

/// MCP 工具列表响应
#[derive(Debug, Deserialize)]
struct ListToolsResponse {
    tools: Vec<McpToolInfo>,
}

/// MCP 工具调用响应
#[derive(Debug, Deserialize)]
struct CallToolResponse {
    content: Vec<ContentItem>,
    #[serde(default, rename = "isError")]
    is_error: bool,
}

/// MCP 内容项
#[derive(Debug, Deserialize)]
struct ContentItem {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
}

/// MCP 客户端配置
#[derive(Debug, Clone)]
pub struct McpClientConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// MCP 客户端 - 通过 stdio 与 MCP 服务器通信
pub struct McpClient {
    config: McpClientConfig,
    process: Mutex<Option<Child>>,
    request_id: Mutex<u64>,
    tools: Mutex<Vec<McpTool>>,
}

impl McpClient {
    pub fn new(config: McpClientConfig) -> Self {
        Self {
            config,
            process: Mutex::new(None),
            request_id: Mutex::new(0),
            tools: Mutex::new(Vec::new()),
        }
    }

    /// 启动 MCP 服务器进程
    pub async fn connect(&self) -> Result<(), AgentError> {
        let mut cmd = Command::new(&self.config.command);
        cmd.args(&self.config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        let child = cmd
            .spawn()
            .map_err(|e| AgentError::McpError(format!("启动 MCP 服务器失败: {}", e)))?;

        *self.process.lock().await = Some(child);

        self.initialize().await?;
        self.refresh_tools().await?;

        Ok(())
    }

    /// 发送 JSON-RPC 请求
    async fn send_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, AgentError> {
        let mut process_guard = self.process.lock().await;
        let process = process_guard
            .as_mut()
            .ok_or_else(|| AgentError::McpError("MCP 服务器未连接".to_string()))?;

        let mut id_guard = self.request_id.lock().await;
        *id_guard += 1;
        let id = *id_guard;
        drop(id_guard);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let request_json = serde_json::to_string(&request)
            .map_err(|e| AgentError::McpError(format!("序列化请求失败: {}", e)))?;

        let stdin = process
            .stdin
            .as_mut()
            .ok_or_else(|| AgentError::McpError("无法获取 stdin".to_string()))?;

        stdin
            .write_all(format!("{}\n", request_json).as_bytes())
            .await
            .map_err(|e| AgentError::McpError(format!("写入请求失败: {}", e)))?;

        stdin
            .flush()
            .await
            .map_err(|e| AgentError::McpError(format!("刷新 stdin 失败: {}", e)))?;

        let stdout = process
            .stdout
            .as_mut()
            .ok_or_else(|| AgentError::McpError("无法获取 stdout".to_string()))?;

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        reader
            .read_line(&mut line)
            .await
            .map_err(|e| AgentError::McpError(format!("读取响应失败: {}", e)))?;

        let response: JsonRpcResponse = serde_json::from_str(&line)
            .map_err(|e| AgentError::McpError(format!("解析响应失败: {}", e)))?;

        if let Some(error) = response.error {
            return Err(AgentError::McpError(format!(
                "MCP 错误 [{}]: {}",
                error.code, error.message
            )));
        }

        response
            .result
            .ok_or_else(|| AgentError::McpError("响应中没有结果".to_string()))
    }

    /// 初始化 MCP 连接
    async fn initialize(&self) -> Result<(), AgentError> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "clientInfo": {
                "name": "agent-assistant",
                "version": "0.1.0"
            }
        });

        self.send_request("initialize", Some(params)).await?;

        self.send_request("notifications/initialized", None)
            .await
            .ok();

        Ok(())
    }

    /// 刷新工具列表
    pub async fn refresh_tools(&self) -> Result<(), AgentError> {
        let result = self.send_request("tools/list", None).await?;

        let list_response: ListToolsResponse = serde_json::from_value(result)
            .map_err(|e| AgentError::McpError(format!("解析工具列表失败: {}", e)))?;

        let tools: Vec<McpTool> = list_response
            .tools
            .into_iter()
            .map(|t| McpTool {
                name: t.name,
                description: t.description.unwrap_or_default(),
                input_schema: t.input_schema.unwrap_or(serde_json::json!({})),
            })
            .collect();

        *self.tools.lock().await = tools;

        Ok(())
    }

    /// 调用工具
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult, AgentError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let result = self.send_request("tools/call", Some(params)).await?;

        let call_response: CallToolResponse = serde_json::from_value(result)
            .map_err(|e| AgentError::McpError(format!("解析工具调用结果失败: {}", e)))?;

        let result_text = call_response
            .content
            .iter()
            .filter_map(|c| {
                if c.content_type == "text" {
                    c.text.clone()
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult {
            tool_name: name.to_string(),
            result: result_text,
            is_error: call_response.is_error,
        })
    }

    /// 断开连接
    pub async fn disconnect(&self) -> Result<(), AgentError> {
        let mut process_guard = self.process.lock().await;
        if let Some(mut process) = process_guard.take() {
            process
                .kill()
                .await
                .map_err(|e| AgentError::McpError(format!("终止 MCP 服务器失败: {}", e)))?;
        }
        Ok(())
    }
}

/// MCP 工具执行器包装
pub struct McpToolExecutorWrapper {
    client: Arc<McpClient>,
}

impl McpToolExecutorWrapper {
    pub fn new(client: Arc<McpClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl McpToolExecutor for McpToolExecutorWrapper {
    async fn execute(&self, tool_call: &ToolCall) -> Result<ToolResult, AgentError> {
        self.client
            .call_tool(&tool_call.name, tool_call.arguments.clone())
            .await
    }

    fn get_tools(&self) -> Vec<McpTool> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { self.client.tools.lock().await.clone() })
        })
    }
}

/// MCP 服务器管理器 - 管理多个 MCP 服务器连接
pub struct McpManager {
    clients: Mutex<HashMap<String, Arc<McpClient>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// 添加 MCP 服务器
    pub async fn add_server(
        &self,
        name: &str,
        config: McpClientConfig,
    ) -> Result<Arc<McpClient>, AgentError> {
        let client = Arc::new(McpClient::new(config));
        client.connect().await?;

        let mut clients = self.clients.lock().await;
        clients.insert(name.to_string(), client.clone());

        Ok(client)
    }

    /// 获取 MCP 客户端
    pub async fn get_client(&self, name: &str) -> Option<Arc<McpClient>> {
        let clients = self.clients.lock().await;
        clients.get(name).cloned()
    }

    /// 移除 MCP 服务器
    pub async fn remove_server(&self, name: &str) -> Result<(), AgentError> {
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.remove(name) {
            client.disconnect().await?;
        }
        Ok(())
    }

    /// 获取所有工具
    pub async fn get_all_tools(&self) -> Vec<McpTool> {
        let clients = self.clients.lock().await;
        let mut all_tools = Vec::new();

        for client in clients.values() {
            let tools = client.tools.lock().await;
            all_tools.extend(tools.clone());
        }

        all_tools
    }

    /// 关闭所有连接
    pub async fn shutdown(&self) -> Result<(), AgentError> {
        let mut clients = self.clients.lock().await;
        for (_, client) in clients.drain() {
            client.disconnect().await?;
        }
        Ok(())
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
