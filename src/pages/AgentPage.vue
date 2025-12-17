<script setup>
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import ConversationList from '@/components/agent/ConversationList.vue'
import ChatMessage from '@/components/agent/ChatMessage.vue'
import ChatInput from '@/components/agent/ChatInput.vue'

// 对话列表
const conversations = ref([])

// 当前选中的对话
const currentConversationId = ref(null)

// 当前对话的消息
const messages = ref([])

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

  // 监听工具执行结果
  unlistenToolResult = await listen('tool-result', (event) => {
    if (streamingMessageId.value) {
      const msg = messages.value.find((m) => m.id === streamingMessageId.value)
      if (msg) {
        // 使用 Hermes 标准格式 <tool_response>
        const { name, result, isError } = event.payload
        const content = JSON.stringify({ name, content: result, is_error: isError })
        msg.content += `\n<tool_response>${content}</tool_response>\n`
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
  if (unlistenToolResult) unlistenToolResult()
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
        class="messages-container"
        @scroll="handleScroll">
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
