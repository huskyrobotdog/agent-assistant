<script setup>
import Tabs from 'primevue/tabs'
import TabList from 'primevue/tablist'
import Tab from 'primevue/tab'
import TabPanels from 'primevue/tabpanels'
import TabPanel from 'primevue/tabpanel'
import McpConfig from './McpConfig.vue'
import AgentConfig from './AgentConfig.vue'
</script>

<template>
  <div class="max-w-6xl mx-auto flex flex-col gap-6">
    <Tabs value="mcp">
      <TabList>
        <Tab value="general">通用设置</Tab>
        <Tab value="mcp">MCP</Tab>
        <Tab value="agent">智能体</Tab>
      </TabList>
      <TabPanels>
        <TabPanel value="general">
          <div class="p-4">
            <h3 class="text-lg font-semibold mb-4">通用设置</h3>
            <p class="text-surface-500">通用设置功能即将推出...</p>
          </div>
        </TabPanel>
        <TabPanel value="mcp">
          <McpConfig />
        </TabPanel>
        <TabPanel value="agent">
          <AgentConfig />
        </TabPanel>
      </TabPanels>
    </Tabs>
  </div>
</template>
