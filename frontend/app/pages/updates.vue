<script setup lang="ts">
const { t, locale } = useI18n()
const { get } = useApi()
const data = ref<any>(null)
const loading = ref(true)
function mapLang(l: string) { return ({ en: 'en-US', 'zh-Hans': 'zh-Hans', 'zh-Hant': 'zh-Hant', ja: 'ja-JP', ko: 'ko-KR' } as Record<string,string>)[l] || l }
async function load(refresh = false) {
  loading.value = true
  try { data.value = await get('/api/updates', { lang: mapLang(locale.value), refresh: refresh ? 1 : undefined }) } catch { data.value = null }
  loading.value = false
}
function fmtDate(ts: number) { return ts ? new Date(ts * 1000).toLocaleDateString() : '' }
onMounted(() => load())
</script>
<template>
  <div>
    <div class="page-head"><h2>{{ t('nav.updates') }}</h2><button class="btn" @click="load(true)">{{ t('common.refresh') }}</button></div>
    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!data" class="state">{{ t('common.offline') }}</div>
    <template v-else>
      <h3>{{ t('updates.patchnotes') }}</h3>
      <div class="grid-list" style="margin-bottom:20px">
        <a v-for="p in data.patchnotes" :key="p.build || p.title" :href="p.link" target="_blank" class="card">
          <strong>{{ p.title }}</strong>
          <div class="muted" style="font-size:12px">{{ fmtDate(p.date) }} · build {{ p.build }}</div>
        </a>
      </div>
      <h3>{{ t('updates.news') }}</h3>
      <div class="grid-list">
        <a v-for="n in data.news" :key="n.gid" :href="n.url" target="_blank" class="card">
          <img v-if="n.thumb" :src="n.thumb" style="width:100%; border-radius:8px; margin-bottom:8px" loading="lazy" />
          <strong>{{ n.title }}</strong>
          <div class="muted" style="font-size:12px">{{ n.feedlabel }} · {{ fmtDate(n.date) }}</div>
        </a>
      </div>
    </template>
  </div>
</template>
