<script setup>
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import ConversationList from './ConversationList.vue'
import ChatMessage from './ChatMessage.vue'
import ChatInput from './ChatInput.vue'
import { chatStream } from '@/utils/llm'

// 对话列表
const conversations = ref([])

// 当前选中的对话
const currentConversationId = ref(null)

// 当前对话的消息
const messages = ref([])

// 上下文信息
const contextInfo = ref({ context_length: 0, current_tokens: 0, current_chars: 0 })

// 格式化数字（千分位）
function formatNumber(num) {
  return num.toLocaleString()
}

// 计算上下文使用百分比
function getContextPercent() {
  if (contextInfo.value.context_length === 0) return 0
  return (contextInfo.value.current_tokens / contextInfo.value.context_length) * 100
}

const isLoading = ref(false)
const messagesContainer = ref(null)
const isUserAtBottom = ref(true)

// 选择对话
function selectConversation(id) {
  currentConversationId.value = id
}

// 新建对话
function createConversation() {
  const ids = conversations.value.map((c) => c.id)
  const newId = ids.length > 0 ? Math.max(...ids) + 1 : 1
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

// 清空对话
function clearMessages() {
  messages.value = []
}

// 当前流式消息 ID
const streamingMessageId = ref(null)

// 构建消息历史（用于 API 调用）
function buildMessages(userContent) {
  // 获取历史消息（排除当前正在输入的）
  const history = messages.value
    .filter((m) => m.role === 'user' || m.role === 'assistant')
    .map((m) => ({
      role: m.role,
      content: m.role === 'assistant' ? cleanThinkTags(m.content) : m.content,
    }))

  // 添加新的用户消息
  history.push({ role: 'user', content: userContent })

  return history
}

// 清理 think 标签
function cleanThinkTags(content) {
  return content
    .replace(/<think>[\s\S]*?<\/think>/g, '')
    .replace(/<think>[\s\S]*$/, '')
    .trim()
}

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
    reasoning: '',
  })

  await nextTick()
  scrollToBottom()

  // 创建新的 AbortController
  abortController = new AbortController()

  try {
    const apiMessages = buildMessages(content)

    await chatStream(apiMessages, {
      signal: abortController.signal,
      onToken: (token) => {
        const msg = messages.value.find((m) => m.id === aiMessageId)
        if (msg) {
          msg.content += token
          nextTick(() => scrollToBottom())
        }
      },
      onReasoning: (token) => {
        const msg = messages.value.find((m) => m.id === aiMessageId)
        if (msg) {
          // 将推理内容包装在 <think> 标签中
          if (!msg.content.includes('<think>')) {
            msg.content = '<think>' + token
          } else if (msg.content.includes('</think>')) {
            // 已有完整的 think 块，追加新的
            msg.content = msg.content.replace(/<\/think>$/, token)
          } else {
            // 正在 think 块中
            msg.content += token
          }
          nextTick(() => scrollToBottom())
        }
      },
      onDone: () => {
        const msg = messages.value.find((m) => m.id === aiMessageId)
        if (msg && msg.content.includes('<think>') && !msg.content.includes('</think>')) {
          msg.content += '</think>'
        }
      },
      onError: (error) => {
        const msg = messages.value.find((m) => m.id === aiMessageId)
        if (msg) {
          msg.content = `❌ 错误: ${error.message}`
        }
      },
    })
  } catch (error) {
    const msg = messages.value.find((m) => m.id === aiMessageId)
    if (msg && !msg.content) {
      msg.content = `❌ 错误: ${error.message}`
    }
  } finally {
    isLoading.value = false
    streamingMessageId.value = null
    abortController = null
    await nextTick()
    scrollToBottom()
  }
}

// 滚动到底部
function scrollToBottom(force = false) {
  if (messagesContainer.value && (force || isUserAtBottom.value)) {
    messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
  }
}

// 检测用户是否在底部
function checkIsAtBottom() {
  if (messagesContainer.value) {
    const { scrollTop, scrollHeight, clientHeight } = messagesContainer.value
    // 允许 30px 的误差
    isUserAtBottom.value = scrollHeight - scrollTop - clientHeight < 30
  }
}

// 处理滚动事件
function handleScroll() {
  checkIsAtBottom()
}

// 用于取消请求的控制器
let abortController = null

onMounted(() => {
  scrollToBottom(true)
})

onUnmounted(() => {
  // 取消正在进行的请求
  if (abortController) {
    abortController.abort()
  }
})
</script>

<template>
  <div class="page-container">
    <!-- 左侧对话列表（暂时屏蔽）
    <ConversationList
      :conversations="conversations"
      :current-id="currentConversationId"
      class="sidebar"
      @select="selectConversation"
      @create="createConversation"
      @delete="deleteConversation"
      @rename="renameConversation" />
    -->

    <!-- 对话窗口 -->
    <div class="chat-window">
      <!-- 顶部工具栏 -->
      <div class="toolbar">
        <!-- 上下文信息 -->
        <div
          v-if="contextInfo.context_length > 0"
          class="context-info">
          <span class="context-label">上下文:</span>
          <span class="context-value">
            {{ formatNumber(contextInfo.current_chars) }} 字符 / {{ formatNumber(contextInfo.current_tokens) }} tokens
          </span>
          <span class="context-percent">({{ getContextPercent().toFixed(1) }}%)</span>
          <div class="context-progress">
            <div
              class="context-progress-fill"
              :style="{ width: `${getContextPercent().toFixed(2)}%` }"
              :class="{ warning: getContextPercent() > 80 }" />
          </div>
        </div>
        <div class="toolbar-spacer" />
        <!-- 清空对话按钮 -->
        <button
          class="clear-btn"
          :disabled="messages.length === 0 || isLoading"
          @click="clearMessages">
          <i class="pi pi-trash" />
          清空对话
        </button>
      </div>

      <!-- 消息列表 -->
      <div
        ref="messagesContainer"
        class="messages-container"
        @scroll="handleScroll">
        <ChatMessage
          v-for="msg in messages"
          :key="msg.id"
          :message="msg" />

        <!-- 加载状态 -->
        <Transition name="fade">
          <div
            v-if="isLoading"
            class="loading-container">
            <img
              src="/logo.svg"
              alt="loading"
              class="loading-logo" />
          </div>
        </Transition>
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

.loading-container {
  position: sticky;
  bottom: 0;
  display: flex;
  justify-content: center;
  padding: 1rem;
}

.loading-logo {
  width: 32px;
  height: 32px;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.3s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--p-surface-200);
  font-size: 0.75rem;
  color: var(--p-text-muted-color);
}

.app-dark .toolbar {
  border-color: var(--p-surface-700);
}

.toolbar-spacer {
  flex: 1;
}

.context-info {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.clear-btn {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0.375rem 0.75rem;
  font-size: 0.75rem;
  color: var(--p-text-muted-color);
  background: transparent;
  border: 1px solid var(--p-surface-300);
  border-radius: 0.375rem;
  cursor: pointer;
  transition: all 0.2s;
}

.clear-btn:hover:not(:disabled) {
  color: var(--p-red-500);
  border-color: var(--p-red-500);
  background: var(--p-red-50);
}

.clear-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.app-dark .clear-btn {
  border-color: var(--p-surface-600);
}

.app-dark .clear-btn:hover:not(:disabled) {
  background: rgba(239, 68, 68, 0.1);
}

.context-label {
  font-weight: 500;
}

.context-value {
  font-family: monospace;
}

.context-percent {
  font-family: monospace;
  color: var(--p-text-muted-color);
}

.context-progress {
  flex: 1;
  max-width: 120px;
  height: 4px;
  background-color: var(--p-surface-200);
  border-radius: 2px;
  overflow: hidden;
}

.app-dark .context-progress {
  background-color: var(--p-surface-700);
}

.context-progress-fill {
  height: 100%;
  background-color: var(--p-primary-color);
  transition: width 0.3s ease;
}

.context-progress-fill.warning {
  background-color: var(--p-orange-500);
}
</style>
