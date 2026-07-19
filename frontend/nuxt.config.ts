// https://nuxt.com/docs/api/configuration/nuxt-config
// AutoTBH_Monitor frontend — Nuxt 4 SPA embedded in a Tauri window.
export default defineNuxtConfig({
  compatibilityDate: '2025-07-15',
  // Tauri hosts a static bundle; no Node server at runtime.
  ssr: false,
  devtools: { enabled: true },

  modules: ['@nuxtjs/i18n'],

  css: ['~/assets/css/main.css'],

  app: {
    head: {
      title: 'AutoTBH_Monitor',
      meta: [{ name: 'viewport', content: 'width=device-width, initial-scale=1' }],
    },
  },

  runtimeConfig: {
    public: {
      // The Rust/axum backend embedded by Tauri (or run standalone).
      apiBase: process.env.NUXT_PUBLIC_API_BASE || 'http://localhost:5260',
    },
  },

  i18n: {
    strategy: 'no_prefix',
    defaultLocale: 'en',
    vueI18n: './i18n.config.ts',
    locales: [
      { code: 'en', name: 'English' },
      { code: 'zh-Hans', name: '简体中文' },
      { code: 'zh-Hant', name: '繁體中文' },
      { code: 'ja', name: '日本語' },
      { code: 'ko', name: '한국어' },
      { code: 'id', name: 'Bahasa Indonesia' },
      { code: 'de', name: 'Deutsch' },
      { code: 'es', name: 'Español' },
      { code: 'fr', name: 'Français' },
      { code: 'it', name: 'Italiano' },
      { code: 'pt-BR', name: 'Português (BR)' },
      { code: 'ru', name: 'Русский' },
      { code: 'th', name: 'ไทย' },
      { code: 'tr', name: 'Türkçe' },
      { code: 'vi', name: 'Tiếng Việt' },
      { code: 'pl', name: 'Polski' },
    ],
  },

  // Tauri needs a fixed dev server + relative asset base for the bundled build.
  devServer: { host: '127.0.0.1', port: 3000 },

  vite: {
    clearScreen: false,
    envPrefix: ['VITE_', 'TAURI_'],
    server: { strictPort: true },
  },
})
