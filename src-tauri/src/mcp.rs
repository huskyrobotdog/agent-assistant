use crate::tool::{McpTool, ToolResult};
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use rmcp::{
    model::{CallToolRequestParam, Tool},
    service::{RunningService, ServiceExt},
    transport::{ConfigureCommandExt, TokioChildProcess},
    RoleClient,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;

/// 全局 MCP 管理器单例
pub static MCP_MANAGER: Lazy<McpManager> = Lazy::new(McpManager::new);

/// MCP 配置结构（与前端配置格式一致）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default, alias = "mcpServers")]
    pub servers: HashMap<String, McpClientConfig>,
}

/// 从数据库初始化 MCP
pub async fn init(db_path: PathBuf) -> Result<()> {
    use rusqlite::Connection;

    // 查询配置
    let config_json = tokio::task::spawn_blocking(move || -> Result<String> {
        let conn = Connection::open(&db_path).context("打开数据库失败")?;
        let result: Result<String, rusqlite::Error> =
            conn.query_row("SELECT value FROM config WHERE key = 'mcp'", [], |row| {
                row.get(0)
            });
        match result {
            Ok(value) => Ok(value),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok("{}".to_string()),
            Err(e) => Err(anyhow::anyhow!("查询配置失败: {}", e)),
        }
    })
    .await
    .context("任务执行失败")??;

    #[cfg(debug_assertions)]
    println!("[MCP] 配置: {}", config_json);

    // 解析配置
    let config: McpConfig =
        serde_json::from_str(&config_json).unwrap_or_else(|_| McpConfig::default());

    #[cfg(debug_assertions)]
    println!("[MCP] 加载配置: {:?}", config);

    // 初始化所有服务器
    for (name, server_config) in config.servers {
        #[cfg(debug_assertions)]
        println!("[MCP] 正在连接服务器: {}", name);

        match MCP_MANAGER.add_server(&name, server_config).await {
            Ok(_) => {
                #[cfg(debug_assertions)]
                println!("[MCP] 服务器 {} 连接成功", name);
            }
            Err(e) => {
                #[cfg(debug_assertions)]
                println!("[MCP] 服务器 {} 连接失败: {}", name, e);
            }
        }
    }

    Ok(())
}

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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub async fn is_connected(&self) -> bool {
        let state = self.state.lock().await;
        state.service.is_some()
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

    /// 获取所有工具
    pub async fn get_all_tools(&self) -> Vec<McpTool> {
        let clients = self.clients.lock().await;
        let mut all_tools = Vec::new();

        for client in clients.values() {
            all_tools.extend(client.get_tools().await);
        }

        all_tools
    }

    /// 通过工具名执行工具（自动查找对应的服务器）
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolResult> {
        let clients = self.clients.lock().await;

        // 遍历所有客户端，查找包含该工具的服务器
        for client in clients.values() {
            let tools = client.get_tools().await;
            if tools.iter().any(|t| t.name == tool_name) {
                return client.call_tool(tool_name, arguments).await;
            }
        }

        Err(anyhow::anyhow!("未找到工具: {}", tool_name))
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
