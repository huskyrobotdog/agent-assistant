import { createRouter, createWebHistory } from 'vue-router'

const routes = [
    {
        path: '/',
        redirect: '/agent',
    },
    {
        path: '/agent',
        name: 'Agent',
        component: () => import('@/pages/AgentPage.vue'),
    },
    {
        path: '/settings',
        name: 'Settings',
        component: () => import('@/pages/SettingsPage.vue'),
    },
]

const router = createRouter({
    history: createWebHistory(),
    routes,
})

export default router
