pub const REACT: &str = "请尽可能回答以下问题。你可以使用以下工具：

{{TOOLS}}

请严格使用以下格式：

Question: 你需要回答的问题
Thought: 你需要思考下一步该做什么
Action: 要执行的动作，必须是 [{{TOOL_NAMES}}] 中的一个
Action Input: 动作的输入参数（JSON 格式）
Observation: 动作执行的结果（由系统提供，你绝对不能自己编造）
...（Thought/Action/Action Input/Observation 可以重复多次）
Thought: 我现在知道最终答案了
Final Answer: 原始问题的最终答案

重要规则：
- 你必须先输出 Action 和 Action Input 调用工具
- Observation 只能由系统提供，你绝对不能自己生成或捏造
- 输出 Action Input 后必须立即停止，等待系统返回 Observation
- Final Answer 必须直接引用 Observation 中的真实数据，禁止编造任何未在 Observation 中出现的内容
- 如果 Observation 显示操作成功但没有返回所需数据，必须继续调用工具获取数据

示例：
Question: 用户的问题
Thought: 分析问题，决定使用哪个工具
Action: 选择的工具名称
Action Input: {参数}

（输出 Action Input 后立即停止，等待系统返回 Observation）

开始！

Question: {{QUERY}}
Thought:";

pub const TOOL_CALL_RESUM: &str = "之前的思考过程：
{{THOUGHTS}}

工具调用信息：
工具名称：{{TOOL_NAME}}
入参：{{TOOL_ARGS}}

返回结果：
{{TOOL_RESULT}}

请总结以上工具调用结果，提取最有用的信息。

输出格式：

[调用] 工具名称 + 关键入参（一行概括）
[结果] 直接输出最有用的结果数据。如果是JSON/列表等结构化数据，提取核心字段或条目；如果是文本，提取关键信息
[结论] 结合之前的思考过程，给出结论或下一步建议

要求：
- 删除元数据、格式噪音、重复内容
- 保留对回答用户问题有帮助的核心数据
- 结果部分应该是可以直接引用的有效信息";
