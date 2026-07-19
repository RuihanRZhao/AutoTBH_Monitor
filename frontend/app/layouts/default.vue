<script setup lang="ts">
const { t, locale, locales, setLocale } = useI18n()
const { get } = useApi()

const nav = [
  { to: '/', key: 'overview', ico: '📊' },
  { to: '/stash', key: 'stash', ico: '💰' },
  { to: '/market', key: 'market', ico: '🏷️' },
  { to: '/meter', key: 'meter', ico: '📈' },
  { to: '/farm', key: 'farm', ico: '⚔️' },
  { to: '/heroes', key: 'heroes', ico: '🦸' },
  { to: '/runes', key: 'runes', ico: '🔮' },
  { to: '/bestiary', key: 'bestiary', ico: '📖' },
  { to: '/crafting', key: 'crafting', ico: '🔨' },
  { to: '/updates', key: 'updates', ico: '📰' },
  { to: '/settings', key: 'settings', ico: '⚙️' },
]

// Currency selector — backed by /api/currency.
const currencies = ref<any[]>([])
const curCode = ref<number | null>(null)
async function loadCurrency() {
  try {
    const j = await get('/api/currency')
    currencies.value = j.list || []
    curCode.value = j.code
  } catch { /* backend offline */ }
}
async function setCurrency(code: number) {
  try {
    const j = await get('/api/currency', { set: code })
    curCode.value = j.code
    useState('currency').value = j.info
    reloadNuxtApp
  } catch {}
}
onMounted(loadCurrency)

function applyTheme(v: string) {
  document.documentElement.setAttribute('data-theme', v)
  localStorage.setItem('tbh_theme', v)
}
onMounted(() => {
  const saved = localStorage.getItem('tbh_theme')
  if (saved) applyTheme(saved)
})
</script>

<template>
  <div class="app">
    <aside class="sidebar">
      <div class="brand">
        <h1>{{ t('app.title') }}</h1>
        <p>{{ t('app.subtitle') }}</p>
      </div>
      <NuxtLink v-for="n in nav" :key="n.to" :to="n.to" class="navlink">
        <span class="ico">{{ n.ico }}</span>{{ t('nav.' + n.key) }}
      </NuxtLink>
      <div class="spacer" />
      <div class="side-ctl">
        <label>{{ t('common.currency') }}</label>
        <select :value="curCode ?? ''" @change="setCurrency(Number(($event.target as HTMLSelectElement).value))">
          <option v-for="c in currencies" :key="c.code" :value="c.code">{{ c.iso }} — {{ c.symbol }}</option>
        </select>
      </div>
      <div class="side-ctl">
        <label>{{ t('common.language') }}</label>
        <select :value="locale" @change="setLocale(($event.target as HTMLSelectElement).value as any)">
          <option v-for="l in (locales as any[])" :key="l.code" :value="l.code">{{ l.name }}</option>
        </select>
      </div>
    </aside>
    <main class="main">
      <slot />
    </main>
  </div>
</template>
