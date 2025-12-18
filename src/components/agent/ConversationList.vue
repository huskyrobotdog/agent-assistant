<script setup>
import { ref, nextTick } from 'vue'

const props = defineProps({
  conversations: {
    type: Array,
    required: true,
  },
  currentId: {
    type: Number,
    default: null,
  },
})

const emit = defineEmits(['select', 'create', 'delete', 'rename'])

const menuRef = ref()
const targetId = ref(null)

// 重命名对话框
const renameDialogVisible = ref(false)
const renameTitle = ref('')

const menuItems = ref([
  {
    label: '编辑名称',
    icon: 'pi pi-pencil',
    command: () => openRenameDialog(),
  },
  {
    label: '删除',
    icon: 'pi pi-trash',
    class: 'text-red-500',
    command: () => emit('delete', targetId.value),
  },
])

function showMenu(event, id) {
  targetId.value = id
  menuRef.value.toggle(event)
}

function openRenameDialog() {
  const conv = props.conversations.find((c) => c.id === targetId.value)
  if (conv) {
    renameTitle.value = conv.title
    renameDialogVisible.value = true
  }
}

function confirmRename() {
  if (targetId.value && renameTitle.value.trim()) {
    emit('rename', { id: targetId.value, title: renameTitle.value.trim() })
  }
  renameDialogVisible.value = false
}
</script>

<template>
  <div class="conversation-list">
    <!-- 头部：新建对话 -->
    <div class="header">
      <Button
        label="新建对话"
        icon="pi pi-plus"
        class="w-full"
        @click="emit('create')" />
    </div>

    <!-- 对话列表 -->
    <div class="list-content">
      <div
        v-for="conv in conversations"
        :key="conv.id"
        class="conversation-item"
        :class="{ active: currentId === conv.id }"
        @click="emit('select', conv.id)">
        <div class="item-content">
          <span class="item-title">{{ conv.title }}</span>
          <Button
            icon="pi pi-ellipsis-v"
            text
            rounded
            severity="secondary"
            class="menu-btn"
            @click.stop="showMenu($event, conv.id)" />
        </div>
        <p class="item-preview">{{ conv.preview }}</p>
      </div>
    </div>

    <Menu
      ref="menuRef"
      :model="menuItems"
      :popup="true" />

    <!-- 重命名对话框 -->
    <Dialog
      v-model:visible="renameDialogVisible"
      header="编辑名称"
      :modal="true"
      :style="{ width: '25rem' }">
      <div class="flex flex-col gap-2">
        <InputText
          v-model="renameTitle"
          class="w-full"
          placeholder="请输入新名称"
          autofocus
          @keydown.enter="confirmRename" />
      </div>
      <template #footer>
        <Button
          label="取消"
          severity="secondary"
          text
          @click="renameDialogVisible = false" />
        <Button
          label="确定"
          @click="confirmRename" />
      </template>
    </Dialog>
  </div>
</template>

<style scoped>
.conversation-list {
  display: flex;
  flex-direction: column;
  height: 100%;
  background-color: var(--p-surface-0);
  border-radius: 0.5rem;
  border: 1px solid var(--p-surface-200);
}

.app-dark .conversation-list {
  background-color: var(--p-surface-900);
  border-color: var(--p-surface-700);
}

.header {
  padding: 0.75rem;
  border-bottom: 1px solid var(--p-surface-200);
}

.app-dark .header {
  border-color: var(--p-surface-700);
}

.list-content {
  flex: 1;
  overflow: auto;
}

.conversation-item {
  padding: 0.75rem;
  cursor: pointer;
  border-bottom: 1px solid var(--p-surface-100);
  transition: background-color 0.15s;
}

.app-dark .conversation-item {
  border-color: var(--p-surface-800);
}

.conversation-item:hover {
  background-color: var(--p-surface-100);
}

.app-dark .conversation-item:hover {
  background-color: var(--p-surface-800);
}

.conversation-item.active {
  background-color: color-mix(in srgb, var(--p-primary-color) 10%, transparent);
}

.item-content {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  margin-bottom: 0.25rem;
}

.item-title {
  flex: 1;
  font-weight: 500;
  font-size: 0.875rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.item-preview {
  font-size: 0.75rem;
  color: var(--p-surface-500);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.menu-btn {
  flex-shrink: 0;
  width: 1.75rem;
  height: 1.75rem;
  opacity: 0;
  transition: opacity 0.15s;
}

.conversation-item:hover .menu-btn {
  opacity: 1;
}
</style>
