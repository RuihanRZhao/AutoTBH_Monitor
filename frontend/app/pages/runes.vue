<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()
const nodes = ref<any[]>([])
const loading = ref(true)
const q = ref('')
async function load() {
  loading.value = true
  try {
    const j = await get('/api/rune-tree')
    nodes.value = Array.isArray(j) ? j : (j.nodes || j.runes || [])
  } catch { nodes.value = [] }
  loading.value = false
}
const filtered = computed(() => {
  const s = q.value.trim().toLowerCase()
  return nodes.value.filter((n) => !s || JSON.stringify(n).toLowerCase().includes(s)).slice(0, 400)
})
onMounted(load)
</script>
<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.runes') }} <span class="muted">({{ nodes.length }})</span></h2>
      <input v-model="q" class="btn" :placeholder="t('common.search')" style="min-width:180px" />
    </div>
    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!nodes.length" class="state">{{ t('common.noData') }}</div>
    <div v-else class="grid-list">
      <div v-for="(n, i) in filtered" :key="n.key ?? n.id ?? i" class="card">
        <strong>{{ n.name || n.title || n.key || ('Node ' + (n.id ?? i)) }}</strong>
        <div class="muted" style="font-size:12px; margin-top:4px">
          {{ n.desc || n.description || n.stat || '' }}
        </div>
      </div>
    </div>
  </div>
</template>
