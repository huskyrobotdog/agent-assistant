mod agent;
mod mcp_async;

pub use agent::*;
pub use mcp_async::*;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as TokioMutex;

/// 全局 Agent 状态（Tauri 管理）
struct TauriAgentState {
    agent: RwLock<Option<Arc<CoTAgent>>>,
    mcp_manager: Arc<McpManager>,
    mcp_executors: TokioMutex<HashMap<String, Arc<McpToolExecutorAsync>>>,
}

impl Default for TauriAgentState {
    fn default() -> Self {
        Self {
            agent: RwLock::new(None),
            mcp_manager: Arc::new(McpManager::new()),
            mcp_executors: TokioMutex::new(HashMap::new()),
        }
    }
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 初始化 Agent
#[tauri::command]
async fn init_agent(
    app: tauri::AppHandle,
    state: tauri::State<'_, TauriAgentState>,
    model_path: String,
    n_ctx: Option<u32>,
    n_threads: Option<i32>,
    n_gpu_layers: Option<u32>,
) -> Result<String, String> {
    let config = AgentConfig {
        model_path: std::path::PathBuf::from(model_path),
        n_ctx: n_ctx.unwrap_or(32768),
        n_threads: n_threads.unwrap_or(
            std::thread::available_parallelism()
                .map_err(|err| err.to_string())?
                .get() as i32,
        ),
        n_gpu_layers: n_gpu_layers.unwrap_or(99),
        ..Default::default()
    };

    // 在后台线程加载模型，避免阻塞主线程
    let agent = tauri::async_runtime::spawn_blocking(move || CoTAgent::new(config))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
        .map_err(|e| e.to_string())?;

    let agent = Arc::new(agent);
    *state.agent.write() = Some(agent.clone());

    // 异步加载 MCP 配置并连接服务器
    let mcp_loaded = load_mcp_servers_async(&app, &state, &agent).await;

    match mcp_loaded {
        Ok(count) if count > 0 => Ok(format!("Agent 初始化成功，已加载 {} 个 MCP 服务器", count)),
        Ok(_) => Ok("Agent 初始化成功".to_string()),
        Err(e) => Ok(format!("Agent 初始化成功，但 MCP 加载失败: {}", e)),
    }
}

/// 异步加载 MCP 服务器配置
async fn load_mcp_servers_async(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, TauriAgentState>,
    agent: &Arc<CoTAgent>,
) -> Result<usize, String> {
    let config_path = get_mcp_config_path(app)?;

    if !config_path.exists() {
        return Ok(0);
    }

    let content =
        std::fs::read_to_string(&config_path).map_err(|e| format!("读取配置文件失败: {}", e))?;

    let config: McpConfigFile =
        serde_json::from_str(&content).map_err(|e| format!("解析配置文件失败: {}", e))?;

    let mut loaded_count = 0;
    let mut all_env_configs: Vec<String> = Vec::new();

    for (name, entry) in config.mcp_servers {
        let mcp_config = McpClientConfig {
            command: entry.command.clone(),
            args: entry.args.clone(),
            env: entry.env.clone(),
            timeout_secs: None,
        };

        // 收集环境变量配置（敏感信息如密码用 *** 替代）
        if !entry.env.is_empty() {
            let env_info: Vec<String> = entry
                .env
                .iter()
                .map(|(k, v)| {
                    let key_upper = k.to_uppercase();
                    if key_upper.contains("PASSWORD")
                        || key_upper.contains("SECRET")
                        || key_upper.contains("KEY")
                    {
                        format!("  {}: (configured)", k)
                    } else {
                        format!("  {}: {}", k, v)
                    }
                })
                .collect();
            if !env_info.is_empty() {
                all_env_configs.push(format!("[{}]\n{}", name, env_info.join("\n")));
            }
        }

        match state.mcp_manager.add_server(&name, mcp_config).await {
            Ok(client) => {
                let executor = Arc::new(McpToolExecutorAsync::new(client));
                executor.cache_tools().await;

                // 注册到 agent（使用同步缓存的工具列表）
                let tools = executor.get_tools_cached();
                for tool in tools {
                    agent.register_mcp_tool(tool);
                }

                // 保存 executor 以便后续调用
                let mut executors = state.mcp_executors.lock().await;
                executors.insert(name.clone(), executor);

                loaded_count += 1;
                #[cfg(debug_assertions)]
                println!("✅ MCP 服务器 {} 已加载", name);
            }
            Err(e) => {
                #[cfg(debug_assertions)]
                eprintln!("❌ MCP 服务器 {} 加载失败: {}", name, e);
            }
        }
    }

    // 将收集的环境变量配置设置到 Agent 上下文中
    if !all_env_configs.is_empty() {
        agent.set_context(&all_env_configs.join("\n\n"));
    }

    Ok(loaded_count)
}

/// 发送消息给 Agent（流式，支持异步 MCP 工具调用）
#[tauri::command]
async fn chat(
    app: tauri::AppHandle,
    state: tauri::State<'_, TauriAgentState>,
    message: String,
    max_iterations: Option<usize>,
) -> Result<String, String> {
    let agent = state
        .agent
        .read()
        .clone()
        .ok_or_else(|| "Agent 未初始化".to_string())?;

    let max_iter = max_iterations.unwrap_or(10);

    // 初始化对话
    {
        let agent_clone = agent.clone();
        let message_clone = message.clone();
        tauri::async_runtime::spawn_blocking(move || {
            agent_clone.prepare_chat(&message_clone);
        })
        .await
        .map_err(|e| format!("准备对话失败: {}", e))?;
    }

    // 发送初始上下文信息
    let context_length = agent.get_context_length();
    let current_tokens = agent.get_current_tokens().unwrap_or(0);
    let current_chars = agent.get_current_chars().unwrap_or(0);
    let _ = app.emit(
        "context-update",
        serde_json::json!({
            "context_length": context_length,
            "current_tokens": current_tokens,
            "current_chars": current_chars
        }),
    );

    let mut final_response = String::new();
    let mut iterations = 0;

    loop {
        if iterations >= max_iter {
            break;
        }

        // 执行一步推理
        let agent_clone = agent.clone();
        let app_clone = app.clone();
        let step_result = tauri::async_runtime::spawn_blocking(move || {
            let callback = |token: &str| {
                let _ = app_clone.emit("chat-token", token.to_string());
            };
            agent_clone.generate_step(Some(&callback))
        })
        .await
        .map_err(|e| format!("推理失败: {}", e))?
        .map_err(|e| e.to_string())?;

        let (response, tool_calls) = step_result;
        final_response = response.clone();

        if tool_calls.is_empty() {
            // 没有工具调用，对话完成
            let agent_clone = agent.clone();
            tauri::async_runtime::spawn_blocking(move || {
                agent_clone.add_assistant_response(&response);
            })
            .await
            .map_err(|e| format!("添加响应失败: {}", e))?;
            break;
        }

        // 添加助手响应（包含工具调用）
        {
            let agent_clone = agent.clone();
            let response_clone = response.clone();
            let tool_calls_clone = tool_calls.clone();
            tauri::async_runtime::spawn_blocking(move || {
                agent_clone.add_assistant_response_with_tools(&response_clone, tool_calls_clone);
            })
            .await
            .map_err(|e| format!("添加响应失败: {}", e))?;
        }

        // 异步执行工具调用
        for tool_call in &tool_calls {
            let result = execute_tool_async(&state, tool_call).await;

            // 工具执行后更新上下文信息
            let current_tokens = agent.get_current_tokens().unwrap_or(0);
            let current_chars = agent.get_current_chars().unwrap_or(0);
            let _ = app.emit(
                "context-update",
                serde_json::json!({
                    "context_length": context_length,
                    "current_tokens": current_tokens,
                    "current_chars": current_chars
                }),
            );

            // 发送工具结果事件
            let _ = app.emit(
                "tool-result",
                serde_json::json!({
                    "name": result.tool_name,
                    "result": result.result,
                    "isError": result.is_error
                }),
            );

            // 添加工具结果到对话历史
            let agent_clone = agent.clone();
            let result_clone = result.clone();
            let tool_name = tool_call.name.clone();
            tauri::async_runtime::spawn_blocking(move || {
                agent_clone.add_tool_result(&tool_name, &result_clone);
            })
            .await
            .map_err(|e| format!("添加工具结果失败: {}", e))?;
        }

        iterations += 1;
    }

    // 发送完成事件
    let _ = app.emit("chat-done", final_response.clone());

    Ok(final_response)
}

/// 异步执行工具调用
async fn execute_tool_async(
    state: &tauri::State<'_, TauriAgentState>,
    tool_call: &ToolCall,
) -> ToolResult {
    let executors = state.mcp_executors.lock().await;

    // 查找能执行此工具的 executor
    for executor in executors.values() {
        let tools = executor.get_tools_cached();
        if tools.iter().any(|t| t.name == tool_call.name) {
            match executor.execute_async(tool_call).await {
                Ok(result) => return result,
                Err(e) => {
                    return ToolResult {
                        tool_name: tool_call.name.clone(),
                        result: format!("工具执行错误: {}", e),
                        is_error: true,
                    }
                }
            }
        }
    }

    ToolResult {
        tool_name: tool_call.name.clone(),
        result: format!("工具 {} 未找到", tool_call.name),
        is_error: true,
    }
}

/// 清空对话历史
#[tauri::command]
fn clear_history(state: tauri::State<'_, TauriAgentState>) -> Result<(), String> {
    let agent = state
        .agent
        .read()
        .clone()
        .ok_or_else(|| "Agent 未初始化".to_string())?;

    agent.clear_history();
    Ok(())
}

/// 获取对话历史
#[tauri::command]
fn get_messages(state: tauri::State<'_, TauriAgentState>) -> Result<Vec<Message>, String> {
    let agent = state
        .agent
        .read()
        .clone()
        .ok_or_else(|| "Agent 未初始化".to_string())?;

    Ok(agent.get_messages())
}

/// 获取 Agent 状态
#[tauri::command]
fn get_agent_state(state: tauri::State<'_, TauriAgentState>) -> Result<String, String> {
    let agent = state
        .agent
        .read()
        .clone()
        .ok_or_else(|| "Agent 未初始化".to_string())?;

    let agent_state = agent.get_state();
    Ok(format!("{:?}", agent_state))
}

/// 获取上下文信息
#[derive(Serialize)]
struct ContextInfo {
    context_length: u32,
    current_tokens: usize,
    current_chars: usize,
}

#[tauri::command]
fn get_context_info(state: tauri::State<'_, TauriAgentState>) -> Result<ContextInfo, String> {
    let agent = state
        .agent
        .read()
        .clone()
        .ok_or_else(|| "Agent 未初始化".to_string())?;

    let context_length = agent.get_context_length();
    let current_tokens = agent.get_current_tokens().unwrap_or(0);
    let current_chars = agent.get_current_chars().unwrap_or(0);

    Ok(ContextInfo {
        context_length,
        current_tokens,
        current_chars,
    })
}

/// 添加 MCP 服务器
#[tauri::command]
async fn add_mcp_server(
    state: tauri::State<'_, TauriAgentState>,
    name: String,
    command: String,
    args: Vec<String>,
) -> Result<String, String> {
    let config = McpClientConfig {
        command,
        args,
        env: std::collections::HashMap::new(),
        timeout_secs: None,
    };

    let client = state
        .mcp_manager
        .add_server(&name, config)
        .await
        .map_err(|e| e.to_string())?;

    let executor = Arc::new(McpToolExecutorAsync::new(client));
    executor.cache_tools().await;

    // 注册到 agent
    if let Some(agent) = state.agent.read().clone() {
        let tools = executor.get_tools_cached();
        for tool in tools {
            agent.register_mcp_tool(tool);
        }
    }

    // 保存 executor
    let mut executors = state.mcp_executors.lock().await;
    executors.insert(name.clone(), executor);

    Ok(format!("MCP 服务器 {} 添加成功", name))
}

/// 移除 MCP 服务器
#[tauri::command]
async fn remove_mcp_server(
    state: tauri::State<'_, TauriAgentState>,
    name: String,
) -> Result<String, String> {
    state
        .mcp_manager
        .remove_server(&name)
        .await
        .map_err(|e| e.to_string())?;

    // 移除 executor
    let mut executors = state.mcp_executors.lock().await;
    executors.remove(&name);

    Ok(format!("MCP 服务器 {} 已移除", name))
}

/// MCP 服务器配置（用于JSON序列化）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct McpServerEntry {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

/// MCP 配置文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct McpConfigFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServerEntry>,
}

/// 获取 mcp.json 文件路径
fn get_mcp_config_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    Ok(app_data.join("mcp.json"))
}

/// 读取 MCP 配置
#[tauri::command]
fn get_mcp_config(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let config_path = get_mcp_config_path(&app)?;

    if !config_path.exists() {
        // 返回默认配置
        let default_config = McpConfigFile::default();
        return serde_json::to_value(default_config).map_err(|e| e.to_string());
    }

    let content =
        std::fs::read_to_string(&config_path).map_err(|e| format!("读取配置文件失败: {}", e))?;

    let config: McpConfigFile =
        serde_json::from_str(&content).map_err(|e| format!("解析配置文件失败: {}", e))?;

    serde_json::to_value(config).map_err(|e| e.to_string())
}

/// 保存 MCP 配置
#[tauri::command]
fn save_mcp_config(app: tauri::AppHandle, config: serde_json::Value) -> Result<String, String> {
    let config_path = get_mcp_config_path(&app)?;

    // 验证配置格式
    let _: McpConfigFile =
        serde_json::from_value(config.clone()).map_err(|e| format!("配置格式无效: {}", e))?;

    // 确保目录存在
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    let content = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;

    std::fs::write(&config_path, content).map_err(|e| format!("保存配置文件失败: {}", e))?;

    Ok("配置保存成功".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::new().build())
        .manage(TauriAgentState::default())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let models_dir = app_data.join("models");
            std::fs::create_dir_all(&models_dir)?;
            #[cfg(debug_assertions)]
            {
                println!("模型目录: {:?}", models_dir);
            }

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            init_agent,
            chat,
            clear_history,
            get_messages,
            get_agent_state,
            get_context_info,
            add_mcp_server,
            remove_mcp_server,
            get_mcp_config,
            save_mcp_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
