<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()
const insights = ref<any>(null)
const loading = ref(true)
async function load() {
  loading.value = true
  try { insights.value = await get('/api/insights') } catch { insights.value = null }
  loading.value = false
}
const heroes = computed(() => insights.value?.insights?.heroes || insights.value?.insights?.party || [])
onMounted(load)
</script>
<template>
  <div>
    <div class="page-head"><h2>{{ t('nav.heroes') }}</h2><button class="btn" @click="load">{{ t('common.refresh') }}</button></div>
    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!heroes.length" class="state">{{ t('common.gameNotFound') }}</div>
    <div v-else class="grid-list">
      <div v-for="(h, i) in heroes" :key="h.key ?? i" class="card">
        <div style="display:flex; gap:10px; align-items:center">
          <img v-if="h.portrait || h.icon" :src="h.portrait || h.icon" class="thumb" />
          <div><strong>{{ h.name || h.key }}</strong><div class="muted" style="font-size:11px">Lv {{ h.level ?? '—' }}</div></div>
        </div>
        <div class="muted" style="font-size:12px; margin-top:8px">
          DPS {{ h.dps ?? '—' }} · POWER {{ h.power ?? '—' }}
        </div>
      </div>
    </div>
  </div>
</template>
