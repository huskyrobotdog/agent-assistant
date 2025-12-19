import OpenAI from 'openai'

// LLM 配置
const config = {
    baseURL: 'https://api.deepseek.com/v1',
    apiKey: 'sk-f891ba405de84e7687e76eb9f54debe7',
    model: 'deepseek-reasoner'
}

// 创建 OpenAI 客户端
const client = new OpenAI({
    baseURL: config.baseURL,
    apiKey: config.apiKey,
    dangerouslyAllowBrowser: true
})

/**
 * 流式对话
 * @param {Array} messages - 消息历史 [{role: 'user', content: '...'}]
 * @param {Object} options - 配置选项
 * @param {Function} options.onToken - 收到 token 时的回调
 * @param {Function} options.onReasoning - 收到推理内容时的回调
 * @param {Function} options.onDone - 完成时的回调
 * @param {Function} options.onError - 错误时的回调
 * @param {AbortSignal} options.signal - 用于取消请求
 */
export async function chatStream(messages, options = {}) {
    const { onToken, onReasoning, onDone, onError, signal } = options

    try {
        const stream = await client.chat.completions.create({
            model: config.model,
            messages,
            stream: true
        }, { signal })

        let fullContent = ''
        let fullReasoning = ''

        for await (const chunk of stream) {
            const delta = chunk.choices[0]?.delta

            // 处理推理内容（Qwen3 的思考过程）
            if (delta?.reasoning_content) {
                fullReasoning += delta.reasoning_content
                onReasoning?.(delta.reasoning_content, fullReasoning)
            }

            // 处理正常内容
            if (delta?.content) {
                fullContent += delta.content
                onToken?.(delta.content, fullContent)
            }
        }

        onDone?.({ content: fullContent, reasoning: fullReasoning })
        return { content: fullContent, reasoning: fullReasoning }
    } catch (error) {
        if (error.name === 'AbortError') {
            onDone?.({ content: '', reasoning: '', aborted: true })
            return { content: '', reasoning: '', aborted: true }
        }
        onError?.(error)
        throw error
    }
}

/**
 * 非流式对话
 * @param {Array} messages - 消息历史
 */
export async function chat(messages) {
    const response = await client.chat.completions.create({
        model: config.model,
        messages,
        stream: false
    })
    return response.choices[0]?.message?.content || ''
}

/**
 * 更新配置
 * @param {Object} newConfig - 新配置
 */
export function updateConfig(newConfig) {
    Object.assign(config, newConfig)
}

/**
 * 获取当前配置
 */
export function getConfig() {
    return { ...config }
}
