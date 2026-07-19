<script setup lang="ts">
const { t } = useI18n()
const { get, base } = useApi()
const theme = ref('dark')
const version = ref<any>(null)
function setTheme(v: string) {
  theme.value = v
  document.documentElement.setAttribute('data-theme', v)
  localStorage.setItem('tbh_theme', v)
}
onMounted(async () => {
  theme.value = localStorage.getItem('tbh_theme') || 'dark'
  try { version.value = await get('/api/version') } catch {}
})
</script>
<template>
  <div>
    <div class="page-head"><h2>{{ t('settings.title') }}</h2></div>
    <div class="card" style="max-width:520px; margin-bottom:14px">
      <div class="k">{{ t('settings.theme') }}</div>
      <div style="display:flex; gap:8px; margin-top:8px">
        <button class="btn" :style="theme==='dark' ? 'border-color:var(--accent)' : ''" @click="setTheme('dark')">{{ t('settings.dark') }}</button>
        <button class="btn" :style="theme==='light' ? 'border-color:var(--accent)' : ''" @click="setTheme('light')">{{ t('settings.light') }}</button>
      </div>
    </div>
    <div class="card" style="max-width:520px">
      <div class="k">{{ t('settings.about') }}</div>
      <p class="muted" style="font-size:13px; line-height:1.7; margin:8px 0 0">
        AutoTBH_Monitor · v{{ version?.version || '1.22.4' }}<br />
        Backend: {{ base }}<br />
        Wiki data v{{ version?.wikiVersion || '—' }} — {{ version?.wikiItems || '—' }} items,
        {{ version?.wikiMonsters || '—' }} monsters, {{ version?.wikiRunes || '—' }} runes.<br />
        100% read-only. Not affiliated with Valve or the TBH developers.
      </p>
    </div>
  </div>
</template>
