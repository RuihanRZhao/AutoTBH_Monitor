<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const rank = ref<any>(null)
const idle = ref<any>(null)
const loading = ref(true)
const err = ref<string | null>(null)

async function load() {
  loading.value = true
  err.value = null
  try {
    // Idle is a secondary panel — don't let its failure blank the whole page.
    const [r, i] = await Promise.allSettled([get('/api/farm-rank'), get('/api/idle')])
    if (r.status === 'fulfilled') rank.value = r.value
    else { err.value = String(r.reason?.message || r.reason); rank.value = null }
    idle.value = i.status === 'fulfilled' ? i.value : null
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
      <div v-if="rank.stayVsSwitch" class="stay-box" :class="rank.stayVsSwitch.verdict">
        <template v-if="rank.stayVsSwitch.verdict === 'stay'">
          ✓ {{ t('farm.stay', { stage: rank.stayVsSwitch.current?.label }) }}
        </template>
        <template v-else-if="rank.stayVsSwitch.verdict === 'switch'">
          → {{ t('farm.switch', { from: rank.stayVsSwitch.current?.label, to: rank.stayVsSwitch.bestMeasured?.label }) }}
        </template>
        <template v-else>{{ t('farm.unmeasuredCurrent') }}</template>
        <span v-if="rank.stayVsSwitch.exploreHint" class="muted" style="display:block; font-size:11px; margin-top:4px">
          {{ t('farm.exploreHint', { stage: rank.stayVsSwitch.exploreHint.label }) }}
        </span>
      </div>

      <p v-if="rank.movementSpeedWarning" class="warn-box">⚠ {{ rank.movementSpeedWarning }}</p>
      <p v-else-if="rank.currentPartyMovementSpeed" class="muted" style="font-size:12px; margin:-6px 0 14px">
        {{ t('farm.msNote', { ms: rank.currentPartyMovementSpeed.toFixed(2), dev: rank.movementSpeedDeviationPct.toFixed(1) }) }}
      </p>

      <!-- Idle / offline accrual -->
      <div v-if="idle?.idle?.unlocked" class="idle-box">
        <div class="idle-head">
          <strong>{{ t('farm.idleTitle') }}</strong>
          <span class="muted" style="font-size:11px">{{ t('farm.idleUnverified') }}</span>
        </div>
        <div class="idle-grid">
          <div><span class="k">{{ t('farm.accrued') }}</span>
            <span class="v">{{ num(idle.idle.accruedGold) }} <span class="muted">/ {{ num(idle.idle.fullGold) }}</span> ⦿</span></div>
          <div><span class="k">EXP</span>
            <span class="v">{{ num(idle.idle.accruedExp) }} <span class="muted">/ {{ num(idle.idle.fullExp) }}</span></span></div>
          <div><span class="k">{{ t('farm.toCap') }}</span>
            <span class="v">{{ fmtSec(idle.idle.secsToCap) }}</span></div>
          <div v-if="idle.forecast?.gold100kSec != null"><span class="k">{{ t('farm.gold100k') }}</span>
            <span class="v">{{ fmtSec(idle.forecast.gold100kSec) }}</span></div>
        </div>
        <div class="idle-bar"><div class="idle-fill" :style="{ width: Math.round((idle.idle.frac || 0) * 100) + '%' }"></div></div>
      </div>
      <p v-else-if="idle && idle.idle && !idle.idle.unlocked" class="muted" style="font-size:12px; margin:0 0 14px">
        {{ t('farm.idleLocked') }}
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
            <td>{{ num(r.goldPerHour) }}</td>
            <td>
              {{ num(r.expPerHour) }}
              <span v-if="r.expFromTable" class="tag" title="Estimated from the base reward table — the meter has no measured XP for this stage yet">est</span>
            </td>
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
.stay-box {
  border: 1px solid var(--good); border-radius: 6px; padding: 8px 12px;
  font-size: 13px; margin: 0 0 14px;
}
.stay-box.switch { border-color: var(--warn); }
.stay-box.unmeasured { border-color: var(--muted, #888); color: var(--muted, #888); }
.idle-box {
  border: 1px solid var(--border, #333); border-radius: 6px; padding: 10px 12px; margin: 0 0 16px;
}
.idle-head { display: flex; justify-content: space-between; align-items: baseline; margin-bottom: 8px; }
.idle-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 8px 16px; }
.idle-grid .k { display: block; font-size: 11px; color: var(--muted, #888); }
.idle-grid .v { font-size: 15px; font-weight: 600; }
.idle-bar { height: 4px; background: var(--border, #333); border-radius: 2px; margin-top: 10px; overflow: hidden; }
.idle-fill { height: 100%; background: var(--good); }
</style>
