<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const rank = ref<any>(null)
const loading = ref(true)
const err = ref<string | null>(null)

async function load() {
  loading.value = true
  err.value = null
  try {
    rank.value = await get('/api/farm-rank')
  } catch (e: any) {
    err.value = String(e?.message || e)
    rank.value = null
  }
  loading.value = false
}
onMounted(load)

function num(n: any, d = 0) {
  return n == null ? '—' : Number(n).toLocaleString(undefined, { maximumFractionDigits: d })
}
function fmtSec(s: any) {
  if (s == null) return '—'
  const v = Number(s)
  if (v < 120) return v.toFixed(0) + 's'
  if (v < 3600) return (v / 60).toFixed(1) + 'm'
  return (v / 3600).toFixed(1) + 'h'
}
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.farm') }}</h2>
      <button class="btn" @click="load">{{ t('common.refresh') }}</button>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="err" class="state">{{ err }}</div>

    <template v-else-if="rank">
      <p v-if="rank.movementSpeedWarning" class="warn-box">⚠ {{ rank.movementSpeedWarning }}</p>
      <p v-else-if="rank.currentPartyMovementSpeed" class="muted" style="font-size:12px; margin:-6px 0 14px">
        {{ t('farm.msNote', { ms: rank.currentPartyMovementSpeed.toFixed(2), dev: rank.movementSpeedDeviationPct.toFixed(1) }) }}
      </p>

      <h3>{{ t('farm.measured') }}</h3>
      <p class="muted" style="font-size:12px; margin:-4px 0 10px">{{ t('farm.measuredNote') }}</p>
      <table v-if="rank.measured.length">
        <thead>
          <tr>
            <th>{{ t('farm.stage') }}</th><th>{{ t('farm.samples') }}</th><th>{{ t('farm.clear') }}</th>
            <th>{{ t('farm.goldHr') }}</th><th>{{ t('farm.expHr') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="r in rank.measured" :key="r.stageKey">
            <td>{{ r.label }} <span class="muted">#{{ r.stageKey }}</span></td>
            <td>{{ r.n }}</td>
            <td>{{ fmtSec(r.clearSec) }}</td>
            <td>
              {{ num(r.goldPerHour) }}
              <span v-if="r.tableGoldDisagreesWithMeasured" class="tag warn-tag" title="Measured gold/hr disagrees with the table's expectedGold by more than 25%">⚠</span>
            </td>
            <td>{{ num(r.expPerHour) }}</td>
          </tr>
        </tbody>
      </table>
      <p v-else class="state" style="padding:12px 0">{{ t('farm.noMeasured') }}</p>

      <h3 style="margin-top:24px">{{ t('farm.modelled') }}</h3>
      <p class="muted" style="font-size:12px; margin:-4px 0 10px">{{ rank.modelledCaveat }}</p>
      <table v-if="rank.modelled.length">
        <thead>
          <tr><th>{{ t('farm.stage') }}</th><th>{{ t('farm.clear') }}</th><th>{{ t('farm.goldHr') }}</th><th>{{ t('farm.expHr') }}</th></tr>
        </thead>
        <tbody>
          <tr v-for="r in rank.modelled.slice(0, 25)" :key="r.stageKey">
            <td>{{ r.label }} <span class="muted">#{{ r.stageKey }}</span></td>
            <td>{{ fmtSec(r.clearSec) }}</td>
            <td>{{ num(r.goldPerHour) }}</td>
            <td>{{ num(r.expPerHour) }}</td>
          </tr>
        </tbody>
      </table>
      <p class="muted" style="font-size:11px; margin-top:6px" v-if="rank.modelled.length > 25">
        {{ t('farm.modelledTruncated', { n: rank.modelled.length }) }}
      </p>
    </template>

    <div v-else class="state">{{ t('common.noData') }}</div>
  </div>
</template>

<style scoped>
h3 { margin: 0 0 4px; }
.warn-box {
  border: 1px solid var(--warn); color: var(--warn); border-radius: 6px;
  padding: 8px 12px; font-size: 12px; margin: 0 0 14px;
}
.warn-tag { border-color: var(--warn); color: var(--warn); margin-left: 4px; }
</style>
