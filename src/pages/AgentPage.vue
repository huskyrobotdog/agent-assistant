<script setup>
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import ConversationList from '@/components/agent/ConversationList.vue'
import ChatMessage from '@/components/agent/ChatMessage.vue'
import ChatInput from '@/components/agent/ChatInput.vue'

// 对话列表
const conversations = ref([
  { id: 1, title: '量化策略分析', time: '今天 14:30', preview: '关于双均线策略的优化建议...' },
  { id: 2, title: '风险控制讨论', time: '今天 10:15', preview: '如何设置合理的止损点位...' },
  { id: 3, title: '回测结果分析', time: '昨天', preview: '最近的回测数据显示...' },
])

// 当前选中的对话
const currentConversationId = ref(1)

// 当前对话的消息
const messages = ref([
  {
    id: 1,
    role: 'user',
    content: '请帮我分析一下双均线交叉策略的优缺点',
  },
  {
    id: 2,
    role: 'assistant',
    content: `## 双均线交叉策略分析

### 优点

1. **简单易懂**：策略逻辑清晰，容易实现和理解
2. **趋势跟踪**：能够有效捕捉中长期趋势
3. **自动化程度高**：信号明确，适合程序化交易

### 缺点

1. **滞后性**：移动平均线本身具有滞后性
2. **震荡市不适用**：在横盘整理期间会产生大量假信号
3. **参数敏感**：均线周期的选择对策略效果影响很大

### 代码示例

\`\`\`python
def dual_ma_strategy(data, short_period=5, long_period=20):
    data['ma_short'] = data['close'].rolling(short_period).mean()
    data['ma_long'] = data['close'].rolling(long_period).mean()
    
    # 金叉买入，死叉卖出
    data['signal'] = 0
    data.loc[data['ma_short'] > data['ma_long'], 'signal'] = 1
    data.loc[data['ma_short'] < data['ma_long'], 'signal'] = -1
    
    return data
\`\`\`

建议在实际使用中结合其他指标进行过滤，以减少假信号。`,
    thinking: `用户询问双均线策略，我需要从以下几个方面进行分析：
1. 策略的基本原理
2. 优点分析：简单性、趋势跟踪能力
3. 缺点分析：滞后性、震荡市表现
4. 给出代码示例帮助理解
5. 提供改进建议`,
  },
  {
    id: 3,
    role: 'user',
    content: '如何优化这个策略来减少假信号？',
  },
  {
    id: 4,
    role: 'assistant',
    content: `## 减少假信号的优化方法

### 1. 添加过滤条件

可以结合 **RSI** 或 **MACD** 等指标进行信号确认：

\`\`\`python
def optimized_strategy(data):
    # 计算均线
    data['ma_short'] = data['close'].rolling(5).mean()
    data['ma_long'] = data['close'].rolling(20).mean()
    
    # 计算 RSI
    data['rsi'] = calculate_rsi(data['close'], 14)
    
    # 只有当 RSI 也支持时才开仓
    data['signal'] = 0
    buy_condition = (data['ma_short'] > data['ma_long']) & (data['rsi'] < 70)
    sell_condition = (data['ma_short'] < data['ma_long']) & (data['rsi'] > 30)
    
    data.loc[buy_condition, 'signal'] = 1
    data.loc[sell_condition, 'signal'] = -1
    
    return data
\`\`\`

### 2. 增加信号确认时间

等待交叉信号持续 **2-3 根K线** 后再执行交易。

### 3. 使用自适应均线

根据市场波动率动态调整均线周期，如 **Kaufman 自适应均线 (KAMA)**。

| 优化方法 | 效果 | 复杂度 |
|---------|------|--------|
| RSI过滤 | ⭐⭐⭐ | 低 |
| 信号确认 | ⭐⭐ | 低 |
| 自适应均线 | ⭐⭐⭐⭐ | 中 |`,
    reactChain: [
      { type: 'thought', content: '用户想要优化策略减少假信号，我需要思考几种常见的优化方法' },
      { type: 'action', content: '分析技术指标过滤、时间过滤、自适应方法等优化策略' },
      {
        type: 'observation',
        content: 'RSI过滤效果好且复杂度低，信号确认简单但效果一般，自适应均线效果最好但复杂度中等',
      },
      { type: 'thought', content: '需要给出具体代码和对比表格便于用户理解和选择' },
    ],
  },
])

const isLoading = ref(false)
const messagesContainer = ref(null)

// 选择对话
function selectConversation(id) {
  currentConversationId.value = id
}

// 新建对话
function createConversation() {
  const newId = Math.max(...conversations.value.map((c) => c.id)) + 1
  conversations.value.unshift({
    id: newId,
    title: '新对话',
    time: '刚刚',
    preview: '开始新的对话...',
  })
  currentConversationId.value = newId
  messages.value = []
}

// 删除对话
function deleteConversation(id) {
  conversations.value = conversations.value.filter((c) => c.id !== id)
  if (currentConversationId.value === id && conversations.value.length > 0) {
    currentConversationId.value = conversations.value[0].id
  }
}

// 重命名对话
function renameConversation({ id, title }) {
  const conv = conversations.value.find((c) => c.id === id)
  if (conv) {
    conv.title = title
  }
}

// 当前流式消息 ID
const streamingMessageId = ref(null)

// 发送消息
async function handleSend(content) {
  const userMessage = {
    id: Date.now(),
    role: 'user',
    content,
  }
  messages.value.push(userMessage)
  isLoading.value = true

  // 创建空的 AI 消息用于流式填充
  const aiMessageId = Date.now() + 1
  streamingMessageId.value = aiMessageId
  messages.value.push({
    id: aiMessageId,
    role: 'assistant',
    content: '',
  })

  await nextTick()
  scrollToBottom()

  try {
    await invoke('chat', { message: content })
  } catch (error) {
    // 更新消息为错误信息
    const msg = messages.value.find((m) => m.id === aiMessageId)
    if (msg) {
      msg.content = `❌ 错误: ${error}`
    }
  } finally {
    isLoading.value = false
    streamingMessageId.value = null
    await nextTick()
    scrollToBottom()
  }
}

// 滚动到底部
function scrollToBottom() {
  if (messagesContainer.value) {
    messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
  }
}

// 事件监听器
let unlistenToken = null
let unlistenDone = null

onMounted(async () => {
  scrollToBottom()

  // 监听流式 token
  unlistenToken = await listen('chat-token', (event) => {
    if (streamingMessageId.value) {
      const msg = messages.value.find((m) => m.id === streamingMessageId.value)
      if (msg) {
        msg.content += event.payload
        nextTick(() => scrollToBottom())
      }
    }
  })

  // 监听完成事件
  unlistenDone = await listen('chat-done', () => {
    isLoading.value = false
    streamingMessageId.value = null
  })
})

onUnmounted(() => {
  if (unlistenToken) unlistenToken()
  if (unlistenDone) unlistenDone()
})
</script>

<template>
  <div class="page-container">
    <!-- 左侧对话列表 -->
    <ConversationList
      :conversations="conversations"
      :current-id="currentConversationId"
      class="sidebar"
      @select="selectConversation"
      @create="createConversation"
      @delete="deleteConversation"
      @rename="renameConversation" />

    <!-- 右侧对话窗口 -->
    <div class="chat-window">
      <!-- 消息列表 -->
      <div
        ref="messagesContainer"
        class="messages-container">
        <ChatMessage
          v-for="msg in messages"
          :key="msg.id"
          :message="msg" />

        <!-- 加载状态 -->
        <div
          v-if="isLoading"
          class="flex justify-start">
          <div class="loading-bubble">
            <ProgressSpinner
              style="width: 20px; height: 20px"
              strokeWidth="4" />
          </div>
        </div>
      </div>

      <!-- 输入区域 -->
      <ChatInput
        :disabled="isLoading"
        :loading="isLoading"
        @send="handleSend" />
    </div>
  </div>
</template>

<style scoped>
.page-container {
  height: 100%;
  display: flex;
  gap: 1rem;
  overflow: hidden;
}

.sidebar {
  width: 18rem;
  flex-shrink: 0;
}

.chat-window {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background-color: var(--p-surface-0);
  border-radius: 0.5rem;
  border: 1px solid var(--p-surface-200);
}

.app-dark .chat-window {
  background-color: var(--p-surface-900);
  border-color: var(--p-surface-700);
}

.messages-container {
  flex: 1;
  overflow: auto;
  padding: 1rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.loading-bubble {
  background-color: var(--p-surface-100);
  border-radius: 0.5rem;
  padding: 0.75rem 1rem;
}

.app-dark .loading-bubble {
  background-color: var(--p-surface-800);
}
</style>
