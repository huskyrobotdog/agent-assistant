use crate::tool::{McpTool, ToolCall, ToolResult};
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rmcp::{
    model::{CallToolRequestParam, Tool},
    service::{RunningService, ServiceExt},
    transport::{ConfigureCommandExt, TokioChildProcess},
    RoleClient,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;

/// 全局 MCP 管理器单例
pub static MCP_MANAGER: Lazy<McpManager> = Lazy::new(McpManager::new);

/// MCP 客户端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpClientConfig {
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// MCP 客户端状态
struct McpClientState {
    service: Option<RunningService<RoleClient, ()>>,
    tools: Vec<McpTool>,
}

/// 异步 MCP 客户端 - 使用 rmcp 库
pub struct McpClient {
    config: McpClientConfig,
    state: TokioMutex<McpClientState>,
}

impl McpClient {
    pub fn new(config: McpClientConfig) -> Self {
        Self {
            config,
            state: TokioMutex::new(McpClientState {
                service: None,
                tools: Vec::new(),
            }),
        }
    }

    /// 连接到 MCP 服务器
    pub async fn connect(&self) -> Result<()> {
        let mut cmd = Command::new(&self.config.command);

        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        let args = self.config.args.clone();
        let transport = TokioChildProcess::new(cmd.configure(move |c| {
            for arg in &args {
                c.arg(arg);
            }
        }))
        .context("创建传输层失败")?;

        let service = ().serve(transport).await.context("连接 MCP 服务器失败")?;

        let mut state = self.state.lock().await;
        state.service = Some(service);

        drop(state);

        self.refresh_tools().await?;

        Ok(())
    }

    /// 刷新工具列表
    pub async fn refresh_tools(&self) -> Result<()> {
        let state = self.state.lock().await;
        let service = state.service.as_ref().context("MCP 服务器未连接")?;

        let tools_result = service
            .list_tools(Default::default())
            .await
            .context("获取工具列表失败")?;

        drop(state);

        let tools: Vec<McpTool> = tools_result
            .tools
            .into_iter()
            .map(|t| Self::convert_tool(&t))
            .collect();

        let mut state = self.state.lock().await;
        state.tools = tools;

        Ok(())
    }

    /// 转换 rmcp Tool 到 McpTool
    fn convert_tool(tool: &Tool) -> McpTool {
        McpTool {
            name: tool.name.to_string(),
            description: tool
                .description
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            input_schema: serde_json::to_value(&*tool.input_schema)
                .unwrap_or_else(|_| serde_json::json!({})),
        }
    }

    /// 调用工具
    pub async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> Result<ToolResult> {
        let state = self.state.lock().await;
        let service = state.service.as_ref().context("MCP 服务器未连接")?;

        let tool_name = name.to_string();
        let result = service
            .call_tool(CallToolRequestParam {
                name: tool_name.into(),
                arguments: arguments.as_object().cloned(),
            })
            .await
            .context("调用工具失败")?;

        let result_text = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.to_string()))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult {
            tool_name: name.to_string(),
            result: result_text,
            is_error: result.is_error.unwrap_or(false),
        })
    }

    /// 断开连接
    pub async fn disconnect(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(service) = state.service.take() {
            service.cancel().await.context("断开连接失败")?;
        }
        Ok(())
    }

    /// 获取工具列表
    pub async fn get_tools(&self) -> Vec<McpTool> {
        let state = self.state.lock().await;
        state.tools.clone()
    }

    /// 检查是否已连接
    pub async fn is_connected(&self) -> bool {
        let state = self.state.lock().await;
        state.service.is_some()
    }
}

/// MCP 工具执行器包装（异步版本）
pub struct McpToolExecutorAsync {
    client: Arc<McpClient>,
    tools_cache: RwLock<Vec<McpTool>>,
}

impl McpToolExecutorAsync {
    pub fn new(client: Arc<McpClient>) -> Self {
        Self {
            client,
            tools_cache: RwLock::new(Vec::new()),
        }
    }

    /// 同步缓存工具列表（在连接后调用一次）
    pub async fn cache_tools(&self) {
        let tools = self.client.get_tools().await;
        *self.tools_cache.write() = tools;
    }

    /// 异步执行工具
    pub async fn execute_async(&self, tool_call: &ToolCall) -> Result<ToolResult> {
        self.client
            .call_tool(&tool_call.name, tool_call.arguments.clone())
            .await
    }

    /// 获取缓存的工具列表（同步）
    pub fn get_tools_cached(&self) -> Vec<McpTool> {
        self.tools_cache.read().clone()
    }
}

/// MCP 服务器管理器 - 管理多个 MCP 服务器连接
pub struct McpManager {
    clients: TokioMutex<HashMap<String, Arc<McpClient>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: TokioMutex::new(HashMap::new()),
        }
    }

    /// 添加并连接 MCP 服务器
    pub async fn add_server(&self, name: &str, config: McpClientConfig) -> Result<Arc<McpClient>> {
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
    pub async fn remove_server(&self, name: &str) -> Result<()> {
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
            all_tools.extend(client.get_tools().await);
        }

        all_tools
    }

    /// 获取所有客户端名称
    pub async fn get_server_names(&self) -> Vec<String> {
        let clients = self.clients.lock().await;
        clients.keys().cloned().collect()
    }

    /// 关闭所有连接
    pub async fn shutdown(&self) -> Result<()> {
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
