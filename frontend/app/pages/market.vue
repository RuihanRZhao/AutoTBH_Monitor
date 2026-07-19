<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const data = ref<any>(null)
const loading = ref(true)
const q = ref('')

async function load(refresh = false) {
  loading.value = true
  try { data.value = await get('/api/items', { appid: 3678970, refresh: refresh ? 1 : undefined }) }
  catch { data.value = null }
  loading.value = false
}
const cur = computed(() => data.value?.currency || { symbol: '', decimals: 2 })
const items = computed(() => {
  const list = data.value?.items || []
  const s = q.value.trim().toLowerCase()
  return (s ? list.filter((i: any) => (i.wikiName || i.name || '').toLowerCase().includes(s)) : list).slice(0, 300)
})
onMounted(() => load())
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('market.items') }}</h2>
      <div style="display:flex; gap:8px">
        <input v-model="q" class="btn" :placeholder="t('common.search')" style="min-width:180px" />
        <button class="btn" @click="load(true)">{{ t('common.refresh') }}</button>
      </div>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!data" class="state">{{ t('common.offline') }}</div>
    <table v-else>
      <thead>
        <tr><th></th><th>{{ t('common.name') }}</th><th>{{ t('market.listings') }}</th><th>{{ t('common.price') }}</th></tr>
      </thead>
      <tbody>
        <tr v-for="it in items" :key="it.hash">
          <td><img v-if="it.wikiIcon || it.icon" :src="it.wikiIcon || it.icon" class="thumb" loading="lazy" /></td>
          <td>
            <span :style="{ color: it.gradeColor || 'inherit' }">{{ it.wikiName || it.name }}</span>
            <div class="muted" style="font-size:11px">{{ it.type }}</div>
          </td>
          <td class="muted">{{ it.listings ?? '—' }}</td>
          <td>{{ fmtMoney(it.priceCents, cur.symbol, cur.decimals) }}</td>
        </tr>
      </tbody>
    </table>
    <p v-if="data?.stale" class="muted" style="margin-top:10px">cache · {{ new Date(data.fetchedAt).toLocaleString() }}</p>
  </div>
</template>
