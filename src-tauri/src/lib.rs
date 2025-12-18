mod agent;
mod mcp;
mod tool;

pub use agent::*;
pub use mcp::*;
pub use tool::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{Emitter, Manager};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 初始化 Agent
#[tauri::command]
async fn init_agent(
    app: tauri::AppHandle,
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
    tauri::async_runtime::spawn_blocking(move || init_agent_singleton(config))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
        .map_err(|e| e.to_string())?;

    // 异步加载 MCP 配置并连接服务器
    let mcp_loaded = load_mcp_servers_async(&app).await;

    match mcp_loaded {
        Ok(count) if count > 0 => Ok(format!("Agent 初始化成功，已加载 {} 个 MCP 服务器", count)),
        Ok(_) => Ok("Agent 初始化成功".to_string()),
        Err(e) => Ok(format!("Agent 初始化成功，但 MCP 加载失败: {}", e)),
    }
}

/// 获取 SQLite 数据库路径
fn get_db_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let app_data = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("获取应用配置目录失败: {}", e))?;
    Ok(app_data.join("datas.db"))
}

/// 异步加载 MCP 服务器配置
async fn load_mcp_servers_async(app: &tauri::AppHandle) -> Result<usize, String> {
    let db_path = get_db_path(app)?;

    if !db_path.exists() {
        return Ok(0);
    }

    // 从 SQLite 读取 MCP 配置
    let conn =
        rusqlite::Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {}", e))?;

    let config_value: Option<String> = conn
        .query_row("SELECT value FROM config WHERE key = 'mcp'", [], |row| {
            row.get(0)
        })
        .ok();

    let config: McpConfigFile = match config_value {
        Some(json_str) => {
            serde_json::from_str(&json_str).map_err(|e| format!("解析配置失败: {}", e))?
        }
        None => return Ok(0),
    };

    let mut loaded_count = 0;
    let mut all_env_configs: Vec<String> = Vec::new();

    for (name, entry) in config.mcp_servers {
        let mcp_config = McpClientConfig {
            command: entry.command.clone(),
            args: entry.args.clone(),
            env: entry.env.clone(),
            timeout_secs: None,
        };

        // 构建命名空间：mcp.服务名
        let namespace = format!("mcp.{}", name);

        // 收集环境变量配置（完整显示，供 agent 使用）
        if !entry.env.is_empty() {
            let env_info: Vec<String> = entry
                .env
                .iter()
                .map(|(k, v)| format!("  {}: {}", k, v))
                .collect();
            if !env_info.is_empty() {
                all_env_configs.push(format!("环境变量 {}\n{}", namespace, env_info.join("\n")));
            }
        }

        match MCP_MANAGER.add_server(&name, mcp_config).await {
            Ok(_) => {
                // 注册到 agent（使用同步缓存的工具列表，带命名空间）
                if let Some(executor) = MCP_MANAGER.get_executor(&name).await {
                    if let Some(agent) = AGENT.read().as_ref() {
                        let tools = executor.get_tools_cached();
                        for tool in tools {
                            agent.register_mcp_tool(tool, &namespace);
                        }
                    }
                }

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
        if let Some(agent) = AGENT.read().as_ref() {
            agent.set_context(&all_env_configs.join("\n\n"));
        }
    }

    Ok(loaded_count)
}

/// 发送消息给 Agent（流式，支持异步 MCP 工具调用）
#[tauri::command]
async fn chat(
    app: tauri::AppHandle,
    message: String,
    max_iterations: Option<usize>,
) -> Result<String, String> {
    // 检查 Agent 是否已初始化
    {
        let guard = AGENT.read();
        if guard.is_none() {
            return Err("Agent 未初始化".to_string());
        }
    }

    let max_iter = max_iterations.unwrap_or(10);

    // 初始化对话
    {
        let message_clone = message.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Some(agent) = AGENT.read().as_ref() {
                agent.prepare_chat(&message_clone);
            }
        })
        .await
        .map_err(|e| format!("准备对话失败: {}", e))?;
    }

    // 发送初始上下文信息
    let (context_length, current_tokens, current_chars) = {
        let guard = AGENT.read();
        let agent = guard.as_ref().ok_or("Agent 未初始化")?;
        (
            agent.get_context_length(),
            agent.get_current_tokens().unwrap_or(0),
            agent.get_current_chars().unwrap_or(0),
        )
    };
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
        let app_clone = app.clone();
        let step_result = tauri::async_runtime::spawn_blocking(move || {
            let guard = AGENT.read();
            let agent = guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Agent 未初始化"))?;
            let callback = |token: &str| {
                let _ = app_clone.emit("chat-token", token.to_string());
            };
            agent.generate_step(Some(&callback))
        })
        .await
        .map_err(|e| format!("推理失败: {}", e))?
        .map_err(|e| e.to_string())?;

        let (response, tool_calls) = step_result;
        final_response = response.clone();

        #[cfg(debug_assertions)]
        println!(
            "\n📊 [循环状态] iterations={}, tool_calls.len()={}",
            iterations,
            tool_calls.len()
        );

        if tool_calls.is_empty() {
            // 没有工具调用，对话完成
            #[cfg(debug_assertions)]
            println!("🛑 [准备退出循环] 无工具调用");

            tauri::async_runtime::spawn_blocking(move || {
                if let Some(agent) = AGENT.read().as_ref() {
                    agent.add_assistant_response(&response);
                }
            })
            .await
            .map_err(|e| format!("添加响应失败: {}", e))?;

            #[cfg(debug_assertions)]
            println!("🛑 [执行 break]");
            break;
        }

        // 添加助手响应（包含工具调用）
        {
            let response_clone = response.clone();
            let tool_calls_clone = tool_calls.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Some(agent) = AGENT.read().as_ref() {
                    agent.add_assistant_response_with_tools(&response_clone, tool_calls_clone);
                }
            })
            .await
            .map_err(|e| format!("添加响应失败: {}", e))?;
        }

        // 异步执行工具调用
        for tool_call in &tool_calls {
            let result = execute_tool_async(tool_call).await;

            // 工具执行后更新上下文信息
            let (current_tokens, current_chars) = {
                let guard = AGENT.read();
                if let Some(agent) = guard.as_ref() {
                    (
                        agent.get_current_tokens().unwrap_or(0),
                        agent.get_current_chars().unwrap_or(0),
                    )
                } else {
                    (0, 0)
                }
            };
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
            let result_clone = result.clone();
            let tool_name = tool_call.name.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Some(agent) = AGENT.read().as_ref() {
                    agent.add_tool_result(&tool_name, &result_clone);
                }
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

/// 异步执行工具调用（支持命名空间格式如 mcp.mysql.connect_db）
async fn execute_tool_async(tool_call: &ToolCall) -> ToolResult {
    let executors = MCP_MANAGER.get_all_executors().await;

    #[cfg(debug_assertions)]
    println!(
        "\n⚡ [工具调用] {} | executors 数量: {}",
        tool_call.name,
        executors.len()
    );

    // 从命名空间格式中提取原始工具名（mcp.mysql.connect_db -> connect_db）
    let original_tool_name = tool_call.name.rsplit('.').next().unwrap_or(&tool_call.name);

    #[cfg(debug_assertions)]
    println!("   原始工具名: {}", original_tool_name);

    // 创建使用原始工具名的 ToolCall
    let original_tool_call = ToolCall {
        name: original_tool_name.to_string(),
        arguments: tool_call.arguments.clone(),
    };

    // 查找能执行此工具的 executor
    for (name, executor) in executors.iter() {
        let tools = executor.get_tools_cached();
        #[cfg(debug_assertions)]
        println!("   检查 executor '{}': {} 个工具", name, tools.len());

        if tools.iter().any(|t| t.name == original_tool_name) {
            #[cfg(debug_assertions)]
            println!("   ✓ 找到匹配的 executor: {}", name);

            match executor.execute_async(&original_tool_call).await {
                Ok(result) => {
                    #[cfg(debug_assertions)]
                    println!("   ✓ 执行成功");
                    return result;
                }
                Err(e) => {
                    #[cfg(debug_assertions)]
                    println!("   ✗ 执行失败: {}", e);
                    return ToolResult {
                        tool_name: tool_call.name.clone(),
                        result: format!("工具执行错误: {}", e),
                        is_error: true,
                    };
                }
            }
        }
    }

    #[cfg(debug_assertions)]
    println!("   ✗ 未找到匹配的 executor");

    ToolResult {
        tool_name: tool_call.name.clone(),
        result: format!("工具 {} 未找到", tool_call.name),
        is_error: true,
    }
}

/// 清空对话历史
#[tauri::command]
fn clear_history() -> Result<(), String> {
    let guard = AGENT.read();
    let agent = guard.as_ref().ok_or_else(|| "Agent 未初始化".to_string())?;
    agent.clear_history();
    Ok(())
}

/// 获取对话历史
#[tauri::command]
fn get_messages() -> Result<Vec<Message>, String> {
    let guard = AGENT.read();
    let agent = guard.as_ref().ok_or_else(|| "Agent 未初始化".to_string())?;
    Ok(agent.get_messages())
}

/// 获取 Agent 状态
#[tauri::command]
fn get_agent_state() -> Result<String, String> {
    let guard = AGENT.read();
    let agent = guard.as_ref().ok_or_else(|| "Agent 未初始化".to_string())?;
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
fn get_context_info() -> Result<ContextInfo, String> {
    let guard = AGENT.read();
    let agent = guard.as_ref().ok_or_else(|| "Agent 未初始化".to_string())?;

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

    MCP_MANAGER
        .add_server(&name, config)
        .await
        .map_err(|e| e.to_string())?;

    // 注册到 agent（带命名空间）
    let namespace = format!("mcp.{}", name);
    if let Some(executor) = MCP_MANAGER.get_executor(&name).await {
        if let Some(agent) = AGENT.read().as_ref() {
            let tools = executor.get_tools_cached();
            for tool in tools {
                agent.register_mcp_tool(tool, &namespace);
            }
        }
    }

    Ok(format!("MCP 服务器 {} 添加成功", name))
}

/// 移除 MCP 服务器
#[tauri::command]
async fn remove_mcp_server(name: String) -> Result<String, String> {
    MCP_MANAGER
        .remove_server(&name)
        .await
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

/// 获取数据库迁移配置
fn get_migrations() -> Vec<tauri_plugin_sql::Migration> {
    use tauri_plugin_sql::{Migration, MigrationKind};

    vec![
        Migration {
            version: 1,
            description: "create_agents_table",
            sql: r#"
                CREATE TABLE IF NOT EXISTS agents (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    system_prompt TEXT DEFAULT '',
                    allow_tools INTEGER DEFAULT 1,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
                )
            "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "create_config_table",
            sql: r#"
                CREATE TABLE IF NOT EXISTS config (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    key TEXT NOT NULL UNIQUE,
                    value TEXT DEFAULT '{}',
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
                )
            "#,
            kind: MigrationKind::Up,
        },
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations("sqlite:datas.db", get_migrations())
                .build(),
        )
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
            remove_mcp_server
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
