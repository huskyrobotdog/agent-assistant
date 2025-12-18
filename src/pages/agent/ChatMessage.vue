<script setup>
import { ref, computed, TransitionGroup } from 'vue'
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

const showThinking = ref(true)

const isUser = computed(() => props.message.role === 'user')

// 提取 think 块内容
const thinkingContent = computed(() => {
  const content = props.message.content || ''
  const thinkBlocks = []
  const regex = /<think>([\s\S]*?)<\/think>/g
  let match
  while ((match = regex.exec(content)) !== null) {
    const text = match[1].trim()
    if (text) thinkBlocks.push(text)
  }
  // 检查未闭合的 <think> 块
  const lastOpenIdx = content.lastIndexOf('<think>')
  const lastCloseIdx = content.lastIndexOf('</think>')
  if (lastOpenIdx > lastCloseIdx) {
    const unclosed = content.substring(lastOpenIdx + 7).trim()
    if (unclosed) thinkBlocks.push(unclosed)
  }
  return thinkBlocks.join('\n\n')
})

const hasThinkingContent = computed(() => !!thinkingContent.value)

// 清理内容：移除 <think> 标签及其内容
function cleanContent(text) {
  // 移除 <think>...</think> 标签及内容
  let cleaned = text.replace(/<think>[\s\S]*?<\/think>/g, '')
  // 移除未闭合的 <think> 标签及后续内容
  const thinkIdx = cleaned.indexOf('<think>')
  if (thinkIdx !== -1) {
    cleaned = cleaned.substring(0, thinkIdx)
  }
  // 移除残留的 "Result" 词（当 stop word 生效时可能残留）
  cleaned = cleaned.replace(/\nResult\s*$/g, '')
  cleaned = cleaned.replace(/^Result\s*$/gm, '')
  return cleaned.trim()
}

// 解析内容，按顺序提取 <think> 块和 ReAct 格式
const parsedContent = computed(() => {
  const content = props.message.content || ''
  const steps = []
  let finalAnswer = ''

  // 分段解析：按 <think> 块和普通内容交替处理
  const segments = []
  let remaining = content
  const thinkRegex = /<think>([\s\S]*?)<\/think>/g
  let lastIndex = 0
  let match

  while ((match = thinkRegex.exec(content)) !== null) {
    // 添加 think 块之前的普通内容
    if (match.index > lastIndex) {
      segments.push({ type: 'text', content: content.substring(lastIndex, match.index) })
    }
    // 添加 think 块
    segments.push({ type: 'think', content: match[1].trim() })
    lastIndex = match.index + match[0].length
  }
  // 添加最后的普通内容
  if (lastIndex < content.length) {
    segments.push({ type: 'text', content: content.substring(lastIndex) })
  }

  // 检查未闭合的 <think> 块
  const lastOpenIdx = content.lastIndexOf('<think>')
  const lastCloseIdx = content.lastIndexOf('</think>')
  if (lastOpenIdx > lastCloseIdx) {
    // 移除之前可能添加的不完整内容
    const unclosedContent = content.substring(lastOpenIdx + 7).trim()
    if (unclosedContent) {
      // 找到并更新最后一个 text segment
      const lastTextIdx = segments.findLastIndex((s) => s.type === 'text')
      if (lastTextIdx >= 0) {
        segments[lastTextIdx].content = segments[lastTextIdx].content.replace(/<think>[\s\S]*$/, '')
      }
      segments.push({ type: 'think', content: unclosedContent })
    }
  }

  // 解析每个段落
  for (const segment of segments) {
    if (segment.type === 'think' && segment.content) {
      steps.push({ type: 'thinking', content: segment.content })
    } else if (segment.type === 'text') {
      // 解析 ReAct 格式
      const text = segment.content.trim()
      if (!text) continue

      const lines = text.split('\n')
      let currentType = null
      let currentContent = []
      let currentToolCall = null

      function saveCurrentStep() {
        if (currentType === 'thought' && currentContent.length > 0) {
          const t = currentContent.join('\n').trim()
          if (t) steps.push({ type: 'thought', content: t })
        }
        currentContent = []
      }

      function saveToolCall() {
        if (currentToolCall) {
          steps.push({ ...currentToolCall })
          currentToolCall = null
        }
      }

      for (let i = 0; i < lines.length; i++) {
        const line = lines[i]

        if (line.startsWith('Thought:')) {
          saveCurrentStep()
          saveToolCall()
          currentType = 'thought'
          currentContent = [line.substring(8).trim()]
        } else if (line.startsWith('Action:')) {
          saveCurrentStep()
          saveToolCall()
          currentType = 'action'
          currentToolCall = {
            type: 'tool_call',
            actionName: line.substring(7).trim(),
            actionInput: '',
            observation: '',
          }
        } else if (line.startsWith('Action Input:')) {
          if (currentToolCall) {
            currentToolCall.actionInput = line.substring(13).trim()
          }
        } else if (line.startsWith('Observation:') || line.startsWith('Observ')) {
          if (currentToolCall) {
            const t = line.startsWith('Observation:') ? line.substring(12).trim() : line.substring(6).trim()
            currentToolCall.observation = t
          }
          currentType = 'observation'
        } else if (line.startsWith('Final Answer:')) {
          saveCurrentStep()
          saveToolCall()
          currentType = null
          finalAnswer = line.substring(13).trim()
          for (let j = i + 1; j < lines.length; j++) {
            finalAnswer += '\n' + lines[j]
          }
          finalAnswer = finalAnswer.trim()
          break
        } else if (currentType === 'thought') {
          currentContent.push(line)
        } else if (currentType === 'observation' && currentToolCall) {
          currentToolCall.observation += '\n' + line
        } else if (currentToolCall && !currentToolCall.observation) {
          currentToolCall.actionInput += '\n' + line
        }
      }

      saveCurrentStep()
      saveToolCall()
    }
  }

  // 检测流式状态
  const hasReactContent = steps.some((s) => s.type === 'thought' || s.type === 'tool_call')
  const isStreaming = hasReactContent && !finalAnswer

  return {
    steps,
    response: finalAnswer,
    isStreaming,
    hasSummary: !!finalAnswer,
  }
})

// 获取流式步骤（已整合到 parsedContent 中，保留兼容）
const streamingStep = computed(() => {
  return null
})

const renderedContent = computed(() => {
  return marked(parsedContent.value.response)
})

// 判断是否有步骤要显示
const hasSteps = computed(() => {
  return parsedContent.value.steps.length > 0 || streamingStep.value
})

// 解析 CoT 思维链步骤
const cotSteps = computed(() => {
  if (!props.message.cotChain) return []
  return props.message.cotChain
})

// 判断是否有 CoT 思维链
const hasCotChain = computed(() => {
  return props.message.cotChain && props.message.cotChain.length > 0
})

// 判断是否有思考过程（包含步骤）
const hasThinking = computed(() => {
  return hasSteps.value && !hasCotChain.value
})

// 获取所有步骤（已按顺序包含 thinking 步骤）
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
    case 'thinking':
      return { icon: 'pi-sparkles', color: 'thinking' }
    case 'thought':
      return { icon: 'pi-lightbulb', color: 'thought' }
    case 'tool_call':
      return { icon: 'pi-wrench', color: 'tool_call' }
    default:
      return { icon: 'pi-circle', color: '' }
  }
}

function getStepLabel(step) {
  const type = typeof step === 'string' ? step : step.type
  switch (type) {
    case 'thinking':
      return '推理'
    case 'thought':
      return '思考'
    case 'tool_call':
      return step.actionName ? `工具调用: ${step.actionName}` : '工具调用'
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
      class="message-bubble rounded-lg"
      :class="isUser ? 'user-message max-w-[70%]' : 'assistant-message w-full'">
      <!-- 用户消息 -->
      <template v-if="isUser">
        <p class="whitespace-pre-wrap">{{ message.content }}</p>
      </template>

      <!-- AI 消息 -->
      <template v-else>
        <!-- CoT 思维链 -->
        <div
          v-if="hasCotChain"
          class="cot-chain mb-3">
          <button
            class="flex items-center gap-1 text-xs text-surface-500 hover:text-surface-700 dark:hover:text-surface-300 transition-colors"
            @click="toggleThinking">
            <i
              class="pi"
              :class="showThinking ? 'pi-chevron-down' : 'pi-chevron-right'" />
            <i class="pi pi-sitemap" />
            <span>推理过程 ({{ cotSteps.length }} 步)</span>
          </button>
          <div
            v-show="showThinking"
            class="cot-steps">
            <div
              v-for="(step, index) in cotSteps"
              :key="index"
              class="cot-step"
              :class="getStepStyle(step.type).color">
              <div class="step-header">
                <i
                  class="pi"
                  :class="getStepStyle(step.type).icon" />
                <span class="step-label">{{ getStepLabel(step) }}</span>
              </div>
              <!-- 工具调用特殊显示 -->
              <div
                v-if="step.type === 'tool' && step.toolInput"
                class="tool-input-box">
                <pre class="tool-input-code">{{ step.toolInput }}</pre>
              </div>
              <div
                v-else
                class="step-content selectable">
                {{ step.content }}
              </div>
            </div>
          </div>
        </div>

        <!-- 思考/操作过程 - 时间线UI -->
        <div
          v-else-if="hasThinking"
          class="thinking-timeline mb-3">
          <TransitionGroup name="step-slide">
            <div
              v-for="(step, index) in allSteps"
              :key="`${step.type}-${index}`"
              class="timeline-item"
              :class="[getStepStyle(step.type).color, { 'is-streaming': step.isStreaming }]">
              <div class="timeline-marker">
                <i
                  class="pi"
                  :class="getStepStyle(step.type).icon" />
              </div>
              <div class="timeline-content">
                <div class="timeline-label">{{ getStepLabel(step) }}</div>
                <!-- 工具调用特殊显示 -->
                <template v-if="step.type === 'tool_call'">
                  <div
                    v-if="step.actionInput"
                    class="timeline-text selectable">
                    {{ step.actionInput }}
                  </div>
                  <div
                    v-if="step.observation"
                    class="tool-observation">
                    <div class="observation-label">执行结果</div>
                    <div class="timeline-text selectable">{{ step.observation }}</div>
                  </div>
                </template>
                <div
                  v-else
                  class="timeline-text selectable text-animate">
                  {{ step.content }}
                </div>
              </div>
            </div>
          </TransitionGroup>
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

/* Think 思考过程样式 */
.thinking-block {
  border-left: 2px solid #f59e0b;
  padding-left: 0.75rem;
}

.thinking-content {
  margin-top: 0.5rem;
  padding: 0.75rem;
  background-color: color-mix(in srgb, #f59e0b 8%, transparent);
  border-radius: 0.375rem;
  color: var(--p-surface-600);
  font-family: ui-monospace, monospace;
  font-size: 0.8125rem;
  line-height: 1.6;
  max-height: 300px;
  overflow-y: auto;
}

.app-dark .thinking-content {
  background-color: color-mix(in srgb, #f59e0b 12%, transparent);
  color: var(--p-surface-400);
}

/* 动画 */
.fade-slide-enter-active,
.fade-slide-leave-active {
  transition: all 0.3s ease;
}

.fade-slide-enter-from,
.fade-slide-leave-to {
  opacity: 0;
  transform: translateY(-8px);
  max-height: 0;
}

/* 时间线样式 */
.thinking-timeline {
  position: relative;
  padding-left: 1.5rem;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
}

.thinking-timeline::before {
  content: '';
  position: absolute;
  left: 0.4375rem;
  top: 0.5rem;
  bottom: 0.5rem;
  width: 2px;
  background: linear-gradient(
    180deg,
    #ef4444 0%,
    #f97316 16%,
    #eab308 32%,
    #22c55e 48%,
    #3b82f6 64%,
    #8b5cf6 80%,
    #ec4899 100%
  );
  opacity: 0.6;
}

.timeline-item {
  position: relative;
  padding-bottom: 1rem;
  display: inline-block;
  width: auto;
  max-width: 85%;
}

.timeline-item:last-child {
  padding-bottom: 0;
}

.timeline-item + .timeline-item {
  margin-top: 0.25rem;
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

/* 推理类型 - 灰色 */
.timeline-item.thinking .timeline-marker i {
  color: #64748b;
}

.timeline-item.thinking .timeline-content {
  background-color: color-mix(in srgb, #64748b 6%, transparent);
  border-color: color-mix(in srgb, #64748b 15%, transparent);
}

.timeline-item.thinking .timeline-label {
  color: #475569;
}

/* 思考类型 - 青蓝色 */
.timeline-item.thought .timeline-marker i {
  color: #0ea5e9;
}

.timeline-item.thought .timeline-content {
  background-color: color-mix(in srgb, #0ea5e9 6%, transparent);
  border-color: color-mix(in srgb, #0ea5e9 15%, transparent);
}

.timeline-item.thought .timeline-label {
  color: #0284c7;
}

/* 工具调用类型 - 青色 */
.timeline-item.tool_call .timeline-marker i {
  color: #14b8a6;
}

.timeline-item.tool_call .timeline-content {
  background-color: color-mix(in srgb, #14b8a6 6%, transparent);
  border-color: color-mix(in srgb, #14b8a6 15%, transparent);
}

.timeline-item.tool_call .timeline-label {
  color: #0d9488;
}

/* 执行结果样式 */
.tool-observation {
  margin-top: 0.75rem;
  padding-top: 0.75rem;
  border-top: 1px solid var(--p-surface-200);
}

.app-dark .tool-observation {
  border-top-color: var(--p-surface-600);
}

.observation-label {
  font-size: 0.7rem;
  font-weight: 600;
  color: #0d9488;
  margin-bottom: 0.375rem;
  text-transform: uppercase;
  letter-spacing: 0.025em;
}

/* 工具输入框样式 - 固定高度可滚动 */
.tool-input-box {
  max-height: 120px;
  overflow-y: auto;
  background-color: var(--p-surface-100);
  border-radius: 0.375rem;
  margin-top: 0.25rem;
}

.app-dark .tool-input-box {
  background-color: var(--p-surface-800);
}

.tool-input-code {
  margin: 0;
  padding: 0.5rem 0.75rem;
  font-family: ui-monospace, 'Fira Code', 'Monaco', 'Consolas', monospace;
  font-size: 0.75rem;
  line-height: 1.5;
  color: var(--p-surface-700);
  white-space: pre-wrap;
  word-break: break-all;
}

.app-dark .tool-input-code {
  color: var(--p-surface-300);
}

/* 滚动条样式 */
.tool-input-box::-webkit-scrollbar {
  width: 4px;
}

.tool-input-box::-webkit-scrollbar-track {
  background: transparent;
}

.tool-input-box::-webkit-scrollbar-thumb {
  background-color: var(--p-surface-300);
  border-radius: 2px;
}

.app-dark .tool-input-box::-webkit-scrollbar-thumb {
  background-color: var(--p-surface-600);
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

/* 步骤平滑展开动画 */
.step-slide-enter-active {
  animation: step-enter 0.5s cubic-bezier(0.4, 0, 0.2, 1);
}

.step-slide-leave-active {
  animation: step-leave 0.3s ease-in;
}

.step-slide-move {
  transition: transform 0.4s cubic-bezier(0.4, 0, 0.2, 1);
}

@keyframes step-enter {
  0% {
    opacity: 0;
    transform: translateY(-10px) scale(0.95);
    max-height: 0;
  }
  60% {
    opacity: 0.8;
    transform: translateY(0) scale(1);
    max-height: 200px;
  }
  100% {
    opacity: 1;
    transform: translateY(0) scale(1);
    max-height: 1000px;
  }
}

@keyframes step-leave {
  0% {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
  100% {
    opacity: 0;
    transform: translateY(-10px) scale(0.95);
  }
}

/* 文字渐变动画 */
.text-animate {
  animation: text-fade-in 0.5s ease-out;
}

@keyframes text-fade-in {
  0% {
    opacity: 0.3;
  }
  100% {
    opacity: 1;
  }
}

/* 流式输出时的文字渐变效果 */
.is-streaming .timeline-text {
  background: linear-gradient(90deg, var(--p-surface-600) 0%, var(--p-surface-400) 50%, var(--p-surface-600) 100%);
  background-size: 200% auto;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: text-shimmer 2s linear infinite;
}

.app-dark .is-streaming .timeline-text {
  background: linear-gradient(90deg, var(--p-surface-400) 0%, var(--p-surface-200) 50%, var(--p-surface-400) 100%);
  background-size: 200% auto;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

@keyframes text-shimmer {
  0% {
    background-position: 200% center;
  }
  100% {
    background-position: -200% center;
  }
}

.timeline-content {
  background-color: var(--p-surface-50);
  border-radius: 0.5rem;
  padding: 0.75rem 1rem;
  border: 1px solid var(--p-surface-200);
  transition: all 0.4s cubic-bezier(0.4, 0, 0.2, 1);
  width: fit-content;
  max-width: 100%;
}

.app-dark .timeline-content {
  background-color: var(--p-surface-800);
  border-color: var(--p-surface-700);
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

.app-dark .timeline-item.thinking .timeline-content {
  background-color: color-mix(in srgb, #64748b 10%, transparent);
  border-color: color-mix(in srgb, #64748b 20%, transparent);
}

.app-dark .timeline-item.thought .timeline-content {
  background-color: color-mix(in srgb, #0ea5e9 10%, transparent);
  border-color: color-mix(in srgb, #0ea5e9 20%, transparent);
}

.app-dark .timeline-item.tool_call .timeline-content {
  background-color: color-mix(in srgb, #14b8a6 10%, transparent);
  border-color: color-mix(in srgb, #14b8a6 20%, transparent);
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
  background-color: var(--p-surface-100);
  border-radius: 0.5rem;
  padding: 1rem;
  overflow-x: auto;
  margin: 0.5rem 0;
}

.app-dark .markdown-content :deep(pre) {
  background-color: var(--p-surface-800);
}

.markdown-content :deep(code) {
  font-family: 'Fira Code', 'Monaco', 'Consolas', monospace;
  font-size: 0.875rem;
  color: var(--p-surface-700);
}

.app-dark .markdown-content :deep(code) {
  color: var(--p-surface-200);
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

/* highlight.js 代码主题 - 亮色模式 */
.markdown-content :deep(.hljs-keyword),
.markdown-content :deep(.hljs-selector-tag),
.markdown-content :deep(.hljs-built_in),
.markdown-content :deep(.hljs-name) {
  color: #7c3aed;
}

.markdown-content :deep(.hljs-string),
.markdown-content :deep(.hljs-attr) {
  color: #16a34a;
}

.markdown-content :deep(.hljs-number),
.markdown-content :deep(.hljs-literal) {
  color: #ea580c;
}

.markdown-content :deep(.hljs-comment) {
  color: #6b7280;
  font-style: italic;
}

.markdown-content :deep(.hljs-function .hljs-title),
.markdown-content :deep(.hljs-title.function_) {
  color: #2563eb;
}

.markdown-content :deep(.hljs-variable),
.markdown-content :deep(.hljs-params) {
  color: #374151;
}

/* highlight.js 代码主题 - 暗色模式 */
.app-dark .markdown-content :deep(.hljs-keyword),
.app-dark .markdown-content :deep(.hljs-selector-tag),
.app-dark .markdown-content :deep(.hljs-built_in),
.app-dark .markdown-content :deep(.hljs-name) {
  color: #c792ea;
}

.app-dark .markdown-content :deep(.hljs-string),
.app-dark .markdown-content :deep(.hljs-attr) {
  color: #c3e88d;
}

.app-dark .markdown-content :deep(.hljs-number),
.app-dark .markdown-content :deep(.hljs-literal) {
  color: #f78c6c;
}

.app-dark .markdown-content :deep(.hljs-comment) {
  color: #546e7a;
  font-style: italic;
}

.app-dark .markdown-content :deep(.hljs-function .hljs-title),
.app-dark .markdown-content :deep(.hljs-title.function_) {
  color: #82aaff;
}

.app-dark .markdown-content :deep(.hljs-variable),
.app-dark .markdown-content :deep(.hljs-params) {
  color: #eeffff;
}
</style>
