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

const renderedContent = computed(() => {
  return marked(props.message.content)
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

// 判断是否有普通思考过程
const hasThinking = computed(() => {
  return props.message.thinking && !hasReactChain.value
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

        <!-- 普通思维链 -->
        <div
          v-else-if="hasThinking"
          class="thinking-toggle mb-2">
          <button
            class="flex items-center gap-1 text-xs text-surface-500 hover:text-surface-700 dark:hover:text-surface-300 transition-colors"
            @click="toggleThinking">
            <i
              class="pi"
              :class="showThinking ? 'pi-chevron-down' : 'pi-chevron-right'" />
            <i class="pi pi-lightbulb" />
            <span>思考过程</span>
          </button>
          <div
            v-show="showThinking"
            class="thinking-content">
            {{ message.thinking }}
          </div>
        </div>

        <!-- Markdown 内容 -->
        <div
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
  background-color: var(--p-surface-100);
  padding: 0.75rem 1rem;
}

:deep(.app-dark) .message-bubble.assistant-message,
.app-dark .message-bubble.assistant-message {
  background-color: var(--p-surface-800);
}

.thinking-content {
  margin-top: 0.5rem;
  padding: 0.75rem;
  background-color: color-mix(in srgb, var(--p-surface-200) 50%, transparent);
  border-radius: 0.25rem;
  font-size: 0.875rem;
  color: var(--p-surface-600);
  white-space: pre-wrap;
  border-left: 2px solid var(--p-primary-color);
}

.app-dark .thinking-content {
  background-color: color-mix(in srgb, var(--p-surface-700) 50%, transparent);
  color: var(--p-surface-400);
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
