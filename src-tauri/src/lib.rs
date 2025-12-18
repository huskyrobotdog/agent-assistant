mod agent;
mod db;
mod mcp;

use tauri::{path::BaseDirectory, Emitter, Manager};

/// 初始化 Agent
#[tauri::command]
async fn init_agent(app: tauri::AppHandle) -> Result<String, String> {
    let model_path = app
        .path()
        .resolve("resources/agent", BaseDirectory::Resource)
        .map_err(|e| format!("获取模型路径失败: {}", e))?;

    tauri::async_runtime::spawn_blocking(move || agent::init(model_path))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
        .map_err(|e| e.to_string())?;

    Ok("Agent 初始化成功".to_string())
}

/// 初始化 MCP
#[tauri::command]
async fn init_mcp(app: tauri::AppHandle) -> Result<String, String> {
    let db_path = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取数据目录失败: {}", e))?
        .join("datas.db");

    mcp::init(db_path).await.map_err(|e| e.to_string())?;

    Ok("MCP 初始化成功".to_string())
}

/// 发送消息给 Agent（流式）
#[tauri::command]
async fn chat(
    app: tauri::AppHandle,
    message: String,
    _system_prompt: Option<String>,
) -> Result<String, String> {
    let app_clone = app.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let callback = |token: &str| {
            let _ = app_clone.emit("chat-token", token.to_string());
        };

        agent::chat(&message, Some(&callback))
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))?
    .map_err(|e| e.to_string())?;

    let _ = app.emit("chat-done", result.clone());

    Ok(result)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations("sqlite:datas.db", db::get_migrations())
                .build(),
        )
        // .setup(|app| Ok(()))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![init_agent, init_mcp, chat])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
