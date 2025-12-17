mod agent;
mod mcp;

pub use agent::*;
pub use mcp::*;

use parking_lot::RwLock;
use std::sync::Arc;
use tauri::Manager;

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
        n_gpu_layers: n_gpu_layers.unwrap_or(0),
        ..Default::default()
    };

    let agent = ReactAgent::new(config).map_err(|e| e.to_string())?;

    *state.agent.write() = Some(Arc::new(agent));

    Ok("Agent 初始化成功".to_string())
}

/// 发送消息给 Agent
#[tauri::command]
async fn chat(
    state: tauri::State<'_, TauriAgentState>,
    message: String,
    max_iterations: Option<usize>,
) -> Result<String, String> {
    let agent = state
        .agent
        .read()
        .clone()
        .ok_or_else(|| "Agent 未初始化".to_string())?;

    let response = agent
        .run(&message, max_iterations.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())?;

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
async fn add_mcp_server(
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
    };

    mcp_manager
        .add_server(&name, config)
        .await
        .map_err(|e| e.to_string())?;

    let agent_opt = state.agent.read().clone();
    if let Some(agent) = agent_opt {
        if let Some(client) = mcp_manager.get_client(&name).await {
            let executor = Arc::new(McpToolExecutorWrapper::new(client));
            agent.register_tool_executor(&name, executor);
        }
    }

    Ok(format!("MCP 服务器 {} 添加成功", name))
}

/// 移除 MCP 服务器
#[tauri::command]
async fn remove_mcp_server(
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
        .await
        .map_err(|e| e.to_string())?;

    Ok(format!("MCP 服务器 {} 已移除", name))
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
            remove_mcp_server
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
