import { createRouter, createWebHistory } from 'vue-router'

const routes = [
    {
        path: '/',
        redirect: '/agent',
    },
    {
        path: '/agent',
        name: 'Agent',
        component: () => import('@/pages/agent/Index.vue'),
    },
    {
        path: '/settings',
        name: 'Settings',
        component: () => import('@/pages/settings/Index.vue'),
    },
]

const router = createRouter({
    history: createWebHistory(),
    routes,
})

export default router
