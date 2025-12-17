<script setup>
import { ref, computed } from 'vue'

const props = defineProps({
  disabled: {
    type: Boolean,
    default: false,
  },
  loading: {
    type: Boolean,
    default: false,
  },
  maxLength: {
    type: Number,
    default: 2000,
  },
})

const emit = defineEmits(['send'])

const inputMessage = ref('')

const charCount = computed(() => inputMessage.value.length)

function handleSend() {
  if (!inputMessage.value.trim() || props.disabled) return

  emit('send', inputMessage.value)
  inputMessage.value = ''
}

function handleKeydown(event) {
  if (event.isComposing || event.keyCode === 229) return

  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    handleSend()
  }
}
</script>

<template>
  <div class="input-wrapper">
    <!-- 旋转光晕 -->
    <div
      v-if="loading"
      class="glow-wrapper">
      <div class="glow-spot"></div>
    </div>
    <div class="input-outer">
      <div class="input-container">
        <Textarea
          v-model="inputMessage"
          placeholder="输入消息..."
          :autoResize="false"
          rows="3"
          :maxlength="maxLength"
          class="message-input"
          @keydown="handleKeydown" />
        <div class="input-footer">
          <span class="char-count">{{ charCount }}/{{ maxLength }}</span>
          <Button
            icon="pi pi-send"
            text
            rounded
            :disabled="!inputMessage.trim() || disabled"
            class="send-button"
            @click="handleSend" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.input-wrapper {
  padding: 1rem;
  border-top: 1px solid var(--p-surface-200);
  position: relative;
}

.app-dark .input-wrapper {
  border-color: var(--p-surface-700);
}

/* 旋转光晕容器 */
.glow-wrapper {
  position: absolute;
  top: 0.75rem;
  left: 0.75rem;
  right: 0.75rem;
  bottom: 0.75rem;
  border-radius: 0.75rem;
  overflow: hidden;
}

/* 旋转的光斑 - 单条彩虹光带 */
.glow-spot {
  position: absolute;
  width: 200%;
  height: 200%;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  background: conic-gradient(
    from 0deg,
    transparent 0deg,
    transparent 315deg,
    #3b82f6 325deg,
    #22c55e 335deg,
    #eab308 345deg,
    #f97316 352deg,
    #ef4444 358deg,
    transparent 360deg
  );
  animation: rotate-glow 2s linear infinite;
  filter: blur(12px);
  opacity: 1;
}

@keyframes rotate-glow {
  0% {
    transform: translate(-50%, -50%) rotate(0deg);
  }
  100% {
    transform: translate(-50%, -50%) rotate(360deg);
  }
}

.input-outer {
  position: relative;
  border-radius: 0.75rem;
  padding: 1px;
  background: var(--p-surface-200);
  z-index: 1;
}

.app-dark .input-outer {
  background: var(--p-surface-700);
}

.input-container {
  display: flex;
  flex-direction: column;
  background-color: var(--p-surface-100);
  border-radius: 0.625rem;
  position: relative;
  z-index: 1;
}

.app-dark .input-container {
  background-color: var(--p-surface-800);
}

.message-input {
  width: 100%;
  border: none !important;
  background: transparent !important;
  box-shadow: none !important;
  padding: 0.75rem 1rem;
  resize: none;
}

.message-input :deep(textarea) {
  resize: none;
}

.message-input:focus {
  outline: none !important;
  box-shadow: none !important;
}

.input-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 0.5rem;
  padding: 0.25rem 0.5rem 0.5rem;
}

.char-count {
  font-size: 0.75rem;
  color: var(--p-surface-400);
}

.send-button {
  flex-shrink: 0;
}
</style>
