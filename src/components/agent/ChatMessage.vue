<script setup>
import { ref, computed } from 'vue'
import { marked } from 'marked'
import hljs from 'highlight.js'

// 配置 marked
marked.setOptions({
  highlight: (code, lang) => {
    if (lang && hljs.getLanguage(lang)) {
      return hljs.highlight(code, { language: lang }).value
    }
    return hljs.highlightAuto(code).value
  },
  breaks: true,
  gfm: true,
})

const props = defineProps({
  message: {
    type: Object,
    required: true,
  },
})

const showThinking = ref(false)

const isUser = computed(() => props.message.role === 'user')

// 解析内容，提取思考/操作/观察等步骤
const parsedContent = computed(() => {
  const content = props.message.content || ''

  // 解析各种标签
  const thinkRegex = /<think>([\s\S]*?)<\/think>/g
  const toolCallRegex = /<tool_call>([\s\S]*?)<\/tool_call>/g

  // 提取所有步骤
  const steps = []

  // 提取思考内容
  let match
  while ((match = thinkRegex.exec(content)) !== null) {
    if (match[1].trim()) {
      steps.push({ type: 'thought', content: match[1].trim(), index: match.index })
    }
  }

  // 提取工具调用
  while ((match = toolCallRegex.exec(content)) !== null) {
    if (match[1].trim()) {
      steps.push({ type: 'action', content: match[1].trim(), index: match.index })
    }
  }

  // 按出现顺序排序
  steps.sort((a, b) => a.index - b.index)

  // 移除所有标签后的响应内容
  let responseContent = content.replace(thinkRegex, '').replace(toolCallRegex, '')

  // 移除未闭合的标签及其内容（流式中）
  const thinkOpen = (content.match(/<think>/g) || []).length
  const thinkClose = (content.match(/<\/think>/g) || []).length
  const toolOpen = (content.match(/<tool_call>/g) || []).length
  const toolClose = (content.match(/<\/tool_call>/g) || []).length

  const isStreaming = thinkOpen > thinkClose || toolOpen > toolClose

  if (thinkOpen > thinkClose) {
    const idx = responseContent.lastIndexOf('<think>')
    if (idx !== -1) responseContent = responseContent.substring(0, idx)
  }
  if (toolOpen > toolClose) {
    const idx = responseContent.lastIndexOf('<tool_call>')
    if (idx !== -1) responseContent = responseContent.substring(0, idx)
  }

  return {
    steps,
    response: responseContent.trim(),
    isStreaming,
  }
})

// 获取流式步骤（未关闭的标签）
const streamingStep = computed(() => {
  const content = props.message.content || ''

  // 检查未闭合的 think
  const thinkOpen = (content.match(/<think>/g) || []).length
  const thinkClose = (content.match(/<\/think>/g) || []).length
  if (thinkOpen > thinkClose) {
    const idx = content.lastIndexOf('<think>')
    return { type: 'thought', content: content.substring(idx + 7).trim() }
  }

  // 检查未闭合的 tool_call
  const toolOpen = (content.match(/<tool_call>/g) || []).length
  const toolClose = (content.match(/<\/tool_call>/g) || []).length
  if (toolOpen > toolClose) {
    const idx = content.lastIndexOf('<tool_call>')
    return { type: 'action', content: content.substring(idx + 11).trim() }
  }

  return null
})

const renderedContent = computed(() => {
  return marked(parsedContent.value.response)
})

// 判断是否有步骤要显示
const hasSteps = computed(() => {
  return parsedContent.value.steps.length > 0 || streamingStep.value
})

// 解析 ReAct 思维链步骤
const reactSteps = computed(() => {
  if (!props.message.reactChain) return []
  return props.message.reactChain
})

// 判断是否有 ReAct 思维链
const hasReactChain = computed(() => {
  return props.message.reactChain && props.message.reactChain.length > 0
})

// 判断是否有思考过程（兼容旧逻辑）
const hasThinking = computed(() => {
  return hasSteps.value && !hasReactChain.value
})

// 获取所有步骤（包括流式的）
const allSteps = computed(() => {
  const steps = [...parsedContent.value.steps]
  if (streamingStep.value) {
    steps.push({ ...streamingStep.value, isStreaming: true })
  }
  return steps
})

function toggleThinking() {
  showThinking.value = !showThinking.value
}

// 获取步骤类型的图标和颜色
function getStepStyle(type) {
  switch (type) {
    case 'thought':
      return { icon: 'pi-lightbulb', color: 'thought' }
    case 'action':
      return { icon: 'pi-bolt', color: 'action' }
    case 'observation':
      return { icon: 'pi-eye', color: 'observation' }
    default:
      return { icon: 'pi-circle', color: '' }
  }
}

function getStepLabel(type) {
  switch (type) {
    case 'thought':
      return '思考'
    case 'action':
      return '行动'
    case 'observation':
      return '观察'
    default:
      return type
  }
}
</script>

<template>
  <div
    class="flex"
    :class="isUser ? 'justify-end' : 'justify-start'">
    <div
      class="message-bubble max-w-[80%] rounded-lg"
      :class="isUser ? 'user-message' : 'assistant-message'">
      <!-- 用户消息 -->
      <template v-if="isUser">
        <p class="whitespace-pre-wrap">{{ message.content }}</p>
      </template>

      <!-- AI 消息 -->
      <template v-else>
        <!-- ReAct 思维链 -->
        <div
          v-if="hasReactChain"
          class="react-chain mb-3">
          <button
            class="flex items-center gap-1 text-xs text-surface-500 hover:text-surface-700 dark:hover:text-surface-300 transition-colors"
            @click="toggleThinking">
            <i
              class="pi"
              :class="showThinking ? 'pi-chevron-down' : 'pi-chevron-right'" />
            <i class="pi pi-sitemap" />
            <span>推理过程 ({{ reactSteps.length }} 步)</span>
          </button>
          <div
            v-show="showThinking"
            class="react-steps">
            <div
              v-for="(step, index) in reactSteps"
              :key="index"
              class="react-step"
              :class="getStepStyle(step.type).color">
              <div class="step-header">
                <i
                  class="pi"
                  :class="getStepStyle(step.type).icon" />
                <span class="step-label">{{ getStepLabel(step.type) }}</span>
              </div>
              <div class="step-content selectable">{{ step.content }}</div>
            </div>
          </div>
        </div>

        <!-- 思考/操作过程 - 时间线UI -->
        <div
          v-else-if="hasThinking"
          class="thinking-timeline mb-3">
          <div
            v-for="(step, index) in allSteps"
            :key="index"
            class="timeline-item"
            :class="[getStepStyle(step.type).color, { 'is-streaming': step.isStreaming }]">
            <div class="timeline-marker">
              <i
                class="pi"
                :class="getStepStyle(step.type).icon" />
            </div>
            <div class="timeline-content">
              <div class="timeline-label">{{ getStepLabel(step.type) }}</div>
              <div class="timeline-text selectable">{{ step.content }}</div>
            </div>
          </div>
        </div>

        <!-- Markdown 内容（只在有响应内容时显示） -->
        <div
          v-if="parsedContent.response"
          class="markdown-content selectable"
          v-html="renderedContent" />
      </template>
    </div>
  </div>
</template>

<style scoped>
.message-bubble.user-message {
  background-color: var(--p-primary-color);
  color: var(--p-primary-contrast-color);
  padding: 0.5rem 1rem;
}

.message-bubble.assistant-message {
  background-color: transparent;
  padding: 0;
}

/* 时间线样式 */
.thinking-timeline {
  position: relative;
  padding-left: 1.5rem;
}

.thinking-timeline::before {
  content: '';
  position: absolute;
  left: 0.4375rem;
  top: 0.5rem;
  bottom: 0.5rem;
  width: 2px;
  background: linear-gradient(180deg, #f59e0b 0%, #3b82f6 100%);
  opacity: 0.4;
}

.timeline-item {
  position: relative;
  padding-bottom: 0.75rem;
}

.timeline-item:last-child {
  padding-bottom: 0;
}

.timeline-marker {
  position: absolute;
  left: -1.5rem;
  top: 0;
  width: 1rem;
  height: 1rem;
  display: flex;
  align-items: center;
  justify-content: center;
  background-color: var(--p-surface-0);
  border-radius: 50%;
  z-index: 1;
}

.timeline-marker i {
  font-size: 0.75rem;
  color: #f59e0b;
}

/* 思考类型 - 橙色 */
.timeline-item.thought .timeline-marker i {
  color: #f59e0b;
}

.timeline-item.thought .timeline-content {
  background-color: color-mix(in srgb, #f59e0b 8%, transparent);
  border-color: color-mix(in srgb, #f59e0b 20%, transparent);
}

.timeline-item.thought .timeline-label {
  color: #d97706;
}

/* 操作类型 - 蓝色 */
.timeline-item.action .timeline-marker i {
  color: #3b82f6;
}

.timeline-item.action .timeline-content {
  background-color: color-mix(in srgb, #3b82f6 8%, transparent);
  border-color: color-mix(in srgb, #3b82f6 20%, transparent);
}

.timeline-item.action .timeline-label {
  color: #2563eb;
}

/* 观察类型 - 绿色 */
.timeline-item.observation .timeline-marker i {
  color: #10b981;
}

.timeline-item.observation .timeline-content {
  background-color: color-mix(in srgb, #10b981 8%, transparent);
  border-color: color-mix(in srgb, #10b981 20%, transparent);
}

.timeline-item.observation .timeline-label {
  color: #059669;
}

.timeline-item.is-streaming .timeline-marker i {
  animation: pulse 1.5s ease-in-out infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.4;
  }
}

.timeline-content {
  background-color: color-mix(in srgb, #f59e0b 8%, transparent);
  border-radius: 0.375rem;
  padding: 0.5rem 0.75rem;
  border: 1px solid color-mix(in srgb, #f59e0b 20%, transparent);
}

.timeline-label {
  font-size: 0.75rem;
  font-weight: 500;
  color: #d97706;
  margin-bottom: 0.25rem;
}

.timeline-text {
  font-size: 0.8125rem;
  line-height: 1.5;
  color: var(--p-surface-600);
  white-space: pre-wrap;
}

.app-dark .timeline-marker {
  background-color: var(--p-surface-900);
}

.app-dark .timeline-text {
  color: var(--p-surface-400);
}

.app-dark .timeline-item.thought .timeline-content {
  background-color: color-mix(in srgb, #f59e0b 12%, transparent);
  border-color: color-mix(in srgb, #f59e0b 25%, transparent);
}

.app-dark .timeline-item.action .timeline-content {
  background-color: color-mix(in srgb, #3b82f6 12%, transparent);
  border-color: color-mix(in srgb, #3b82f6 25%, transparent);
}

.app-dark .timeline-item.observation .timeline-content {
  background-color: color-mix(in srgb, #10b981 12%, transparent);
  border-color: color-mix(in srgb, #10b981 25%, transparent);
}

/* ReAct 思维链样式 */
.react-steps {
  margin-top: 0.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.react-step {
  padding: 0.5rem 0.75rem;
  border-radius: 0.25rem;
  border-left: 3px solid;
  font-size: 0.875rem;
}

.react-step.thought {
  background-color: color-mix(in srgb, #f59e0b 10%, transparent);
  border-color: #f59e0b;
}

.react-step.action {
  background-color: color-mix(in srgb, #3b82f6 10%, transparent);
  border-color: #3b82f6;
}

.react-step.observation {
  background-color: color-mix(in srgb, #10b981 10%, transparent);
  border-color: #10b981;
}

.step-header {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  font-weight: 500;
  margin-bottom: 0.25rem;
}

.react-step.thought .step-header {
  color: #d97706;
}

.react-step.action .step-header {
  color: #2563eb;
}

.react-step.observation .step-header {
  color: #059669;
}

.step-content {
  color: var(--p-surface-600);
  white-space: pre-wrap;
}

.app-dark .step-content {
  color: var(--p-surface-400);
}

/* Markdown 样式 */
.markdown-content :deep(h1),
.markdown-content :deep(h2),
.markdown-content :deep(h3) {
  margin-top: 1rem;
  margin-bottom: 0.5rem;
  font-weight: 600;
}

.markdown-content :deep(h1) {
  font-size: 1.25rem;
}
.markdown-content :deep(h2) {
  font-size: 1.125rem;
}
.markdown-content :deep(h3) {
  font-size: 1rem;
}

.markdown-content :deep(p) {
  margin-bottom: 0.5rem;
}

.markdown-content :deep(ul),
.markdown-content :deep(ol) {
  margin-left: 1.5rem;
  margin-bottom: 0.5rem;
}

.markdown-content :deep(li) {
  margin-bottom: 0.25rem;
}

.markdown-content :deep(pre) {
  background-color: var(--p-surface-800);
  border-radius: 0.5rem;
  padding: 1rem;
  overflow-x: auto;
  margin: 0.5rem 0;
}

.markdown-content :deep(code) {
  font-family: 'Fira Code', 'Monaco', 'Consolas', monospace;
  font-size: 0.875rem;
}

.markdown-content :deep(:not(pre) > code) {
  background-color: var(--p-surface-200);
  padding: 0.125rem 0.375rem;
  border-radius: 0.25rem;
}

.app-dark .markdown-content :deep(:not(pre) > code) {
  background-color: var(--p-surface-700);
}

.markdown-content :deep(table) {
  width: 100%;
  border-collapse: collapse;
  margin: 0.5rem 0;
}

.markdown-content :deep(th),
.markdown-content :deep(td) {
  border: 1px solid var(--p-surface-300);
  padding: 0.5rem;
  text-align: left;
}

.app-dark .markdown-content :deep(th),
.app-dark .markdown-content :deep(td) {
  border-color: var(--p-surface-600);
}

.markdown-content :deep(th) {
  background-color: var(--p-surface-100);
}

.app-dark .markdown-content :deep(th) {
  background-color: var(--p-surface-800);
}

/* highlight.js 代码主题 */
.markdown-content :deep(.hljs-keyword),
.markdown-content :deep(.hljs-selector-tag),
.markdown-content :deep(.hljs-built_in),
.markdown-content :deep(.hljs-name) {
  color: #c792ea;
}

.markdown-content :deep(.hljs-string),
.markdown-content :deep(.hljs-attr) {
  color: #c3e88d;
}

.markdown-content :deep(.hljs-number),
.markdown-content :deep(.hljs-literal) {
  color: #f78c6c;
}

.markdown-content :deep(.hljs-comment) {
  color: #546e7a;
  font-style: italic;
}

.markdown-content :deep(.hljs-function .hljs-title),
.markdown-content :deep(.hljs-title.function_) {
  color: #82aaff;
}

.markdown-content :deep(.hljs-variable),
.markdown-content :deep(.hljs-params) {
  color: #eeffff;
}
</style>
