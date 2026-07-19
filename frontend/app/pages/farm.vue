<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()
const data = ref<any>(null)
const loading = ref(true)
async function load() {
  loading.value = true
  try { data.value = await get('/api/farm-calibration') } catch { data.value = null }
  loading.value = false
}
onMounted(load)
</script>
<template>
  <div>
    <div class="page-head"><h2>{{ t('nav.farm') }}</h2><button class="btn" @click="load">{{ t('common.refresh') }}</button></div>
    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!data?.stages?.length" class="state">{{ t('common.noData') }} — play a few runs with the meter to calibrate.</div>
    <table v-else>
      <thead><tr><th>Stage</th><th>Runs</th><th>Clear (s)</th><th>Gold/s</th><th>EXP/s</th><th>DPS</th></tr></thead>
      <tbody>
        <tr v-for="s in data.stages" :key="s.stageKey">
          <td>{{ s.stageKey }}</td><td>{{ s.n }}</td><td>{{ s.clearSec?.toFixed(1) }}</td>
          <td>{{ s.goldPerSec?.toFixed(0) ?? '—' }}</td><td>{{ s.expPerSec?.toFixed(0) ?? '—' }}</td>
          <td>{{ s.dps?.toFixed(0) }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
