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

/// 获取 prompt 文件内容
#[tauri::command]
async fn get_prompt(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let prompt_path = app
        .path()
        .resolve(
            format!("resources/prompt/{}.md", name),
            BaseDirectory::Resource,
        )
        .map_err(|e| format!("获取 prompt 路径失败: {}", e))?;

    std::fs::read_to_string(&prompt_path).map_err(|e| format!("读取 prompt 文件失败: {}", e))
}

/// 获取 MCP 工具描述作为 prompt
#[tauri::command]
async fn get_mcp_prompt() -> Result<String, String> {
    let tools = mcp::MCP_MANAGER.get_all_tools().await;

    if tools.is_empty() {
        return Ok(String::new());
    }

    let mut prompt = String::from("# 可用工具\n\n");
    for tool in tools {
        prompt.push_str(&format!("## {}\n", tool.name));
        prompt.push_str(&format!("{}\n\n", tool.description));
        prompt.push_str(&format!("参数: {}\n\n", tool.input_schema));
    }

    Ok(prompt)
}

/// 发送消息给 Agent（流式）
#[tauri::command]
async fn chat(
    app: tauri::AppHandle,
    message: String,
    system_prompt: Option<String>,
) -> Result<String, String> {
    // 如果没有传递 system_prompt，则使用默认模板并替换占位符
    let final_prompt = match system_prompt {
        Some(p) => p,
        None => {
            // 读取 agent.md 模板
            let prompt_path = app
                .path()
                .resolve("resources/prompt/agent.md", BaseDirectory::Resource)
                .map_err(|e| format!("获取 prompt 路径失败: {}", e))?;

            let template = std::fs::read_to_string(&prompt_path)
                .map_err(|e| format!("读取 prompt 文件失败: {}", e))?;

            // 构建工具 prompt（Qwen ReAct 格式）
            let tools = mcp::MCP_MANAGER.get_all_tools().await;
            let (tools_prompt, tool_names) = if tools.is_empty() {
                (String::new(), String::new())
            } else {
                let mut descs = Vec::new();
                let mut names = Vec::new();
                for tool in tools {
                    // Qwen ReAct 工具描述格式
                    let desc = format!(
                        "{}: Call this tool to interact with the {} API. What is the {} API useful for? {} Parameters: {} Format the arguments as a JSON object.",
                        tool.name,
                        tool.name,
                        tool.name,
                        tool.description,
                        tool.input_schema
                    );
                    descs.push(desc);
                    names.push(tool.name);
                }
                (descs.join("\n\n"), names.join(","))
            };

            // 替换占位符
            template
                .replace("{{TOOLS}}", &tools_prompt)
                .replace("{{TOOL_NAMES}}", &tool_names)
                .replace("{{QUERY}}", &message)
        }
    };

    let app_clone = app.clone();

    // 获取所有 MCP 执行器用于工具调用
    let executors = mcp::MCP_MANAGER.get_all_executors().await;

    let result = tauri::async_runtime::spawn_blocking(move || {
        let callback = |token: &str| {
            let _ = app_clone.emit("chat-token", token.to_string());
        };

        // 工具执行器：解析工具名并调用对应的 MCP 服务
        let tool_executor =
            |tool_call: &crate::tool::ToolCall| -> anyhow::Result<crate::tool::ToolResult> {
                // 解析工具名：格式为 mcp.server_name.tool_name
                let parts: Vec<&str> = tool_call.name.splitn(3, '.').collect();
                if parts.len() != 3 || parts[0] != "mcp" {
                    return Err(anyhow::anyhow!("无效的工具名格式: {}", tool_call.name));
                }
                let server_name = parts[1];
                let tool_name = parts[2];

                // 查找对应的执行器
                let executor = executors
                    .get(server_name)
                    .ok_or_else(|| anyhow::anyhow!("未找到 MCP 服务器: {}", server_name))?;

                // 在当前线程创建一个新的 tokio runtime 来执行异步调用
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;

                let local_tool_call = crate::tool::ToolCall {
                    name: tool_name.to_string(),
                    arguments: tool_call.arguments.clone(),
                };

                rt.block_on(executor.execute_async(&local_tool_call))
            };

        // Qwen ReAct: 完整 prompt 作为 user 消息，不使用 system prompt
        agent::chat_with_tools(&final_prompt, None, Some(&callback), Some(&tool_executor))
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
        .invoke_handler(tauri::generate_handler![
            init_agent,
            init_mcp,
            get_prompt,
            get_mcp_prompt,
            chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
