mod agent;
mod db;
mod mcp;
mod tool;

use tauri::{path::BaseDirectory, Emitter, Manager};

/// 初始化 Agent
#[tauri::command]
async fn init_agent(app: tauri::AppHandle) -> Result<String, String> {
    let model_path = app
        .path()
        .resolve("resources/model/agent", BaseDirectory::Resource)
        .map_err(|e| format!("获取模型路径失败: {}", e))?;

    tauri::async_runtime::spawn_blocking(move || agent::init(model_path))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
        .map_err(|e| e.to_string())?;

    Ok("Agent 初始化成功".to_string())
}

/// 发送消息给 Agent（流式）
#[tauri::command]
async fn chat(
    app: tauri::AppHandle,
    message: String,
    system_prompt: Option<String>,
) -> Result<String, String> {
    let app_clone = app.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let callback = |token: &str| {
            let _ = app_clone.emit("chat-token", token.to_string());
        };
        agent::chat(&message, system_prompt.as_deref(), Some(&callback))
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
        .invoke_handler(tauri::generate_handler![init_agent, chat,])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
