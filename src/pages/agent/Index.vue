<script setup>
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import ConversationList from './ConversationList.vue'
import ChatMessage from './ChatMessage.vue'
import ChatInput from './ChatInput.vue'

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

// 事件监听器
let unlistenToken = null
let unlistenDone = null
let unlistenToolResult = null
let unlistenContextUpdate = null

onMounted(async () => {
  scrollToBottom(true)

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

  // 监听工具执行结果（使用 CoT Result 格式）
  unlistenToolResult = await listen('tool-result', (event) => {
    if (streamingMessageId.value) {
      const msg = messages.value.find((m) => m.id === streamingMessageId.value)
      if (msg) {
        const { result } = event.payload
        msg.content += `\nResult: ${result}\n`
        nextTick(() => scrollToBottom())
      }
    }
  })

  // 监听完成事件
  unlistenDone = await listen('chat-done', () => {
    isLoading.value = false
    streamingMessageId.value = null
  })

  // 监听上下文更新事件（实时）
  unlistenContextUpdate = await listen('context-update', (event) => {
    contextInfo.value = event.payload
  })
})

// 更新上下文信息（暂未实现后端命令）
async function updateContextInfo() {
  // TODO: 后端需要实现 get_context_info 命令
}

onUnmounted(() => {
  if (unlistenToken) unlistenToken()
  if (unlistenDone) unlistenDone()
  if (unlistenToolResult) unlistenToolResult()
  if (unlistenContextUpdate) unlistenContextUpdate()
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
      <!-- 上下文信息栏 -->
      <div
        v-if="contextInfo.context_length > 0"
        class="context-bar">
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

.context-bar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--p-surface-200);
  font-size: 0.75rem;
  color: var(--p-text-muted-color);
}

.app-dark .context-bar {
  border-color: var(--p-surface-700);
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
