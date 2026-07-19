<script setup lang="ts">
const { t, locale } = useI18n()
const { get } = useApi()

const data = ref<any>(null)
const tab = ref<'monsters' | 'stages'>('monsters')
const loading = ref(true)
const q = ref('')

async function load() {
  loading.value = true
  try { data.value = await get('/api/codex', { kind: 'all', lang: mapLang(locale.value) }) }
  catch { data.value = null }
  loading.value = false
}
function mapLang(l: string) {
  const m: Record<string, string> = { en: 'en-US', 'zh-Hans': 'zh-Hans', 'zh-Hant': 'zh-Hant', ja: 'ja-JP', ko: 'ko-KR', id: 'id-ID', de: 'de-DE', es: 'es-ES', fr: 'fr-FR', it: 'it-IT', 'pt-BR': 'pt-BR', ru: 'ru-RU', th: 'th-TH', tr: 'tr-TR', vi: 'vi-VN', pl: 'pl-PL' }
  return m[l] || 'en-US'
}
const monsters = computed(() => filterBy(data.value?.monsters))
const stages = computed(() => filterBy(data.value?.stages))
function filterBy(list: any[] | undefined) {
  const s = q.value.trim().toLowerCase()
  return (list || []).filter((x) => !s || (x.name || '').toLowerCase().includes(s))
}
onMounted(load)
watch(locale, load)
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.bestiary') }}</h2>
      <input v-model="q" class="btn" :placeholder="t('common.search')" style="min-width:180px" />
    </div>
    <div style="display:flex; gap:8px; margin-bottom:14px">
      <button class="btn" :style="tab==='monsters' ? 'border-color:var(--accent)' : ''" @click="tab='monsters'">{{ t('bestiary.monsters') }}</button>
      <button class="btn" :style="tab==='stages' ? 'border-color:var(--accent)' : ''" @click="tab='stages'">{{ t('bestiary.stages') }}</button>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <template v-else-if="tab==='monsters'">
      <div class="grid-list">
        <div v-for="m in monsters" :key="m.key" class="card">
          <div style="display:flex; gap:10px; align-items:center">
            <img v-if="m.portrait" :src="m.portrait" class="thumb" loading="lazy" />
            <div>
              <strong>{{ m.name }}</strong>
              <div class="muted" style="font-size:11px">{{ m.type }}</div>
            </div>
          </div>
          <div class="muted" style="font-size:12px; margin-top:8px">
            {{ t('bestiary.life') }} {{ m.life }} · {{ t('bestiary.atk') }} {{ m.atk }} ·
            {{ t('bestiary.gold') }} {{ m.gold }} · {{ t('bestiary.exp') }} {{ m.exp }}
          </div>
        </div>
      </div>
    </template>
    <template v-else>
      <table>
        <thead><tr><th>#</th><th>{{ t('common.name') }}</th><th>Lv</th><th>Waves</th><th>{{ t('bestiary.gold') }}</th><th>{{ t('bestiary.exp') }}</th></tr></thead>
        <tbody>
          <tr v-for="s in stages" :key="s.key">
            <td class="muted">{{ s.act }}-{{ s.no }}</td>
            <td>{{ s.name }}<span v-if="s.boss" class="tag" style="margin-left:6px">BOSS</span></td>
            <td>{{ s.level }}</td><td>{{ s.waves }}</td>
            <td>{{ s.goldPerClear }}</td><td>{{ s.expPerClear }}</td>
          </tr>
        </tbody>
      </table>
    </template>
  </div>
</template>
