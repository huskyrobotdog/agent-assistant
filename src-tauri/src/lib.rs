mod agent;

pub use agent::*;

use tauri::{path::BaseDirectory, Emitter, Manager};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 初始化 Agent
#[tauri::command]
async fn init_agent_cmd(app: tauri::AppHandle) -> Result<String, String> {
    let model_path = app
        .path()
        .resolve("resources/model/agent", BaseDirectory::Resource)
        .map_err(|e| format!("获取模型路径失败: {}", e))?;

    tauri::async_runtime::spawn_blocking(move || agent::init_agent(model_path))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
        .map_err(|e| e.to_string())?;

    Ok("Agent 初始化成功".to_string())
}

/// 发送消息给 Agent（流式）
#[tauri::command]
async fn chat_cmd(
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
            println!("模型目录: {:?}", models_dir);

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, init_agent_cmd, chat_cmd,])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
