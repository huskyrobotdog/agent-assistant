use crate::mcp::{McpClientConfig, McpTool, ToolCall, ToolResult};
use anyhow::{Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel, Special};
use llama_cpp_2::sampling::LlamaSampler;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::num::NonZeroU32;
use std::path::PathBuf;

// ReAct Prompt 模板（编译时嵌入）
pub const REACT_PROMPT_TEMPLATE: &str = "请尽可能回答以下问题。你可以使用以下工具：

{{TOOLS}}

请使用以下格式：

Question: 你需要回答的问题
Thought: 你需要思考下一步该做什么
Action: 要执行的动作，必须是 [{{TOOL_NAMES}}] 中的一个
Action Input: 动作的输入参数
Observation: 动作执行的结果
...（Thought/Action/Action Input/Observation 可以重复多次）
Thought: 我现在知道最终答案了
Final Answer: 原始问题的最终答案

开始！

Question: {{QUERY}}";

/// 根据工具信息生成工具描述
/// 参考：https://github.com/QwenLM/Qwen/blob/main/examples/react_prompt.md
fn format_tool_desc(tool: &McpTool) -> String {
    let params_str = serde_json::to_string(&tool.input_schema).unwrap_or_default();
    format!(
        "{name}: 调用此工具与 {name} API 进行交互。{name} API 有什么用？{desc} 参数：{params}",
        name = tool.name,
        desc = tool.description,
        params = params_str
    )
}

/// 构建服务器上下文提示词
fn build_server_context(configs: &[(String, McpClientConfig)]) -> String {
    if configs.is_empty() {
        return String::new();
    }

    let mut result = String::from("\n\n注意：以下 MCP 服务器已配置并连接：\n");
    for (name, config) in configs {
        result.push_str(&format!("- Server '{}':\n", name));
        result.push_str(&format!(
            "  command: {} {}\n",
            config.command,
            config.args.join(" ")
        ));
        if !config.env.is_empty() {
            result.push_str("  environment:\n");
            for (key, value) in &config.env {
                result.push_str(&format!("    {}: {}\n", key, value));
            }
        }
    }
    result.push_str("这些环境变量已预先配置，你无需再次提供。\n");
    result
}

/// 构建完整的 ReAct Prompt
pub fn build_react_prompt(
    query: &str,
    tools: &[McpTool],
    server_configs: &[(String, McpClientConfig)],
) -> String {
    let (tools_prompt, tool_names) = if tools.is_empty() {
        (String::new(), String::new())
    } else {
        let descs: Vec<String> = tools.iter().map(format_tool_desc).collect();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        (descs.join("\n\n"), names.join(","))
    };

    let context_prompt = build_server_context(server_configs);

    REACT_PROMPT_TEMPLATE
        .replace("{{TOOLS}}", &format!("{}{}", tools_prompt, context_prompt))
        .replace("{{TOOL_NAMES}}", &tool_names)
        .replace("{{QUERY}}", query)
}

// 模型配置常量
const N_CTX: u32 = 32768;
const N_THREADS: i32 = 4;
const N_GPU_LAYERS: u32 = 99;
const TEMPERATURE: f32 = 0.6;
const TOP_P: f32 = 0.95;
const TOP_K: i32 = 20;
const MIN_P: f32 = 0.0;
const PRESENCE_PENALTY: f32 = 1.0;
const MAX_TOKENS: i32 = 32768;
const SEED: u32 = 1234;
const MAX_TOOL_CALLS: usize = 1024;

/// 全局 Agent 单例
pub static AGENT: Lazy<Mutex<Option<Agent>>> = Lazy::new(|| Mutex::new(None));

/// Agent 实现 - 只初始化一次，复用 kvcache
pub struct Agent {
    model: &'static LlamaModel,
    ctx: LlamaContext<'static>,
}

// SAFETY: Agent 通过全局 Mutex 保护，同一时间只有一个线程可以访问
unsafe impl Send for Agent {}

/// 初始化全局 Agent（只调用一次）
pub fn init(model_path: PathBuf) -> Result<()> {
    let mut guard = AGENT.lock();
    if guard.is_some() {
        return Ok(()); // 已初始化，直接返回
    }
    let agent = Agent::new(model_path)?;
    *guard = Some(agent);
    Ok(())
}

/// 执行工具调用（通过 MCP_MANAGER）
fn execute_tool(tool_call: &ToolCall) -> Result<ToolResult> {
    tauri::async_runtime::block_on(
        crate::mcp::MCP_MANAGER.execute_tool(&tool_call.name, tool_call.arguments.clone()),
    )
}

/// 使用全局 Agent 进行对话
/// - query: 用户问题
/// - callback: 流式输出回调
pub fn chat(query: &str, callback: Option<&dyn Fn(&str)>) -> Result<String> {
    // 获取工具列表和服务器配置
    let tools = tauri::async_runtime::block_on(crate::mcp::MCP_MANAGER.get_all_tools());
    let server_configs =
        tauri::async_runtime::block_on(crate::mcp::MCP_MANAGER.get_server_configs());

    // 构建 ReAct Prompt
    let prompt = build_react_prompt(query, &tools, &server_configs);

    let mut guard = AGENT.lock();
    let agent = guard.as_mut().context("Agent 未初始化")?;

    // 如果没有工具，直接生成回复
    if tools.is_empty() {
        return agent.generate_simple(&prompt, callback);
    }

    // 有工具时，进入 ReAct 循环
    agent.react_loop(&prompt, callback)
}

/// 解析工具调用（Qwen ReAct 格式：Action: xxx\nAction Input: {...}）
/// 参考：https://github.com/QwenLM/Qwen/blob/main/examples/react_prompt.md
pub fn parse_tool_call(response: &str) -> Option<ToolCall> {
    // 查找 Action 和 Action Input 的位置
    // 注意：需要处理开头没有换行的情况
    let (i, action_prefix_len) = response
        .rfind("\nAction:")
        .map(|pos| (pos, "\nAction:".len()))
        .or_else(|| {
            if response.starts_with("Action:") {
                Some((0, "Action:".len()))
            } else {
                None
            }
        })?;

    let (j, input_prefix_len) = response
        .rfind("\nAction Input:")
        .map(|pos| (pos, "\nAction Input:".len()))
        .or_else(|| {
            if response.starts_with("Action Input:") {
                Some((0, "Action Input:".len()))
            } else {
                None
            }
        })?;

    // Action 必须在 Action Input 之前
    if i >= j {
        return None;
    }

    // 确定 Action Input 的结束位置
    // 如果没有 Observation 或 Observation 在 Action Input 之前，则使用文本末尾
    let k = response
        .rfind("\nObservation:")
        .filter(|&pos| pos > j)
        .unwrap_or(response.len());

    // 提取 Action 名称
    let action_start = i + action_prefix_len;
    let tool_name = response[action_start..j].trim().to_string();

    // 提取 Action Input（JSON 参数）
    let input_start = j + input_prefix_len;
    let args_str = response[input_start..k].trim();

    // 解析 JSON 参数
    let arguments: serde_json::Value = if args_str.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(args_str).unwrap_or_else(|_| serde_json::json!({}))
    };

    Some(ToolCall {
        name: tool_name,
        arguments,
    })
}

impl Agent {
    /// 创建新的 Agent（只调用一次）
    pub fn new(model_path: PathBuf) -> Result<Self> {
        // 禁用 llama 日志
        let log_options = llama_cpp_2::LogOptions::default().with_logs_enabled(false);
        llama_cpp_2::send_logs_to_tracing(log_options);

        // 使用 Box::leak 获取 'static 引用，单例模式下不会泄漏
        let backend = Box::leak(Box::new(
            LlamaBackend::init().context("初始化 llama 后端失败")?,
        ));

        let model_params = LlamaModelParams::default().with_n_gpu_layers(N_GPU_LAYERS);
        let model = Box::leak(Box::new(
            LlamaModel::load_from_file(backend, &model_path, &model_params)
                .context("加载模型失败")?,
        ));

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(NonZeroU32::new(N_CTX).unwrap()))
            .with_n_batch(N_CTX)
            .with_n_threads(N_THREADS)
            .with_n_threads_batch(N_THREADS);

        let ctx = model
            .new_context(backend, ctx_params)
            .context("创建上下文失败")?;

        Ok(Self { model, ctx })
    }

    /// 简单生成（无工具）
    pub fn generate_simple(
        &mut self,
        prompt: &str,
        callback: Option<&dyn Fn(&str)>,
    ) -> Result<String> {
        let messages = vec![("user".to_string(), prompt.to_string())];
        let full_prompt = self.build_prompt_from_messages(&messages)?;
        let response = self.generate(&full_prompt, callback)?;
        self.clear_kv_cache();
        Ok(response)
    }

    /// 总结对话历史（不包含初始 prompt，压缩上下文）
    fn summarize_history(&mut self, history: &[(String, String)]) -> Result<String> {
        if history.is_empty() {
            return Ok(String::new());
        }

        // 构建历史内容
        let history_text: String = history
            .iter()
            .map(|(role, content)| format!("{}: {}", role, content))
            .collect::<Vec<_>>()
            .join("\n");

        // 如果历史较短，直接返回
        if history_text.len() < 800 {
            return Ok(history_text);
        }

        let prompt = format!(
            "请简洁地总结以下对话历史，保留关键信息（工具调用、结果、重要数据）：\n\n\
            {}\n\n\
            总结：",
            history_text
        );

        let messages = vec![("user".to_string(), prompt)];
        let full_prompt = self.build_prompt_from_messages(&messages)?;
        let summary = self.generate(&full_prompt, None)?;
        self.clear_kv_cache();

        #[cfg(debug_assertions)]
        println!("[Agent] 历史总结: {}", summary.trim());

        Ok(summary.trim().to_string())
    }

    /// ReAct 循环（有工具时自动调用）
    pub fn react_loop(&mut self, prompt: &str, callback: Option<&dyn Fn(&str)>) -> Result<String> {
        #[cfg(debug_assertions)]
        {
            println!("[Agent] 开始 ReAct 循环");
            println!("[Agent] 初始 Prompt:\n{}", prompt);
        }

        // 构建消息历史
        let mut messages: Vec<(String, String)> = Vec::new();
        messages.push(("user".to_string(), prompt.to_string()));

        let mut final_response = String::new();

        for iteration in 0..MAX_TOOL_CALLS {
            // 构建 prompt
            let full_prompt = self.build_prompt_from_messages(&messages)?;

            #[cfg(debug_assertions)]
            if iteration > 0 {
                println!("[Agent] 增量消息: {:?}", messages.last().unwrap().1);
            }

            // 生成回复
            #[cfg(debug_assertions)]
            print!("[Agent] 响应 #{}: ", iteration + 1);
            let response = self.generate(&full_prompt, callback)?;
            #[cfg(debug_assertions)]
            println!();

            // 清空 kvcache
            self.clear_kv_cache();

            // 检测工具调用
            if let Some(tool_call) = parse_tool_call(&response) {
                #[cfg(debug_assertions)]
                println!(
                    "[Agent] 检测到工具调用: {} - {:?}",
                    tool_call.name, tool_call.arguments
                );

                // 添加 assistant 消息
                messages.push(("assistant".to_string(), response.clone()));

                // 执行工具（通过 MCP_MANAGER）
                match execute_tool(&tool_call) {
                    Ok(result) => {
                        #[cfg(debug_assertions)]
                        println!("[Agent] 工具结果: {}", result.result);

                        // 添加观察结果（Qwen ReAct 格式）
                        let observation = format!("\nObservation: {}", result.result);
                        if let Some(cb) = callback {
                            cb(&observation);
                        }
                        messages.push((
                            "user".to_string(),
                            format!("Observation: {}", result.result),
                        ));
                    }
                    Err(e) => {
                        #[cfg(debug_assertions)]
                        println!("[Agent] 工具执行失败: {}", e);

                        let error_msg = format!("Observation: 工具执行失败 - {}", e);
                        if let Some(cb) = callback {
                            cb(&format!("\n{}", error_msg));
                        }
                        messages.push(("user".to_string(), error_msg));
                    }
                }

                // // 总结消息历史（不包含初始 prompt）
                // if messages.len() > 2 {
                //     let history = &messages[1..];
                //     if let Ok(summary) = self.summarize_history(history) {
                //         // 保留初始 prompt，用总结替代其余消息
                //         let initial_prompt = messages[0].clone();
                //         messages.clear();
                //         messages.push(initial_prompt);
                //         messages
                //             .push(("user".to_string(), format!("之前步骤的总结：\n{}", summary)));
                //     }
                // }
            } else {
                // 没有工具调用，返回最终响应
                final_response = response;
                break;
            }
        }

        Ok(final_response)
    }

    /// 从消息历史构建 prompt
    fn build_prompt_from_messages(&self, messages: &[(String, String)]) -> Result<String> {
        let template = self
            .model
            .chat_template(None)
            .context("获取 chat template 失败")?;

        let chat_messages: Vec<LlamaChatMessage> = messages
            .iter()
            .map(|(role, content)| LlamaChatMessage::new(role.clone(), content.clone()))
            .collect::<Result<Vec<_>, _>>()
            .context("构建消息失败")?;

        self.model
            .apply_chat_template(&template, &chat_messages, true)
            .context("应用 chat template 失败")
    }

    /// 生成回复
    fn generate(&mut self, prompt: &str, callback: Option<&dyn Fn(&str)>) -> Result<String> {
        let tokens = self
            .model
            .str_to_token(prompt, AddBos::Never)
            .context("分词失败")?;

        let mut batch = LlamaBatch::new(N_CTX as usize, 1);

        let last_index = tokens.len() as i32 - 1;
        for (i, token) in tokens.iter().enumerate() {
            let is_last = i as i32 == last_index;
            batch
                .add(*token, i as i32, &[0], is_last)
                .context("添加 token 失败")?;
        }

        self.ctx.decode(&mut batch).context("解码失败")?;

        // 采样器
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::penalties(64, 1.1, 0.0, PRESENCE_PENALTY),
            LlamaSampler::temp(TEMPERATURE),
            LlamaSampler::top_k(TOP_K),
            LlamaSampler::top_p(TOP_P, 1),
            LlamaSampler::min_p(MIN_P, 1),
            LlamaSampler::dist(SEED),
        ]);

        let mut output = String::new();
        let mut n_cur = batch.n_tokens();
        let mut decoder = encoding_rs::UTF_8.new_decoder();

        while n_cur < MAX_TOKENS {
            let token = sampler.sample(&self.ctx, batch.n_tokens() - 1);
            sampler.accept(token);

            if self.model.is_eog_token(token) {
                break;
            }

            let token_bytes = self
                .model
                .token_to_bytes(token, Special::Tokenize)
                .context("转换 token 失败")?;

            let mut token_str = String::with_capacity(32);
            let _ = decoder.decode_to_string(&token_bytes, &mut token_str, false);

            output.push_str(&token_str);

            // 检测 stop word: "Observation" 或 "Observation:"
            if output.ends_with("\nObservation:") || output.ends_with("\nObservation") {
                // 移除 stop word
                if output.ends_with("\nObservation:") {
                    output.truncate(output.len() - "\nObservation:".len());
                } else {
                    output.truncate(output.len() - "\nObservation".len());
                }
                break;
            }

            // 调试打印流式输出
            #[cfg(debug_assertions)]
            {
                use std::io::Write;
                print!("{}", token_str);
                let _ = std::io::stdout().flush();
            }

            // 回调
            if let Some(cb) = callback {
                cb(&token_str);
            }

            batch.clear();
            batch
                .add(token, n_cur, &[0], true)
                .context("添加 token 失败")?;

            self.ctx.decode(&mut batch).context("解码失败")?;

            n_cur += 1;
        }

        Ok(output)
    }

    /// 清空 kvcache
    pub fn clear_kv_cache(&mut self) {
        self.ctx.clear_kv_cache();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(N_CTX, 32768);
        assert_eq!(TEMPERATURE, 0.2);
    }
}
