import { createApp } from "vue";
import PrimeVue from 'primevue/config';
import Aura from '@primeuix/themes/aura';
import Ripple from 'primevue/ripple';
import Tooltip from 'primevue/tooltip';
import 'primeicons/primeicons.css';
import '@/style.css';
import App from '@/App.vue';
import router from '@/router.js';
import { invoke } from '@tauri-apps/api/core';
import { appDataDir, join } from '@tauri-apps/api/path';

const app = createApp(App);

app.use(router);

app.use(PrimeVue, {
    ripple: true,
    theme: {
        preset: Aura,
        options: {
            prefix: 'p',
            darkModeSelector: '.app-dark',
            cssLayer: {
                name: 'primevue',
                order: 'theme, base, primevue'
            }
        }
    }
});

app.directive('ripple', Ripple);
app.directive('tooltip', Tooltip);

const loadApp = () => {
    const logo = document.querySelector('.loading-logo');
    if (logo) {
        logo.classList.add('fade-out');
        setTimeout(() => app.mount("#app"), 800);
    } else {
        app.mount("#app");
    }
};


const startApp = async () => {
    const appData = await appDataDir();
    // const modelPath = await join(appData, 'models', 'Qwen3-4B-Thinking-2507-UD-IQ1_M.gguf');
    // const modelPath = await join(appData, 'models', 'Qwen3-1.7B-Q4_K_M.gguf');
    // const modelPath = await join(appData, 'models', 'Qwen3-0.6B-Q4_K_M.gguf');
    const modelPath = await join(appData, 'models', 'Qwen3-4B-Q4_K_M.gguf');
    // const modelPath = await join(appData, 'models', 'Phi-4-mini-reasoning-Q8_0.gguf');
    // const modelPath = await join(appData, 'models', 'Qwen3-1.7B-Q4_0.gguf');
    // const modelPath = await join(appData, 'models', 'Qwen3-0.6B-Q8_0.gguf');
    console.log(modelPath)
    await invoke('init_agent', { modelPath });
    loadApp()
}

startApp()
