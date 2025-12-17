mod agent;
mod mcp;

pub use agent::*;
pub use mcp::*;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// 全局 Agent 状态（Tauri 管理）
struct TauriAgentState {
    agent: RwLock<Option<Arc<ReactAgent>>>,
    mcp_manager: RwLock<Option<Arc<McpManager>>>,
}

impl Default for TauriAgentState {
    fn default() -> Self {
        Self {
            agent: RwLock::new(None),
            mcp_manager: RwLock::new(Some(Arc::new(McpManager::new()))),
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
        n_ctx: n_ctx.unwrap_or(8192),
        n_threads: n_threads.unwrap_or(4),
        n_gpu_layers: n_gpu_layers.unwrap_or(99),
        ..Default::default()
    };

    // 在后台线程加载模型，避免阻塞主线程
    let agent = tokio::task::spawn_blocking(move || ReactAgent::new(config))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
        .map_err(|e| e.to_string())?;

    let agent = Arc::new(agent);
    *state.agent.write() = Some(agent.clone());

    // 加载 MCP 配置并连接服务器
    let mcp_loaded = load_mcp_servers(&app, &state, &agent);

    match mcp_loaded {
        Ok(count) if count > 0 => Ok(format!("Agent 初始化成功，已加载 {} 个 MCP 服务器", count)),
        Ok(_) => Ok("Agent 初始化成功".to_string()),
        Err(e) => Ok(format!("Agent 初始化成功，但 MCP 加载失败: {}", e)),
    }
}

/// 加载 MCP 服务器配置
fn load_mcp_servers(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, TauriAgentState>,
    agent: &Arc<ReactAgent>,
) -> Result<usize, String> {
    let config_path = get_mcp_config_path(app)?;

    if !config_path.exists() {
        return Ok(0);
    }

    let content =
        std::fs::read_to_string(&config_path).map_err(|e| format!("读取配置文件失败: {}", e))?;

    let config: McpConfigFile =
        serde_json::from_str(&content).map_err(|e| format!("解析配置文件失败: {}", e))?;

    let mcp_manager = state
        .mcp_manager
        .read()
        .clone()
        .ok_or_else(|| "MCP 管理器未初始化".to_string())?;

    let mut loaded_count = 0;

    for (name, entry) in config.mcp_servers {
        let mcp_config = McpClientConfig {
            command: entry.command.clone(),
            args: entry.args.clone(),
            env: entry.env.clone(),
            timeout_secs: None,
        };

        match mcp_manager.add_server(&name, mcp_config) {
            Ok(_) => {
                if let Some(client) = mcp_manager.get_client(&name) {
                    let executor = Arc::new(McpToolExecutorWrapper::new(client));
                    agent.register_tool_executor(&name, executor);
                    loaded_count += 1;
                    #[cfg(debug_assertions)]
                    println!("✅ MCP 服务器 {} 已加载", name);
                }
            }
            Err(e) => {
                #[cfg(debug_assertions)]
                eprintln!("❌ MCP 服务器 {} 加载失败: {}", name, e);
            }
        }
    }

    Ok(loaded_count)
}

/// 发送消息给 Agent（流式）
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
    let app_clone = app.clone();

    let app_clone2 = app.clone();

    // 在后台线程执行推理，避免阻塞主线程
    let response = tokio::task::spawn_blocking(move || {
        let callback = |token: &str| {
            let _ = app_clone.emit("chat-token", token.to_string());
        };
        let tool_callback = |name: &str, result: &str, is_error: bool| {
            let _ = app_clone2.emit(
                "tool-result",
                serde_json::json!({
                    "name": name,
                    "result": result,
                    "isError": is_error
                }),
            );
        };
        agent.run_with_callbacks(&message, max_iter, Some(&callback), Some(&tool_callback))
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
    .map_err(|e| e.to_string())?;

    // 发送完成事件
    let _ = app.emit("chat-done", response.clone());

    Ok(response)
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

/// 添加 MCP 服务器
#[tauri::command]
fn add_mcp_server(
    state: tauri::State<'_, TauriAgentState>,
    name: String,
    command: String,
    args: Vec<String>,
) -> Result<String, String> {
    let mcp_manager = state
        .mcp_manager
        .read()
        .clone()
        .ok_or_else(|| "MCP 管理器未初始化".to_string())?;

    let config = McpClientConfig {
        command,
        args,
        env: std::collections::HashMap::new(),
        timeout_secs: None,
    };

    mcp_manager
        .add_server(&name, config)
        .map_err(|e| e.to_string())?;

    let agent_opt = state.agent.read().clone();
    if let Some(agent) = agent_opt {
        if let Some(client) = mcp_manager.get_client(&name) {
            let executor = Arc::new(McpToolExecutorWrapper::new(client));
            agent.register_tool_executor(&name, executor);
        }
    }

    Ok(format!("MCP 服务器 {} 添加成功", name))
}

/// 移除 MCP 服务器
#[tauri::command]
fn remove_mcp_server(
    state: tauri::State<'_, TauriAgentState>,
    name: String,
) -> Result<String, String> {
    let mcp_manager = state
        .mcp_manager
        .read()
        .clone()
        .ok_or_else(|| "MCP 管理器未初始化".to_string())?;

    mcp_manager
        .remove_server(&name)
        .map_err(|e| e.to_string())?;

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
            add_mcp_server,
            remove_mcp_server,
            get_mcp_config,
            save_mcp_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
