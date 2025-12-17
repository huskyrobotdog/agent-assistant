use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel, Special};
use llama_cpp_2::sampling::LlamaSampler;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

/// Agent 错误类型
#[derive(Error, Debug)]
pub enum AgentError {
    #[error("模型加载失败: {0}")]
    ModelLoadError(String),
    #[error("推理错误: {0}")]
    InferenceError(String),
    #[error("工具执行错误: {0}")]
    ToolExecutionError(String),
    #[error("解析错误: {0}")]
    ParseError(String),
    #[error("MCP 错误: {0}")]
    McpError(String),
}

/// MCP 工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// MCP 工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// MCP 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub result: String,
    pub is_error: bool,
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
            n_ctx: 8192,
            n_threads: 4,
            n_gpu_layers: 99,
            temperature: 0.6,
            top_p: 0.95,
            top_k: 20,
            min_p: 0.0,            // Qwen3 推荐 0.0
            presence_penalty: 1.0, // Qwen3 建议 ≤ 2.0，降低以保持输出质量
            max_tokens: 4096,
            seed: 1234,
        }
    }
}

/// ReAct Agent 状态
#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    Idle,
    Thinking,
    Acting,
    Observing,
    Finished,
    Error,
}

/// Agent 生成回调
pub type GenerationCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Agent 生成回调引用
pub type GenerationCallbackRef<'a> = Option<&'a dyn Fn(&str)>;

/// 工具结果回调引用
pub type ToolResultCallbackRef<'a> = Option<&'a dyn Fn(&str, &str, bool)>;

/// MCP 工具执行器 trait
pub trait McpToolExecutor: Send + Sync {
    fn execute(&self, tool_call: &ToolCall) -> Result<ToolResult, AgentError>;
    fn get_tools(&self) -> Vec<McpTool>;
}

/// ReAct Agent 实现
pub struct ReactAgent {
    backend: LlamaBackend,
    model: LlamaModel,
    config: AgentConfig,
    messages: RwLock<Vec<Message>>,
    tools: RwLock<Vec<McpTool>>,
    tool_executors: RwLock<HashMap<String, Arc<dyn McpToolExecutor>>>,
    state: RwLock<AgentState>,
}

impl ReactAgent {
    /// 创建新的 Agent
    pub fn new(config: AgentConfig) -> Result<Self, AgentError> {
        // 禁用 llama 日志
        let log_options = llama_cpp_2::LogOptions::default().with_logs_enabled(false);
        llama_cpp_2::send_logs_to_tracing(log_options);

        let backend = LlamaBackend::init()
            .map_err(|e| AgentError::ModelLoadError(format!("初始化 llama 后端失败: {}", e)))?;

        let model_params = LlamaModelParams::default().with_n_gpu_layers(config.n_gpu_layers);

        let model = LlamaModel::load_from_file(&backend, &config.model_path, &model_params)
            .map_err(|e| AgentError::ModelLoadError(format!("加载模型失败: {}", e)))?;

        Ok(Self {
            backend,
            model,
            config,
            messages: RwLock::new(Vec::new()),
            tools: RwLock::new(Vec::new()),
            tool_executors: RwLock::new(HashMap::new()),
            state: RwLock::new(AgentState::Idle),
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

    /// 构建 Hermes-style 系统提示词 (Qwen3 推荐格式)
    fn build_react_system_prompt(&self) -> String {
        let tools = self.tools.read();
        let tools_json = if tools.is_empty() {
            "[]".to_string()
        } else {
            let tools_array: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema
                        }
                    })
                })
                .collect();
            serde_json::to_string_pretty(&tools_array).unwrap_or_default()
        };

        format!(
            r#"You are a function calling AI model. You are provided with function signatures within <tools></tools> XML tags. You may call one or more functions to assist with the user query. Don't make assumptions about what values to plug into functions.

<tools>
{tools_json}
</tools>

For each function call, return a JSON object with function name and arguments within <tool_call></tool_call> XML tags:

<tool_call>
{{"name": "function_name", "arguments": {{"param": "value"}}}}
</tool_call>

After calling a tool, you will receive the result within <tool_response></tool_response> XML tags.

Guidelines:
- Call only ONE tool at a time
- Wait for tool results in <tool_response> before proceeding
- If a tool fails, try alternative approaches
- Provide concise final answers
- Always respond in the same language as the user's query"#,
            tools_json = tools_json
        )
    }

    /// 添加用户消息
    pub fn add_user_message(&self, content: &str) {
        #[cfg(debug_assertions)]
        println!("\n💬 [用户输入] {}", content);

        let mut messages = self.messages.write();
        messages.push(Message {
            role: Role::User,
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    /// 构建对话上下文
    fn build_prompt(&self) -> Result<String, AgentError> {
        let messages = self.messages.read();

        let template = self
            .model
            .chat_template(None)
            .map_err(|e| AgentError::InferenceError(format!("获取 chat template 失败: {}", e)))?;

        let chat_messages: Result<Vec<_>, _> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                LlamaChatMessage::new(role.to_string(), m.content.clone())
                    .map_err(|e| AgentError::InferenceError(format!("创建消息失败: {}", e)))
            })
            .collect();

        let chat_messages = chat_messages?;

        self.model
            .apply_chat_template(&template, &chat_messages, true)
            .map_err(|e| AgentError::InferenceError(format!("应用 chat template 失败: {}", e)))
    }

    /// 生成回复
    pub fn generate(&self) -> Result<String, AgentError> {
        self.generate_with_callback(None)
    }

    /// 生成回复（带回调）
    pub fn generate_with_callback(
        &self,
        callback: Option<&dyn Fn(&str)>,
    ) -> Result<String, AgentError> {
        *self.state.write() = AgentState::Thinking;

        #[cfg(debug_assertions)]
        println!("\n🧠 [开始推理]");

        let prompt = self.build_prompt()?;

        #[cfg(debug_assertions)]
        {
            let char_count = prompt.chars().count();
            println!("\n📝 [Prompt 长度] {} 字符", char_count);
            // 打印 prompt 的最后 200 个字符（避免输出过多）
            if char_count > 200 {
                let tail: String = prompt.chars().skip(char_count - 200).collect();
                println!("\n📝 [Prompt 末尾] ...{}", tail);
            } else {
                println!("\n📝 [Prompt] {}", prompt);
            }
        }

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(NonZeroU32::new(self.config.n_ctx).unwrap()))
            .with_n_threads(self.config.n_threads)
            .with_n_threads_batch(self.config.n_threads);

        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| AgentError::InferenceError(format!("创建上下文失败: {}", e)))?;

        let tokens = self
            .model
            .str_to_token(&prompt, AddBos::Never) // chat template 已添加 BOS
            .map_err(|e| AgentError::InferenceError(format!("分词失败: {}", e)))?;

        let mut batch = LlamaBatch::new(self.config.n_ctx as usize, 1);

        let last_index = tokens.len() as i32 - 1;
        for (i, token) in tokens.iter().enumerate() {
            let is_last = i as i32 == last_index;
            batch
                .add(*token, i as i32, &[0], is_last)
                .map_err(|e| AgentError::InferenceError(format!("添加 token 失败: {}", e)))?;
        }

        ctx.decode(&mut batch)
            .map_err(|e| AgentError::InferenceError(format!("解码失败: {}", e)))?;

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

        while n_cur < self.config.max_tokens {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);

            if self.model.is_eog_token(token) {
                break;
            }

            let token_bytes = self
                .model
                .token_to_bytes(token, Special::Tokenize)
                .map_err(|e| AgentError::InferenceError(format!("转换 token 失败: {}", e)))?;

            let mut token_str = String::with_capacity(32);
            let _ = decoder.decode_to_string(&token_bytes, &mut token_str, false);

            output.push_str(&token_str);

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
                .map_err(|e| AgentError::InferenceError(format!("添加 token 失败: {}", e)))?;

            ctx.decode(&mut batch)
                .map_err(|e| AgentError::InferenceError(format!("解码失败: {}", e)))?;

            n_cur += 1;
        }

        #[cfg(debug_assertions)]
        println!(
            "\n\n✅ [推理完成] 共生成 {} 个 token",
            n_cur - tokens.len() as i32
        );

        *self.state.write() = AgentState::Idle;
        Ok(output)
    }

    /// 解析工具调用 (支持多种格式)
    fn parse_tool_calls(&self, response: &str) -> Vec<ToolCall> {
        let mut tool_calls = Vec::new();

        // 格式 1: <tool_call>...</tool_call> (Hermes-style，支持多行 JSON)
        let re = regex::Regex::new(r"(?s)<tool_call>\s*(\{.*?\})\s*</tool_call>").ok();
        if let Some(re) = re {
            for cap in re.captures_iter(response) {
                if let Some(json_str) = cap.get(1) {
                    // 清理 JSON 字符串中的换行和多余空格
                    let cleaned = json_str.as_str().trim();
                    if let Ok(tool_call) = serde_json::from_str::<ToolCall>(cleaned) {
                        tool_calls.push(tool_call);
                    }
                }
            }
        }

        // 格式 2: Qwen 风格的 function call (备用)
        if tool_calls.is_empty() {
            let qwen_re =
                regex::Regex::new(r#"(?s)✿FUNCTION✿:\s*(\w+)\s*\n✿ARGS✿:\s*(\{.*?\})(?:\n|$)"#)
                    .ok();
            if let Some(re) = qwen_re {
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

    /// 提取思考内容
    #[allow(dead_code)]
    fn extract_thinking(&self, response: &str) -> Option<String> {
        let re = regex::Regex::new(r"<think>([\s\S]*?)</think>").ok()?;
        re.captures(response)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().trim().to_string())
    }

    /// 执行单次 ReAct 循环
    pub fn step(&self) -> Result<(String, bool), AgentError> {
        self.step_with_callbacks(None, None)
    }

    /// 执行单次 ReAct 循环（带回调）
    pub fn step_with_callback(
        &self,
        callback: Option<&dyn Fn(&str)>,
    ) -> Result<(String, bool), AgentError> {
        self.step_with_callbacks(callback, None)
    }

    /// 执行单次 ReAct 循环（带生成回调和工具结果回调）
    pub fn step_with_callbacks(
        &self,
        callback: Option<&dyn Fn(&str)>,
        tool_callback: Option<&dyn Fn(&str, &str, bool)>,
    ) -> Result<(String, bool), AgentError> {
        #[cfg(debug_assertions)]
        println!("\n🔄 [ReAct Step] 开始执行单次循环");

        let response = self.generate_with_callback(callback)?;

        let tool_calls = self.parse_tool_calls(&response);

        #[cfg(debug_assertions)]
        if !tool_calls.is_empty() {
            println!("\n🔧 [检测到工具调用] {:?}", tool_calls);
        }

        if !tool_calls.is_empty() {
            *self.state.write() = AgentState::Acting;

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

                let mut messages = self.messages.write();
                messages.push(Message {
                    role: Role::Tool,
                    content: format!("工具 {} 执行结果:\n{}", result.tool_name, result.result),
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

    /// 执行工具调用
    fn execute_tool(&self, tool_call: &ToolCall) -> Result<ToolResult, AgentError> {
        #[cfg(debug_assertions)]
        println!(
            "\n⚡ [执行工具] {} 参数: {}",
            tool_call.name, tool_call.arguments
        );

        let executor_opt = {
            let executors = self.tool_executors.read();
            executors
                .iter()
                .find(|(_, executor)| {
                    executor
                        .get_tools()
                        .iter()
                        .any(|t| t.name == tool_call.name)
                })
                .map(|(_, executor)| executor.clone())
        };

        if let Some(executor) = executor_opt {
            let result = executor.execute(tool_call);
            #[cfg(debug_assertions)]
            if let Ok(ref r) = result {
                println!("\n📤 [工具结果] {}: {}", r.tool_name, r.result);
            }
            return result;
        }

        #[cfg(debug_assertions)]
        println!("\n❌ [工具未找到] {}", tool_call.name);

        Ok(ToolResult {
            tool_name: tool_call.name.clone(),
            result: format!("工具 {} 未找到", tool_call.name),
            is_error: true,
        })
    }

    /// 运行完整的 ReAct 循环
    pub fn run(&self, user_input: &str, max_iterations: usize) -> Result<String, AgentError> {
        self.run_with_callbacks(user_input, max_iterations, None, None)
    }

    /// 运行完整的 ReAct 循环（带回调）
    pub fn run_with_callback(
        &self,
        user_input: &str,
        max_iterations: usize,
        callback: Option<&dyn Fn(&str)>,
    ) -> Result<String, AgentError> {
        self.run_with_callbacks(user_input, max_iterations, callback, None)
    }

    /// 运行完整的 ReAct 循环（带生成回调和工具结果回调）
    pub fn run_with_callbacks(
        &self,
        user_input: &str,
        max_iterations: usize,
        callback: Option<&dyn Fn(&str)>,
        tool_callback: Option<&dyn Fn(&str, &str, bool)>,
    ) -> Result<String, AgentError> {
        #[cfg(debug_assertions)]
        println!("\n\n🚀 ================== ReAct Agent 开始 ==================");
        #[cfg(debug_assertions)]
        println!("📊 [最大迭代次数] {}", max_iterations);

        if self.messages.read().is_empty()
            || !self.messages.read().iter().any(|m| m.role == Role::System)
        {
            self.set_system_prompt(&self.build_react_system_prompt());
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
        println!("\n🏁 ================== ReAct Agent 结束 ==================\n");

        Ok(final_response)
    }

    /// 清空对话历史
    pub fn clear_history(&self) {
        let mut messages = self.messages.write();
        messages.retain(|m| m.role == Role::System);
    }

    /// 获取当前状态
    pub fn get_state(&self) -> AgentState {
        self.state.read().clone()
    }

    /// 获取对话历史
    pub fn get_messages(&self) -> Vec<Message> {
        self.messages.read().clone()
    }
}

/// 内置的 Echo 工具执行器（用于测试）
pub struct EchoToolExecutor;

impl McpToolExecutor for EchoToolExecutor {
    fn execute(&self, tool_call: &ToolCall) -> Result<ToolResult, AgentError> {
        Ok(ToolResult {
            tool_name: tool_call.name.clone(),
            result: format!(
                "Echo: {}",
                serde_json::to_string(&tool_call.arguments).unwrap()
            ),
            is_error: false,
        })
    }

    fn get_tools(&self) -> Vec<McpTool> {
        vec![McpTool {
            name: "echo".to_string(),
            description: "回显输入的内容".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "要回显的消息"
                    }
                },
                "required": ["message"]
            }),
        }]
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

        let _agent = ReactAgent::new(AgentConfig {
            model_path: PathBuf::from("test.gguf"),
            ..Default::default()
        });

        // 注意：这个测试在没有实际模型时会失败
        // 实际使用时需要有效的模型文件
    }
}
