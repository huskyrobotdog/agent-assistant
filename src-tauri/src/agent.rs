use crate::tool::{McpTool, McpToolExecutor, ToolCall, ToolResult};
use anyhow::{Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel, Special};
use llama_cpp_2::sampling::LlamaSampler;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// ReAct 系统提示词模板
const REACT_PROMPT: &str = include_str!("../resources/prompt/agent.md");

/// 全局 Agent 单例
pub static AGENT: Lazy<RwLock<Option<CoTAgent>>> = Lazy::new(|| RwLock::new(None));

/// 初始化全局 Agent 单例
pub fn init_agent_singleton(config: AgentConfig) -> Result<()> {
    let agent = CoTAgent::new(config)?;
    *AGENT.write() = Some(agent);
    Ok(())
}

/// 获取全局 Agent 引用（如果已初始化）
pub fn get_agent() -> Option<parking_lot::RwLockReadGuard<'static, Option<CoTAgent>>> {
    let guard = AGENT.read();
    if guard.is_some() {
        Some(guard)
    } else {
        None
    }
}

/// Agent 消息角色
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Agent 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Agent 配置
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub model_path: PathBuf,
    pub n_ctx: u32,
    pub n_threads: i32,
    pub n_gpu_layers: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub min_p: f32,
    pub presence_penalty: f32,
    pub max_tokens: i32,
    pub seed: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::from("models/Qwen3-4B-Thinking-2507-UD-IQ1_M.gguf"),
            n_ctx: 32768,
            n_threads: 4,
            n_gpu_layers: 99,
            temperature: 0.2, // 低温度：更确定性的输出
            top_p: 0.85,      // 较低的 top_p 减少随机性
            top_k: 20,
            min_p: 0.0,            // Qwen3 推荐 0.0
            presence_penalty: 1.0, // Qwen3 建议 ≤ 2.0，降低以保持输出质量
            max_tokens: 4096,
            seed: 1234,
        }
    }
}

/// CoT Agent 状态
#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    Idle,
    Planning,
    Executing,
    Observing,
    Summarizing,
    Finished,
    Error,
}

/// Agent 生成回调
pub type GenerationCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Agent 生成回调引用
pub type GenerationCallbackRef<'a> = Option<&'a dyn Fn(&str)>;

/// 工具结果回调引用
pub type ToolResultCallbackRef<'a> = Option<&'a dyn Fn(&str, &str, bool)>;

/// CoT Agent 实现（任务规划与思维链）
pub struct CoTAgent {
    backend: LlamaBackend,
    model: LlamaModel,
    config: AgentConfig,
    messages: RwLock<Vec<Message>>,
    tools: RwLock<Vec<McpTool>>,
    tool_executors: RwLock<HashMap<String, Arc<dyn McpToolExecutor>>>,
    state: RwLock<AgentState>,
    /// 自定义上下文信息（如 MCP 环境变量配置）
    context: RwLock<String>,
    /// 缓存的 token 数量（用于增量处理）
    cached_token_count: Mutex<usize>,
}

impl CoTAgent {
    /// 创建新的 Agent
    pub fn new(config: AgentConfig) -> Result<Self> {
        // 禁用 llama 日志
        let log_options = llama_cpp_2::LogOptions::default().with_logs_enabled(false);
        llama_cpp_2::send_logs_to_tracing(log_options);

        let backend = LlamaBackend::init().context("初始化 llama 后端失败")?;

        let model_params = LlamaModelParams::default().with_n_gpu_layers(config.n_gpu_layers);

        let model = LlamaModel::load_from_file(&backend, &config.model_path, &model_params)
            .context("加载模型失败")?;

        Ok(Self {
            backend,
            model,
            config,
            messages: RwLock::new(Vec::new()),
            tools: RwLock::new(Vec::new()),
            tool_executors: RwLock::new(HashMap::new()),
            state: RwLock::new(AgentState::Idle),
            context: RwLock::new(String::new()),
            cached_token_count: Mutex::new(0),
        })
    }

    /// 注册 MCP 工具执行器
    pub fn register_tool_executor(&self, name: &str, executor: Arc<dyn McpToolExecutor>) {
        let mut executors = self.tool_executors.write();
        let mut tools = self.tools.write();

        for tool in executor.get_tools() {
            tools.push(tool);
        }
        executors.insert(name.to_string(), executor);
    }

    /// 注册单个 MCP 工具（用于异步场景，带命名空间前缀）
    pub fn register_mcp_tool(&self, tool: McpTool, namespace: &str) {
        let mut tools = self.tools.write();
        let namespaced_tool = McpTool {
            name: format!("{}.{}", namespace, tool.name),
            description: tool.description,
            input_schema: tool.input_schema,
        };
        tools.push(namespaced_tool);
    }

    /// 设置自定义上下文（如 MCP 环境变量配置）
    pub fn set_context(&self, ctx: &str) {
        let mut context = self.context.write();
        *context = ctx.to_string();
    }

    /// 追加上下文信息
    pub fn append_context(&self, ctx: &str) {
        let mut context = self.context.write();
        if !context.is_empty() {
            context.push('\n');
        }
        context.push_str(ctx);
    }

    /// 准备对话（设置系统提示词并添加用户消息）
    pub fn prepare_chat(&self, user_input: &str) {
        if self.messages.read().is_empty()
            || !self.messages.read().iter().any(|m| m.role == Role::System)
        {
            self.set_system_prompt(&self.build_cot_system_prompt());
        }
        self.add_user_message(user_input);
    }

    /// 执行单步生成（返回响应和工具调用）
    pub fn generate_step(
        &self,
        callback: Option<&dyn Fn(&str)>,
    ) -> Result<(String, Vec<ToolCall>)> {
        let response = self.generate_with_callback(callback)?;

        // 如果响应包含"总结"，视为任务完成，不再解析工具调用
        if response.contains("总结：")
            || response.contains("总结:")
            || response.contains("Summary:")
        {
            #[cfg(debug_assertions)]
            println!("\n✅ [检测到总结] 任务完成");
            return Ok((response, Vec::new()));
        }

        let tool_calls = self.parse_tool_calls(&response);
        Ok((response, tool_calls))
    }

    /// 添加助手响应到对话历史
    pub fn add_assistant_response(&self, response: &str) {
        let mut messages = self.messages.write();
        messages.push(Message {
            role: Role::Assistant,
            content: response.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    /// 添加带工具调用的助手响应
    pub fn add_assistant_response_with_tools(&self, response: &str, tool_calls: Vec<ToolCall>) {
        let mut messages = self.messages.write();
        messages.push(Message {
            role: Role::Assistant,
            content: response.to_string(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        });
    }

    /// 添加工具执行结果到对话历史
    pub fn add_tool_result(&self, tool_name: &str, result: &ToolResult) {
        let truncated_result = Self::truncate_result(&result.result, Self::MAX_TOOL_RESULT_LENGTH);
        let mut messages = self.messages.write();
        messages.push(Message {
            role: Role::Tool,
            content: format!("Observation: {}", truncated_result),
            tool_calls: None,
            tool_call_id: Some(tool_name.to_string()),
        });
    }

    /// 设置系统提示词
    pub fn set_system_prompt(&self, prompt: &str) {
        let mut messages = self.messages.write();
        messages.retain(|m| m.role != Role::System);
        messages.insert(
            0,
            Message {
                role: Role::System,
                content: prompt.to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        );
    }

    /// 构建 ReAct 系统提示词
    fn build_cot_system_prompt(&self) -> String {
        let tools = self.tools.read();
        let context = self.context.read();

        // 构建工具描述
        let tool_descs: Vec<String> = tools
            .iter()
            .map(|t| {
                format!(
                    "- `{}[参数]`：{}\n  参数格式：{}",
                    t.name,
                    t.description,
                    serde_json::to_string(&t.input_schema).unwrap_or_default()
                )
            })
            .collect();
        let tools_section = if tool_descs.is_empty() {
            String::new()
        } else {
            format!("\n\n可用的行动类型包括：\n{}", tool_descs.join("\n\n"))
        };

        // 构建上下文信息（如 MCP 环境变量配置）
        let context_section = if context.is_empty() {
            String::new()
        } else {
            format!("{}", *context)
        };

        REACT_PROMPT
            .replace("{{TOOLS}}", &tools_section)
            .replace("{{CONTEXT}}", &context_section)
    }

    /// 添加用户消息
    pub fn add_user_message(&self, content: &str) {
        let mut messages = self.messages.write();
        messages.push(Message {
            role: Role::User,
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    /// 构建对话上下文
    fn build_prompt(&self) -> Result<String> {
        let messages = self.messages.read();

        let template = self
            .model
            .chat_template(None)
            .context("获取 chat template 失败")?;

        let chat_messages: Result<Vec<_>> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                LlamaChatMessage::new(role.to_string(), m.content.clone()).context("创建消息失败")
            })
            .collect();

        let chat_messages = chat_messages?;

        self.model
            .apply_chat_template(&template, &chat_messages, true)
            .context("应用 chat template 失败")
    }

    /// 生成回复
    pub fn generate(&self) -> Result<String> {
        self.generate_with_callback(None)
    }

    /// 生成回复（带回调）
    pub fn generate_with_callback(&self, callback: Option<&dyn Fn(&str)>) -> Result<String> {
        *self.state.write() = AgentState::Planning;

        #[cfg(debug_assertions)]
        {
            let messages = self.messages.read();

            // 只有第一次（只有系统提示词和用户消息）时打印完整调试信息
            let is_first_turn = messages.len() <= 2;

            if is_first_turn {
                println!("\n════════════════════ 调试信息 ════════════════════");

                // 1. 打印系统提示词
                if let Some(sys_msg) = messages.iter().find(|m| m.role == Role::System) {
                    println!("\n📋 [系统提示词]\n{}", sys_msg.content);
                }

                // 2. 打印用户输入
                if let Some(user_msg) = messages.iter().rev().find(|m| m.role == Role::User) {
                    println!("\n💬 [用户输入]\n{}", user_msg.content);
                }
            } else {
                // 后续轮次只打印简短信息
                println!("\n🔄 [继续推理] 当前消息数: {}", messages.len());
            }

            println!("\n🧠 [AI 回复]");
        }

        let prompt = self.build_prompt()?;

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(NonZeroU32::new(self.config.n_ctx).unwrap()))
            .with_n_batch(self.config.n_ctx) // 设置 n_batch 等于 n_ctx，避免 token 超限
            .with_n_threads(self.config.n_threads)
            .with_n_threads_batch(self.config.n_threads);

        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .context("创建上下文失败")?;

        let tokens = self
            .model
            .str_to_token(&prompt, AddBos::Never) // chat template 已添加 BOS
            .context("分词失败")?;

        let mut batch = LlamaBatch::new(self.config.n_ctx as usize, 1);

        let last_index = tokens.len() as i32 - 1;
        for (i, token) in tokens.iter().enumerate() {
            let is_last = i as i32 == last_index;
            batch
                .add(*token, i as i32, &[0], is_last)
                .context("添加 token 失败")?;
        }

        ctx.decode(&mut batch).context("解码失败")?;

        // Qwen3 推荐采样顺序: temp → top_k → top_p → min_p → dist
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::penalties(
                64,                           // 惩罚窗口大小
                1.1,                          // repeat_penalty
                0.0,                          // frequency_penalty
                self.config.presence_penalty, // presence_penalty
            ),
            LlamaSampler::temp(self.config.temperature),
            LlamaSampler::top_k(self.config.top_k),
            LlamaSampler::top_p(self.config.top_p, 1),
            LlamaSampler::min_p(self.config.min_p, 1), // Qwen3 推荐 0.0
            LlamaSampler::dist(self.config.seed),
        ]);

        let mut output = String::new();
        let mut n_cur = batch.n_tokens();
        let mut decoder = encoding_rs::UTF_8.new_decoder();

        // CoT 模式的 stop word：当模型生成 "Result:" 时停止，等待真正的工具结果
        const STOP_WORD: &str = "Result:";

        while n_cur < self.config.max_tokens {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
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

            // 检查 stop word：当检测到 "Result:" 时停止生成
            if output.contains(STOP_WORD) {
                // 移除 stop word，让真正的工具结果来填充
                if let Some(pos) = output.find(STOP_WORD) {
                    output.truncate(pos);
                }
                #[cfg(debug_assertions)]
                println!("\n\n🛑 [Stop Word] 检测到 Result，停止生成");
                break;
            }

            #[cfg(debug_assertions)]
            {
                use std::io::Write;
                print!("{}", token_str);
                let _ = std::io::stdout().flush();
            }

            if let Some(cb) = callback {
                cb(&token_str);
            }

            batch.clear();
            batch
                .add(token, n_cur, &[0], true)
                .context("添加 token 失败")?;

            ctx.decode(&mut batch).context("解码失败")?;

            n_cur += 1;
        }

        let generated_tokens = n_cur - tokens.len() as i32;

        #[cfg(debug_assertions)]
        println!("\n\n✅ [推理完成] 共生成 {} 个 token", generated_tokens);

        // 如果生成 0 token，可能是上下文过长导致模型困惑
        if generated_tokens == 0 && output.is_empty() {
            #[cfg(debug_assertions)]
            println!("\n⚠️ [警告] 模型生成 0 token，可能是上下文过长");

            // 返回提示信息而不是空字符串
            output = "[模型无法生成响应，请尝试清空对话或简化问题]".to_string();
        }

        *self.state.write() = AgentState::Idle;
        Ok(output)
    }

    /// 解析工具调用 (支持多种格式)
    fn parse_tool_calls(&self, response: &str) -> Vec<ToolCall> {
        let mut tool_calls = Vec::new();

        // 格式 1: 中文 ReAct 风格 行动：tool_name[{...}] (优先，支持命名空间如 mcp.mysql.connect_db)
        let cn_react_re = regex::Regex::new(r"行动[：:]\s*([\w.]+)\s*\[(\{.*?\})\]").ok();
        if let Some(re) = cn_react_re {
            for cap in re.captures_iter(response) {
                if let (Some(name), Some(args)) = (cap.get(1), cap.get(2)) {
                    if let Ok(arguments) = serde_json::from_str(args.as_str().trim()) {
                        tool_calls.push(ToolCall {
                            name: name.as_str().trim().to_string(),
                            arguments,
                        });
                    }
                }
            }
        }

        // 格式 2: CoT 风格 Tool/Tool Input
        if tool_calls.is_empty() {
            let cot_re =
                regex::Regex::new(r"(?s)Tool:[ \t]*(\S+)[ \t]*\nTool Input:[ \t]*(\{.*?\})").ok();
            if let Some(re) = cot_re {
                for cap in re.captures_iter(response) {
                    if let (Some(name), Some(args)) = (cap.get(1), cap.get(2)) {
                        if let Ok(arguments) = serde_json::from_str(args.as_str().trim()) {
                            tool_calls.push(ToolCall {
                                name: name.as_str().trim().to_string(),
                                arguments,
                            });
                        }
                    }
                }
            }
        }

        // 格式 3: ReAct 风格 Action/Action Input (兼容旧格式)
        if tool_calls.is_empty() {
            let react_re =
                regex::Regex::new(r"(?s)Action:[ \t]*(\S+)[ \t]*\nAction Input:[ \t]*(\{.*?\})")
                    .ok();
            if let Some(re) = react_re {
                for cap in re.captures_iter(response) {
                    if let (Some(name), Some(args)) = (cap.get(1), cap.get(2)) {
                        if let Ok(arguments) = serde_json::from_str(args.as_str().trim()) {
                            tool_calls.push(ToolCall {
                                name: name.as_str().trim().to_string(),
                                arguments,
                            });
                        }
                    }
                }
            }
        }

        // 格式 3: <tool_call>...</tool_call> (Hermes-style，备用)
        if tool_calls.is_empty() {
            let re = regex::Regex::new(r"(?s)<tool_call>\s*(\{.*?\})\s*</tool_call>").ok();
            if let Some(re) = re {
                for cap in re.captures_iter(response) {
                    if let Some(json_str) = cap.get(1) {
                        let cleaned = json_str.as_str().trim();
                        if let Ok(tool_call) = serde_json::from_str::<ToolCall>(cleaned) {
                            tool_calls.push(tool_call);
                        }
                    }
                }
            }
        }

        // 格式 3: 直接 JSON 对象 (最后备用)
        if tool_calls.is_empty() {
            let json_re = regex::Regex::new(
                r#"(?s)\{\s*"name"\s*:\s*"([^"]+)"\s*,\s*"arguments"\s*:\s*(\{.*?\})\s*\}"#,
            )
            .ok();
            if let Some(re) = json_re {
                for cap in re.captures_iter(response) {
                    if let (Some(name), Some(args)) = (cap.get(1), cap.get(2)) {
                        if let Ok(arguments) = serde_json::from_str(args.as_str().trim()) {
                            tool_calls.push(ToolCall {
                                name: name.as_str().to_string(),
                                arguments,
                            });
                        }
                    }
                }
            }
        }

        tool_calls
    }

    /// 检查是否包含 Final Answer
    #[allow(dead_code)]
    fn has_final_answer(&self, response: &str) -> bool {
        response.contains("Final Answer:")
    }

    /// 提取思考内容
    #[allow(dead_code)]
    fn extract_thinking(&self, response: &str) -> Option<String> {
        let re = regex::Regex::new(r"<think>([\s\S]*?)</think>").ok()?;
        re.captures(response)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
    }

    /// 执行单次 CoT 循环
    pub fn step(&self) -> Result<(String, bool)> {
        self.step_with_callbacks(None, None)
    }

    /// 执行单次 CoT 循环（带回调）
    pub fn step_with_callback(&self, callback: Option<&dyn Fn(&str)>) -> Result<(String, bool)> {
        self.step_with_callbacks(callback, None)
    }

    /// 执行单次 CoT 循环（带生成回调和工具结果回调）
    pub fn step_with_callbacks(
        &self,
        callback: Option<&dyn Fn(&str)>,
        tool_callback: Option<&dyn Fn(&str, &str, bool)>,
    ) -> Result<(String, bool)> {
        #[cfg(debug_assertions)]
        println!("\n🔄 [CoT Step] 开始执行单次循环");

        let response = self.generate_with_callback(callback)?;

        let tool_calls = self.parse_tool_calls(&response);

        #[cfg(debug_assertions)]
        if !tool_calls.is_empty() {
            println!("\n🔧 [检测到工具调用] 共 {} 个", tool_calls.len());
            for (i, tc) in tool_calls.iter().enumerate() {
                println!("  [{}/{}] 工具: {}", i + 1, tool_calls.len(), tc.name);
                println!(
                    "        参数: {}",
                    serde_json::to_string_pretty(&tc.arguments).unwrap_or_default()
                );
            }
        }

        if !tool_calls.is_empty() {
            *self.state.write() = AgentState::Executing;

            {
                let mut messages = self.messages.write();
                messages.push(Message {
                    role: Role::Assistant,
                    content: response.clone(),
                    tool_calls: Some(tool_calls.clone()),
                    tool_call_id: None,
                });
            }

            for tool_call in &tool_calls {
                let result = self.execute_tool(tool_call)?;

                // 通过回调发送工具执行结果
                if let Some(cb) = tool_callback {
                    cb(&result.tool_name, &result.result, result.is_error);
                }

                // 使用 Result 格式（CoT 风格）
                let mut messages = self.messages.write();
                messages.push(Message {
                    role: Role::Tool,
                    content: format!("Result: {}", result.result),
                    tool_calls: None,
                    tool_call_id: Some(tool_call.name.clone()),
                });
            }

            *self.state.write() = AgentState::Observing;
            Ok((response, false))
        } else {
            {
                let mut messages = self.messages.write();
                messages.push(Message {
                    role: Role::Assistant,
                    content: response.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }

            *self.state.write() = AgentState::Finished;
            Ok((response, true))
        }
    }

    /// 工具结果最大长度（字符数）
    const MAX_TOOL_RESULT_LENGTH: usize = 2000;

    /// 截断工具结果，避免 token 超限
    fn truncate_result(result: &str, max_len: usize) -> String {
        if result.len() <= max_len {
            return result.to_string();
        }

        // 按字符边界截断
        let truncated: String = result.chars().take(max_len).collect();
        format!(
            "{}...\n\n[结果已截断，原长度: {} 字符]",
            truncated,
            result.len()
        )
    }

    /// 执行工具调用（支持命名空间格式如 mcp.mysql.connect_db）
    fn execute_tool(&self, tool_call: &ToolCall) -> Result<ToolResult> {
        #[cfg(debug_assertions)]
        println!(
            "\n⚡ [执行工具] {} 参数: {}",
            tool_call.name, tool_call.arguments
        );

        // 从命名空间格式中提取原始工具名（mcp.mysql.connect_db -> connect_db）
        let original_tool_name = tool_call.name.rsplit('.').next().unwrap_or(&tool_call.name);

        // 创建使用原始工具名的 ToolCall
        let original_tool_call = ToolCall {
            name: original_tool_name.to_string(),
            arguments: tool_call.arguments.clone(),
        };

        let executor_opt = {
            let executors = self.tool_executors.read();
            executors
                .iter()
                .find(|(_, executor)| {
                    executor
                        .get_tools()
                        .iter()
                        .any(|t| t.name == original_tool_name)
                })
                .map(|(_, executor)| executor.clone())
        };

        if let Some(executor) = executor_opt {
            let result = executor.execute(&original_tool_call);
            #[cfg(debug_assertions)]
            if let Ok(ref r) = result {
                println!("\n📤 [工具结果] {}", r.tool_name);
                println!("{}", r.result);
            }
            // 截断过长的结果
            return result.map(|mut r| {
                r.result = Self::truncate_result(&r.result, Self::MAX_TOOL_RESULT_LENGTH);
                r
            });
        }

        #[cfg(debug_assertions)]
        println!("\n❌ [工具未找到] {}", tool_call.name);

        Ok(ToolResult {
            tool_name: tool_call.name.clone(),
            result: format!("工具 {} 未找到", tool_call.name),
            is_error: true,
        })
    }

    /// 运行完整的 CoT 循环
    pub fn run(&self, user_input: &str, max_iterations: usize) -> Result<String> {
        self.run_with_callbacks(user_input, max_iterations, None, None)
    }

    /// 运行完整的 CoT 循环（带回调）
    pub fn run_with_callback(
        &self,
        user_input: &str,
        max_iterations: usize,
        callback: Option<&dyn Fn(&str)>,
    ) -> Result<String> {
        self.run_with_callbacks(user_input, max_iterations, callback, None)
    }

    /// 运行完整的 CoT 循环（带生成回调和工具结果回调）
    pub fn run_with_callbacks(
        &self,
        user_input: &str,
        max_iterations: usize,
        callback: Option<&dyn Fn(&str)>,
        tool_callback: Option<&dyn Fn(&str, &str, bool)>,
    ) -> Result<String> {
        #[cfg(debug_assertions)]
        println!("\n\n🚀 ================== CoT Agent 开始 ==================");
        #[cfg(debug_assertions)]
        println!("📊 [最大迭代次数] {}", max_iterations);

        if self.messages.read().is_empty()
            || !self.messages.read().iter().any(|m| m.role == Role::System)
        {
            self.set_system_prompt(&self.build_cot_system_prompt());
            #[cfg(debug_assertions)]
            println!("📋 [系统提示词已设置]");
        }

        self.add_user_message(user_input);

        let mut final_response = String::new();
        let mut iterations = 0;

        loop {
            if iterations >= max_iterations {
                #[cfg(debug_assertions)]
                println!("\n⚠️ [达到最大迭代次数] {}", max_iterations);
                break;
            }

            #[cfg(debug_assertions)]
            println!("\n🔁 [迭代] {}/{}", iterations + 1, max_iterations);

            let (response, is_done) = self.step_with_callbacks(callback, tool_callback)?;

            final_response = response;

            if is_done {
                #[cfg(debug_assertions)]
                println!("\n✅ [任务完成]");
                break;
            }

            iterations += 1;
        }

        #[cfg(debug_assertions)]
        println!("\n🏁 ================== CoT Agent 结束 ==================\n");

        Ok(final_response)
    }

    /// 清空对话历史
    pub fn clear_history(&self) {
        let mut messages = self.messages.write();
        messages.retain(|m| m.role == Role::System);
        // 重置 token 缓存
        *self.cached_token_count.lock().unwrap() = 0;
    }

    /// 获取当前状态
    pub fn get_state(&self) -> AgentState {
        self.state.read().clone()
    }

    /// 获取对话历史
    pub fn get_messages(&self) -> Vec<Message> {
        self.messages.read().clone()
    }

    /// 获取配置的上下文长度
    pub fn get_context_length(&self) -> u32 {
        self.config.n_ctx
    }

    /// 获取当前使用的 token 数量
    pub fn get_current_tokens(&self) -> Result<usize> {
        let prompt = self.build_prompt()?;
        let tokens = self
            .model
            .str_to_token(&prompt, AddBos::Never)
            .context("分词失败")?;
        Ok(tokens.len())
    }

    /// 获取当前 prompt 的字符数
    pub fn get_current_chars(&self) -> Result<usize> {
        let prompt = self.build_prompt()?;
        Ok(prompt.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.n_ctx, 8192);
        assert_eq!(config.temperature, 0.6);
    }

    #[test]
    fn test_parse_tool_calls() {
        let _response = r#"
我需要调用工具来完成任务。
<tool_call>
{"name": "echo", "arguments": {"message": "hello"}}
</tool_call>
"#;

        let _agent = CoTAgent::new(AgentConfig {
            model_path: PathBuf::from("test.gguf"),
            ..Default::default()
        });

        // 注意：这个测试在没有实际模型时会失败
        // 实际使用时需要有效的模型文件
    }
}
