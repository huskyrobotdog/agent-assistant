<script setup>
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Card from 'primevue/card'
import Button from 'primevue/button'
import Message from 'primevue/message'
import * as monaco from 'monaco-editor'
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker'
import jsonWorker from 'monaco-editor/esm/vs/language/json/json.worker?worker'

// 配置 Monaco Editor workers
self.MonacoEnvironment = {
  getWorker(_, label) {
    if (label === 'json') {
      return new jsonWorker()
    }
    return new editorWorker()
  },
}

const editorContainer = ref(null)
let editor = null

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const success = ref('')
const hasErrors = ref(false)

// 获取编辑器内容
const getEditorValue = () => {
  return editor?.getValue() || ''
}

// 设置编辑器内容
const setEditorValue = (value) => {
  if (editor) {
    editor.setValue(value)
  }
}

// 加载配置
const loadConfig = async () => {
  loading.value = true
  error.value = ''
  try {
    const config = await invoke('get_mcp_config')
    setEditorValue(JSON.stringify(config, null, 2))
  } catch (e) {
    error.value = `加载配置失败: ${e}`
  } finally {
    loading.value = false
  }
}

// 保存配置
const saveConfig = async () => {
  if (hasErrors.value) {
    error.value = 'JSON 格式无效，请检查后重试'
    return
  }

  saving.value = true
  error.value = ''
  success.value = ''
  try {
    const config = JSON.parse(getEditorValue())
    await invoke('save_mcp_config', { config })
    success.value = '配置保存成功'
    setTimeout(() => (success.value = ''), 3000)
  } catch (e) {
    error.value = `保存配置失败: ${e}`
  } finally {
    saving.value = false
  }
}

// 格式化 JSON
const formatJson = () => {
  if (editor) {
    editor.getAction('editor.action.formatDocument')?.run()
  }
}

// 检测暗色模式
const isDarkMode = () => {
  return document.documentElement.classList.contains('app-dark')
}

// 初始化编辑器
// 更新编辑器高度
const updateEditorHeight = () => {
  if (!editor || !editorContainer.value) return
  const contentHeight = Math.max(200, editor.getContentHeight())
  editorContainer.value.style.height = `${contentHeight}px`
  editor.layout()
}

const initEditor = () => {
  if (!editorContainer.value) return

  editor = monaco.editor.create(editorContainer.value, {
    value: '{\n  "mcpServers": {}\n}',
    language: 'json',
    theme: isDarkMode() ? 'vs-dark' : 'vs',
    automaticLayout: true,
    minimap: { enabled: false },
    fontSize: 14,
    lineNumbers: 'on',
    scrollBeyondLastLine: false,
    wordWrap: 'on',
    tabSize: 2,
    formatOnPaste: true,
    formatOnType: true,
    scrollbar: {
      vertical: 'hidden',
      horizontal: 'hidden',
      handleMouseWheel: false,
    },
  })

  // 监听内容变化，自动调整高度
  editor.onDidContentSizeChange(() => {
    updateEditorHeight()
  })

  // 监听错误标记
  monaco.editor.onDidChangeMarkers((uris) => {
    const model = editor?.getModel()
    if (model) {
      const markers = monaco.editor.getModelMarkers({ resource: model.uri })
      hasErrors.value = markers.some((m) => m.severity === monaco.MarkerSeverity.Error)
    }
  })

  // 初始化高度
  updateEditorHeight()
}

// 监听暗色模式变化
const observeDarkMode = () => {
  const observer = new MutationObserver(() => {
    if (editor) {
      monaco.editor.setTheme(isDarkMode() ? 'vs-dark' : 'vs')
    }
  })
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class'],
  })
  return observer
}

let darkModeObserver = null

onMounted(() => {
  initEditor()
  darkModeObserver = observeDarkMode()
  loadConfig()
})

onUnmounted(() => {
  editor?.dispose()
  darkModeObserver?.disconnect()
})
</script>

<template>
  <Card>
    <template #title>
      <div class="flex items-center justify-between">
        <span>MCP 服务器配置</span>
        <div class="flex gap-2">
          <Button
            icon="pi pi-refresh"
            text
            rounded
            :loading="loading"
            @click="loadConfig"
            v-tooltip="'刷新'" />
          <Button
            icon="pi pi-align-left"
            text
            rounded
            @click="formatJson"
            v-tooltip="'格式化'" />
        </div>
      </div>
    </template>
    <template #subtitle>配置 Model Context Protocol 服务器连接</template>
    <template #content>
      <Message
        v-if="error"
        severity="error"
        :closable="true"
        @close="error = ''">
        {{ error }}
      </Message>
      <Message
        v-if="success"
        severity="success"
        :closable="true"
        @close="success = ''">
        {{ success }}
      </Message>

      <div class="editor-wrapper mt-4">
        <div
          ref="editorContainer"
          class="editor-container"></div>
      </div>

      <div class="flex justify-end mt-4">
        <Button
          icon="pi pi-save"
          label="保存配置"
          :loading="saving"
          :disabled="hasErrors"
          @click="saveConfig" />
      </div>
    </template>
  </Card>
</template>

<style scoped>
.editor-wrapper {
  border: 1px solid var(--p-surface-300);
  border-radius: 8px;
  overflow: hidden;
}

.editor-container {
  min-height: 200px;
  width: 100%;
}

.app-dark .editor-wrapper {
  border-color: var(--p-surface-700);
}
</style>
