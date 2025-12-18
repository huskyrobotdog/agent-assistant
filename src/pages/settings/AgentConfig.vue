<script setup>
import { ref, onMounted } from 'vue'
import { getDb } from '@/utils/db.js'
import Card from 'primevue/card'
import Button from 'primevue/button'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import Dialog from 'primevue/dialog'
import InputText from 'primevue/inputtext'
import Textarea from 'primevue/textarea'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import ConfirmDialog from 'primevue/confirmdialog'
import { useConfirm } from 'primevue/useconfirm'

const confirm = useConfirm()

// 状态
const agents = ref([])
const loading = ref(false)
const dialogVisible = ref(false)
const isEditing = ref(false)
const error = ref('')
const success = ref('')

// 当前编辑的智能体
const currentAgent = ref({
  id: null,
  name: '',
  system_prompt: '',
  allow_tools: true,
})

// 加载智能体列表
const loadAgents = async () => {
  const db = getDb()
  if (!db) return
  loading.value = true
  error.value = ''
  try {
    const result = await db.select('SELECT * FROM agents ORDER BY id ASC')
    agents.value = result.map((row) => ({
      ...row,
      allow_tools: Boolean(row.allow_tools),
    }))
  } catch (e) {
    error.value = `加载智能体列表失败: ${e}`
  } finally {
    loading.value = false
  }
}

// 打开新增对话框
const openAddDialog = () => {
  isEditing.value = false
  currentAgent.value = {
    id: null,
    name: '',
    system_prompt: '',
    allow_tools: true,
  }
  dialogVisible.value = true
}

// 打开编辑对话框
const openEditDialog = (agent) => {
  isEditing.value = true
  currentAgent.value = { ...agent }
  dialogVisible.value = true
}

// 保存智能体
const saveAgent = async () => {
  if (!currentAgent.value.name.trim()) {
    error.value = '智能体名称不能为空'
    return
  }

  error.value = ''
  const db = getDb()
  try {
    if (isEditing.value) {
      await db.execute(
        `UPDATE agents SET name = $1, system_prompt = $2, allow_tools = $3, updated_at = CURRENT_TIMESTAMP WHERE id = $4`,
        [
          currentAgent.value.name.trim(),
          currentAgent.value.system_prompt,
          currentAgent.value.allow_tools ? 1 : 0,
          currentAgent.value.id,
        ]
      )
      success.value = '智能体已更新'
    } else {
      await db.execute(`INSERT INTO agents (name, system_prompt, allow_tools) VALUES ($1, $2, $3)`, [
        currentAgent.value.name.trim(),
        currentAgent.value.system_prompt,
        currentAgent.value.allow_tools ? 1 : 0,
      ])
      success.value = '智能体已创建'
    }
    dialogVisible.value = false
    await loadAgents()
    setTimeout(() => (success.value = ''), 3000)
  } catch (e) {
    if (e.toString().includes('UNIQUE constraint')) {
      error.value = '智能体名称已存在'
    } else {
      error.value = `保存失败: ${e}`
    }
  }
}

// 删除智能体
const deleteAgent = (agent) => {
  confirm.require({
    message: `确定要删除智能体 "${agent.name}" 吗？`,
    header: '确认删除',
    icon: 'pi pi-exclamation-triangle',
    rejectLabel: '取消',
    acceptLabel: '删除',
    rejectProps: {
      severity: 'secondary',
      outlined: true,
    },
    acceptProps: {
      severity: 'danger',
    },
    accept: async () => {
      const db = getDb()
      try {
        await db.execute('DELETE FROM agents WHERE id = $1', [agent.id])
        success.value = '智能体已删除'
        await loadAgents()
        setTimeout(() => (success.value = ''), 3000)
      } catch (e) {
        error.value = `删除失败: ${e}`
      }
    },
  })
}

onMounted(async () => {
  await loadAgents()
})
</script>

<template>
  <Card>
    <template #title>
      <div class="flex items-center justify-between">
        <span>智能体配置</span>
        <div class="flex gap-2">
          <Button
            icon="pi pi-refresh"
            text
            rounded
            :loading="loading"
            @click="loadAgents"
            v-tooltip="'刷新'" />
          <Button
            icon="pi pi-plus"
            text
            rounded
            @click="openAddDialog"
            v-tooltip="'新增智能体'" />
        </div>
      </div>
    </template>
    <template #subtitle>管理智能体的配置信息</template>
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

      <DataTable
        :value="agents"
        :loading="loading"
        stripedRows
        class="mt-4">
        <template #empty>
          <div class="text-center py-8 text-surface-500">
            <i class="pi pi-robot text-4xl mb-2"></i>
            <p>暂无智能体配置</p>
            <Button
              label="创建第一个智能体"
              icon="pi pi-plus"
              class="mt-2"
              @click="openAddDialog" />
          </div>
        </template>
        <Column
          field="name"
          header="名称"
          style="min-width: 120px" />
        <Column
          field="system_prompt"
          header="提示词"
          style="min-width: 200px">
          <template #body="{ data }">
            <span
              class="line-clamp-2 text-sm text-surface-600"
              :title="data.system_prompt">
              {{ data.system_prompt || '-' }}
            </span>
          </template>
        </Column>
        <Column
          field="allow_tools"
          header="允许工具"
          style="width: 100px">
          <template #body="{ data }">
            <i
              :class="['pi', data.allow_tools ? 'pi-check-circle text-green-500' : 'pi-times-circle text-red-500']"></i>
          </template>
        </Column>
        <Column
          header="操作"
          style="width: 120px">
          <template #body="{ data }">
            <div class="flex gap-1">
              <Button
                icon="pi pi-pencil"
                text
                rounded
                size="small"
                @click="openEditDialog(data)"
                v-tooltip="'编辑'" />
              <Button
                icon="pi pi-trash"
                text
                rounded
                size="small"
                severity="danger"
                @click="deleteAgent(data)"
                v-tooltip="'删除'" />
            </div>
          </template>
        </Column>
      </DataTable>
    </template>
  </Card>

  <!-- 新增/编辑对话框 -->
  <Dialog
    v-model:visible="dialogVisible"
    :header="isEditing ? '编辑智能体' : '新增智能体'"
    :style="{ width: '500px' }"
    modal>
    <div class="flex flex-col gap-4 pt-2">
      <div class="flex flex-col gap-2">
        <label
          for="agent-name"
          class="font-medium">
          名称
          <span class="text-red-500">*</span>
        </label>
        <InputText
          id="agent-name"
          v-model="currentAgent.name"
          placeholder="输入智能体名称"
          class="w-full" />
      </div>

      <div class="flex flex-col gap-2">
        <label
          for="agent-prompt"
          class="font-medium">
          系统提示词
        </label>
        <Textarea
          id="agent-prompt"
          v-model="currentAgent.system_prompt"
          placeholder="输入系统提示词..."
          rows="8"
          class="w-full"
          autoResize />
      </div>

      <div class="flex items-center gap-3">
        <ToggleSwitch
          v-model="currentAgent.allow_tools"
          inputId="allow-tools" />
        <label
          for="allow-tools"
          class="cursor-pointer">
          允许使用工具
        </label>
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <Button
          label="取消"
          severity="secondary"
          outlined
          @click="dialogVisible = false" />
        <Button
          :label="isEditing ? '保存' : '创建'"
          icon="pi pi-check"
          @click="saveAgent" />
      </div>
    </template>
  </Dialog>

  <ConfirmDialog />
</template>

<style scoped>
.line-clamp-2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
