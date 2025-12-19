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

pub const RESUM: &str = "请极度压缩总结以上对话，只保留核心信息。

输出格式（每项不超过50字）：

[核心] 一句话概括对话主题和结果
[要点] 关键数据/事实，用分号分隔
[回复] 给用户的最终答案

要求：删除冗余描述，只保留必要信息。";
