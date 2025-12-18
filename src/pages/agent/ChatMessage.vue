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

const showThinking = ref(false)

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
    if (unclosed) thinkBlocks.push(unclosed + ' ...')
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

// 解析内容，提取任务规划/步骤/工具调用/结果等（CoT 任务规划格式）
const parsedContent = computed(() => {
  const content = props.message.content || ''
  const steps = []

  // 先清理 think 标签
  const cleanedContent = cleanContent(content)

  // 检测是否是 CoT 格式（包含 Planning: 或 Step X: 等标记）
  const isCotFormat = /^(Planning:|Step\s*\d+:)/m.test(cleanedContent)

  // 如果不是 CoT 格式，直接返回清理后的内容作为 response
  if (!isCotFormat) {
    return {
      steps: [],
      response: cleanedContent,
      isStreaming: false,
      hasSummary: true,
    }
  }

  // 按行解析 CoT 格式
  const lines = content.split('\n')
  let currentType = null
  let currentContent = []
  let summary = ''
  let remainingContent = [] // 非 CoT 格式的剩余内容
  let inThinkBlock = false

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]

    // 跟踪 <think> 块
    if (line.includes('<think>')) {
      inThinkBlock = true
      continue
    }
    if (line.includes('</think>')) {
      inThinkBlock = false
      continue
    }
    // 跳过 think 块内的内容
    if (inThinkBlock) {
      continue
    }

    if (line.startsWith('Planning:')) {
      // 保存之前的内容
      if (currentType && currentContent.length > 0) {
        const cleaned = cleanContent(currentContent.join('\n'))
        if (cleaned) steps.push({ type: currentType, content: cleaned })
      }
      currentType = 'planning'
      currentContent = [line.substring(9).trim()]
      remainingContent = [] // 进入 CoT 模式，清空剩余内容
    } else if (line.match(/^Step\s*\d+:/)) {
      if (currentType && currentContent.length > 0) {
        const cleaned = cleanContent(currentContent.join('\n'))
        if (cleaned) steps.push({ type: currentType, content: cleaned })
      }
      currentType = 'step'
      currentContent = [line.replace(/^Step\s*\d+:\s*/, '').trim()]
      remainingContent = []
    } else if (line.startsWith('Tool:')) {
      if (currentType && currentContent.length > 0) {
        const cleaned = cleanContent(currentContent.join('\n'))
        if (cleaned) steps.push({ type: currentType, content: cleaned })
      }
      currentType = 'tool'
      currentContent = [line.substring(5).trim()]
    } else if (line.startsWith('Tool Input:')) {
      // Tool Input 追加到 tool
      if (currentType === 'tool') {
        currentContent.push(line.substring(11).trim())
      }
    } else if (line.startsWith('Result:')) {
      if (currentType && currentContent.length > 0) {
        const cleaned = cleanContent(currentContent.join('\n'))
        if (cleaned) steps.push({ type: currentType, content: cleaned })
      }
      currentType = 'result'
      currentContent = [line.substring(7).trim()]
    } else if (line.startsWith('Summary:')) {
      if (currentType && currentContent.length > 0) {
        const cleaned = cleanContent(currentContent.join('\n'))
        if (cleaned) steps.push({ type: currentType, content: cleaned })
      }
      currentType = null
      currentContent = []
      summary = line.substring(8).trim()
      // 后续所有内容都是 Summary
      for (let j = i + 1; j < lines.length; j++) {
        summary += '\n' + lines[j]
      }
      summary = cleanContent(summary)
      break
    } else if (currentType) {
      // 检测到 Markdown 标题或表格，结束当前 CoT 步骤，开始累积为最终响应
      if (line.match(/^#+\s/) || line.match(/^\|.*\|$/)) {
        // 保存当前步骤
        if (currentContent.length > 0) {
          const cleaned = cleanContent(currentContent.join('\n'))
          if (cleaned) steps.push({ type: currentType, content: cleaned })
        }
        currentType = null
        currentContent = []
        // 开始累积剩余内容
        remainingContent.push(line)
      } else if (line.trim() !== 'Result') {
        // 继续当前类型的内容
        currentContent.push(line)
      }
    } else {
      // 不在任何 CoT 类型中，累积为剩余内容
      remainingContent.push(line)
    }
  }

  // 保存最后的内容
  if (currentType && currentContent.length > 0) {
    const cleaned = cleanContent(currentContent.join('\n'))
    if (cleaned) steps.push({ type: currentType, content: cleaned })
  }

  // 如果没有 Summary 但有剩余内容，把剩余内容当作 response
  if (!summary && remainingContent.length > 0) {
    summary = cleanContent(remainingContent.join('\n'))
  }

  // 检测流式状态
  const isStreaming =
    (content.includes('Planning:') || content.includes('Step')) && !content.includes('Summary:') && !summary

  return {
    steps,
    response: summary,
    isStreaming,
    hasSummary: !!summary,
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

// 判断是否有思考过程（兼容旧逻辑）
const hasThinking = computed(() => {
  return hasSteps.value && !hasCotChain.value
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
    case 'planning':
      return { icon: 'pi-list-check', color: 'planning' }
    case 'step':
      return { icon: 'pi-play', color: 'step' }
    case 'tool':
      return { icon: 'pi-bolt', color: 'tool' }
    case 'result':
      return { icon: 'pi-check-circle', color: 'result' }
    default:
      return { icon: 'pi-circle', color: '' }
  }
}

function getStepLabel(type) {
  switch (type) {
    case 'planning':
      return '任务规划'
    case 'step':
      return '执行步骤'
    case 'tool':
      return '工具调用'
    case 'result':
      return '执行结果'
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
        <!-- Think 思考过程 -->
        <div
          v-if="hasThinkingContent"
          class="thinking-block mb-3">
          <button
            class="flex items-center gap-1.5 text-xs text-surface-500 hover:text-surface-700 dark:hover:text-surface-300 transition-colors"
            @click="showThinking = !showThinking">
            <i
              class="pi text-xs"
              :class="showThinking ? 'pi-chevron-down' : 'pi-chevron-right'" />
            <i class="pi pi-lightbulb text-amber-500" />
            <span>思考过程</span>
          </button>
          <Transition name="fade-slide">
            <div
              v-show="showThinking"
              class="thinking-content">
              <pre class="whitespace-pre-wrap text-sm">{{ thinkingContent }}</pre>
            </div>
          </Transition>
        </div>

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
                <div class="timeline-label">{{ getStepLabel(step.type) }}</div>
                <div class="timeline-text selectable text-animate">{{ step.content }}</div>
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

/* 任务规划类型 - 紫色 */
.timeline-item.planning .timeline-marker i {
  color: #8b5cf6;
}

.timeline-item.planning .timeline-content {
  background-color: color-mix(in srgb, #8b5cf6 8%, transparent);
  border-color: color-mix(in srgb, #8b5cf6 20%, transparent);
}

.timeline-item.planning .timeline-label {
  color: #7c3aed;
}

/* 执行步骤类型 - 橙色 */
.timeline-item.step .timeline-marker i {
  color: #f59e0b;
}

.timeline-item.step .timeline-content {
  background-color: color-mix(in srgb, #f59e0b 8%, transparent);
  border-color: color-mix(in srgb, #f59e0b 20%, transparent);
}

.timeline-item.step .timeline-label {
  color: #d97706;
}

/* 工具调用类型 - 蓝色 */
.timeline-item.tool .timeline-marker i {
  color: #3b82f6;
}

.timeline-item.tool .timeline-content {
  background-color: color-mix(in srgb, #3b82f6 8%, transparent);
  border-color: color-mix(in srgb, #3b82f6 20%, transparent);
}

.timeline-item.tool .timeline-label {
  color: #2563eb;
}

/* 执行结果类型 - 绿色 */
.timeline-item.result .timeline-marker i {
  color: #10b981;
}

.timeline-item.result .timeline-content {
  background-color: color-mix(in srgb, #10b981 8%, transparent);
  border-color: color-mix(in srgb, #10b981 20%, transparent);
}

.timeline-item.result .timeline-label {
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

/* 步骤平滑展开动画 */
.step-slide-enter-active {
  animation: step-enter 0.4s ease-out;
}

.step-slide-leave-active {
  animation: step-leave 0.3s ease-in;
}

.step-slide-move {
  transition: transform 0.4s ease;
}

@keyframes step-enter {
  0% {
    opacity: 0;
    transform: translateY(-10px);
    max-height: 0;
  }
  50% {
    opacity: 0.5;
    max-height: 100px;
  }
  100% {
    opacity: 1;
    transform: translateY(0);
    max-height: 500px;
  }
}

@keyframes step-leave {
  0% {
    opacity: 1;
    transform: translateY(0);
  }
  100% {
    opacity: 0;
    transform: translateY(-10px);
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
  background-color: color-mix(in srgb, #f59e0b 8%, transparent);
  border-radius: 0.375rem;
  padding: 0.5rem 0.75rem;
  border: 1px solid color-mix(in srgb, #f59e0b 20%, transparent);
  transition: all 0.3s ease;
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

.app-dark .timeline-item.planning .timeline-content {
  background-color: color-mix(in srgb, #8b5cf6 12%, transparent);
  border-color: color-mix(in srgb, #8b5cf6 25%, transparent);
}

.app-dark .timeline-item.step .timeline-content {
  background-color: color-mix(in srgb, #f59e0b 12%, transparent);
  border-color: color-mix(in srgb, #f59e0b 25%, transparent);
}

.app-dark .timeline-item.tool .timeline-content {
  background-color: color-mix(in srgb, #3b82f6 12%, transparent);
  border-color: color-mix(in srgb, #3b82f6 25%, transparent);
}

.app-dark .timeline-item.result .timeline-content {
  background-color: color-mix(in srgb, #10b981 12%, transparent);
  border-color: color-mix(in srgb, #10b981 25%, transparent);
}

/* CoT 思维链样式 */
.cot-steps {
  margin-top: 0.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.cot-step {
  padding: 0.5rem 0.75rem;
  border-radius: 0.25rem;
  border-left: 3px solid;
  font-size: 0.875rem;
}

.cot-step.planning {
  background-color: color-mix(in srgb, #8b5cf6 10%, transparent);
  border-color: #8b5cf6;
}

.cot-step.step {
  background-color: color-mix(in srgb, #f59e0b 10%, transparent);
  border-color: #f59e0b;
}

.cot-step.tool {
  background-color: color-mix(in srgb, #3b82f6 10%, transparent);
  border-color: #3b82f6;
}

.cot-step.result {
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

.cot-step.planning .step-header {
  color: #7c3aed;
}

.cot-step.step .step-header {
  color: #d97706;
}

.cot-step.tool .step-header {
  color: #2563eb;
}

.cot-step.result .step-header {
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
