use crate::tool::{ToolCall, ToolResult};
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
use regex::Regex;
use std::num::NonZeroU32;
use std::path::PathBuf;

// 模型配置常量
const N_CTX: u32 = 32768;
const N_THREADS: i32 = 4;
const N_GPU_LAYERS: u32 = 99;
const TEMPERATURE: f32 = 0.2;
const TOP_P: f32 = 0.85;
const TOP_K: i32 = 20;
const MIN_P: f32 = 0.0;
const PRESENCE_PENALTY: f32 = 1.0;
const MAX_TOKENS: i32 = 4096;
const SEED: u32 = 1234;

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
    let agent = Agent::new(model_path)?;
    *AGENT.lock() = Some(agent);
    Ok(())
}

/// 使用全局 Agent 进行对话
pub fn chat(
    user_input: &str,
    system_prompt: Option<&str>,
    callback: Option<&dyn Fn(&str)>,
) -> Result<String> {
    let mut guard = AGENT.lock();
    let agent = guard.as_mut().context("Agent 未初始化")?;
    agent.chat(user_input, system_prompt, callback, None)
}

/// 使用全局 Agent 进行对话（带工具执行器）
pub fn chat_with_tools(
    user_input: &str,
    system_prompt: Option<&str>,
    callback: Option<&dyn Fn(&str)>,
    tool_executor: Option<&dyn Fn(&ToolCall) -> Result<ToolResult>>,
) -> Result<String> {
    let mut guard = AGENT.lock();
    let agent = guard.as_mut().context("Agent 未初始化")?;
    agent.chat(user_input, system_prompt, callback, tool_executor)
}

/// 解析工具调用（ReAct 格式：行动：工具名[{参数}]）
pub fn parse_tool_call(response: &str) -> Option<ToolCall> {
    // 匹配格式：行动：工具名[{参数}] 或 Action: tool_name[{params}]
    let re = Regex::new(r"(?:行动|Action)[\s：:]+([\w.]+)\s*\[([\s\S]*?)\]").ok()?;

    let captures = re.captures(response)?;
    let tool_name = captures.get(1)?.as_str().to_string();
    let args_str = captures.get(2)?.as_str().trim();

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

    /// 对话函数 - 支持工具调用循环
    pub fn chat(
        &mut self,
        user_input: &str,
        system_prompt: Option<&str>,
        callback: Option<&dyn Fn(&str)>,
        tool_executor: Option<&dyn Fn(&ToolCall) -> Result<ToolResult>>,
    ) -> Result<String> {
        #[cfg(debug_assertions)]
        if let Some(sp) = system_prompt {
            println!("[Agent] 系统提示词: {}", sp);
        }
        #[cfg(debug_assertions)]
        println!("[Agent] 用户消息: {}", user_input);

        // 构建消息历史
        let mut messages: Vec<(String, String)> = Vec::new();
        if let Some(sp) = system_prompt {
            messages.push(("system".to_string(), sp.to_string()));
        }
        messages.push(("user".to_string(), user_input.to_string()));

        let mut final_response = String::new();
        const MAX_TOOL_CALLS: usize = 10; // 防止无限循环

        for iteration in 0..MAX_TOOL_CALLS {
            // 构建 prompt
            let prompt = self.build_prompt_from_messages(&messages)?;

            // 生成回复
            #[cfg(debug_assertions)]
            print!("[Agent] 响应 #{}: ", iteration + 1);
            let response = self.generate(&prompt, callback)?;
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

                // 执行工具
                if let Some(executor) = tool_executor {
                    match executor(&tool_call) {
                        Ok(result) => {
                            #[cfg(debug_assertions)]
                            println!("[Agent] 工具结果: {}", result.result);

                            // 添加观察结果
                            let observation = format!("\n观察：{}", result.result);
                            if let Some(cb) = callback {
                                cb(&observation);
                            }
                            messages.push(("user".to_string(), format!("观察：{}", result.result)));
                        }
                        Err(e) => {
                            #[cfg(debug_assertions)]
                            println!("[Agent] 工具执行失败: {}", e);

                            let error_msg = format!("观察：工具执行失败 - {}", e);
                            if let Some(cb) = callback {
                                cb(&format!("\n{}", error_msg));
                            }
                            messages.push(("user".to_string(), error_msg));
                        }
                    }
                } else {
                    // 没有工具执行器，返回当前响应
                    final_response = response;
                    break;
                }
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
