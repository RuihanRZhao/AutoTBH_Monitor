<script setup lang="ts">
const { t } = useI18n()
const { get, post } = useApi()

const status = ref<any>(null)
const live = ref<any>(null)
const runs = ref<any[]>([])
let timer: any = null

async function refresh() {
  const [s, l, r] = await Promise.allSettled([get('/api/meter/status'), get('/api/meter'), get('/api/runs')])
  status.value = s.status === 'fulfilled' ? s.value : null
  live.value = l.status === 'fulfilled' ? l.value : null
  runs.value = r.status === 'fulfilled' ? (r.value.runs || []) : []
}
async function toggle() {
  const on = !status.value?.enabled
  try { status.value = await post('/api/meter/enable', { on: on ? 1 : 0 }) } catch {}
  await refresh()
}
function fmtNum(n: any) { return n == null ? '—' : Math.round(Number(n)).toLocaleString() }
function fmtTime(ts: number) { return ts ? new Date(ts).toLocaleTimeString() : '—' }

onMounted(() => { refresh(); timer = setInterval(refresh, 1000) })
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.meter') }}</h2>
      <button class="btn" @click="toggle">
        {{ status?.enabled ? t('meter.stop') : t('meter.start') }}
      </button>
    </div>

    <div class="card" style="margin-bottom:14px">
      <div class="k">{{ t('meter.state') }}</div>
      <div style="margin-top:6px">
        <span class="tag" :style="{ borderColor: status?.attached ? 'var(--good)' : 'var(--border)' }">
          {{ status?.attached ? t('meter.attached') : t('meter.detached') }}
        </span>
        <span class="tag" style="margin-left:6px">{{ t('meter.runs') }}: {{ status?.runCount ?? 0 }}</span>
      </div>
      <p v-if="status?.error" class="muted" style="font-size:12px; margin:8px 0 0">{{ status.error }}</p>
    </div>

    <div class="cards" style="margin-bottom:18px">
      <div class="card"><div class="k">DPS</div><div class="v">{{ fmtNum(live?.live?.dps) }}</div></div>
      <div class="card"><div class="k">{{ t('meter.damage') }}</div><div class="v">{{ fmtNum(live?.live?.total_damage) }}</div></div>
      <div class="card"><div class="k">{{ t('bestiary.gold') }}</div><div class="v">{{ fmtNum(live?.live?.gold) }}</div></div>
      <div class="card"><div class="k">{{ t('bestiary.exp') }}</div><div class="v">{{ fmtNum(live?.live?.xp) }}</div></div>
      <div class="card"><div class="k">{{ t('meter.kills') }}</div><div class="v">{{ fmtNum(live?.live?.kills) }}</div></div>
      <div class="card"><div class="k">{{ t('meter.stage') }}</div><div class="v" style="font-size:16px">{{ live?.live?.stage_key ?? '—' }}</div></div>
    </div>

    <h3>{{ t('meter.history') }}</h3>
    <div v-if="!runs.length" class="state">{{ t('common.noData') }}</div>
    <table v-else>
      <thead>
        <tr><th>{{ t('meter.time') }}</th><th>{{ t('meter.stage') }}</th><th>{{ t('meter.clear') }}</th><th>DPS</th><th>{{ t('bestiary.gold') }}</th><th>{{ t('bestiary.exp') }}</th></tr>
      </thead>
      <tbody>
        <tr v-for="(r, i) in runs.slice(0, 100)" :key="i">
          <td class="muted">{{ fmtTime(r.ts) }}</td>
          <td>{{ r.stageKey ?? '—' }}</td>
          <td>{{ r.clearTime ? r.clearTime.toFixed(1) + 's' : '—' }}</td>
          <td>{{ r.clearTime && r.totalDamage ? fmtNum(r.totalDamage / r.clearTime) : '—' }}</td>
          <td>{{ fmtNum(r.gold) }}</td>
          <td>{{ fmtNum(r.xp) }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
