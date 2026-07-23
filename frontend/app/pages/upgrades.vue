<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const data = ref<any>(null)
const loading = ref(true)
const err = ref<string | null>(null)

async function load() {
  loading.value = true
  err.value = null
  try {
    data.value = await get('/api/upgrades')
  } catch (e: any) {
    err.value = String(e?.message || e)
    data.value = null
  }
  loading.value = false
}

const recon = computed(() => data.value?.reconciliation || null)
// Stats the backend could not convert; listing them is the point — a silently
// shortened comparison is worse than a visibly partial one.
const unverified = computed(() => {
  const set = new Set<string>()
  for (const h of recon.value?.heroes || []) for (const s of h.unverifiedScale || []) set.add(s)
  return [...set]
})

function num(n: any, d = 0) {
  return n == null ? '—' : Number(n).toLocaleString(undefined, { maximumFractionDigits: d })
}
function signed(n: any, d = 0) {
  if (n == null) return '—'
  const v = Number(n)
  if (Math.abs(v) < 0.005) return '0'
  return (v > 0 ? '+' : '') + v.toLocaleString(undefined, { maximumFractionDigits: d })
}
function cls(n: any) {
  const v = Number(n)
  if (!isFinite(v) || Math.abs(v) < 0.005) return 'muted'
  return v > 0 ? 'up' : 'down'
}
onMounted(load)
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.upgrades') }}</h2>
      <button class="btn" @click="load">{{ t('common.refresh') }}</button>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="err" class="state">{{ err }}</div>
    <div v-else-if="data?.needsGame" class="state">{{ t('upgrades.needGame') }}</div>
    <div v-else-if="data?.found === false" class="state">{{ t('common.gameNotFound') }}</div>

    <!-- The backend refuses to emit deltas when our gear lines disagree with the game's own
         ITEM modifiers. Surface that as the headline rather than showing an empty table. -->
    <div v-else-if="data && !data.ok" class="state">
      <p style="color:var(--warn)"><strong>{{ t('upgrades.blocked') }}</strong></p>
      <p class="muted" style="font-size:12px">{{ data.reason || data.error }}</p>
      <table v-if="recon?.heroes?.length" style="margin-top:12px">
        <thead><tr><th>{{ t('heroes.hero') }}</th><th>{{ t('upgrades.stat') }}</th><th>{{ t('upgrades.ours') }}</th><th>{{ t('upgrades.game') }}</th></tr></thead>
        <tbody>
          <template v-for="h in recon.heroes" :key="h.heroKey">
            <tr v-for="(m, i) in h.mismatched" :key="h.heroKey + '-' + i">
              <td>Hero {{ h.heroKey }}</td>
              <td>{{ m.stat }}</td>
              <td>{{ m.buckets.map((b: any) => b.mode + ' ' + b.ours.toFixed(3)).join(', ') }}</td>
              <td>{{ m.buckets.map((b: any) => b.mode + ' ' + b.game.toFixed(3)).join(', ') }}</td>
            </tr>
          </template>
        </tbody>
      </table>
    </div>

    <template v-else-if="data?.heroes?.length">
      <p class="muted" style="font-size:12px; margin:-6px 0 14px">
        {{ t('upgrades.note', { stage: data.stageLevel }) }}
        <span v-if="recon" style="margin-left:8px">
          ✓ {{ t('upgrades.reconciled', { n: recon.heroes.reduce((a: number, h: any) => a + h.balanced.length, 0) }) }}
        </span>
      </p>

      <div v-for="h in data.heroes" :key="h.heroKey" class="hero-block">
        <h3>
          Hero {{ h.heroKey }}
          <span class="muted" style="font-size:13px; font-weight:400; margin-left:10px">
            DPS {{ num(h.dps, 1) }} · EHP {{ num(h.ehp) }} · POWER {{ num(h.power, 1) }}
          </span>
        </h3>

        <!-- This hero's gear didn't reconcile; show why, no swap deltas. -->
        <div v-if="h.blocked" class="state" style="padding:8px 0">
          <p class="muted" style="font-size:12px">
            ⚠ {{ t('upgrades.heroBlocked') }}:
            {{ (h.mismatched || []).map((m:any) => m.stat).join(', ') }}
          </p>
        </div>

        <table v-else>
          <thead>
            <tr>
              <th>{{ t('upgrades.slot') }}</th>
              <th>{{ t('upgrades.best') }}</th>
              <th style="text-align:right">ΔPOWER</th>
              <th style="text-align:right">ΔDPS</th>
              <th style="text-align:right">ΔEHP</th>
              <th style="text-align:right">{{ t('upgrades.candidates') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="s in h.slots" :key="s.slot">
              <td>
                <span class="tag">{{ s.gearType }}</span>
              </td>
              <td>
                <template v-if="s.best">
                  {{ s.best.name || s.best.itemKey }}
                  <span v-if="s.best.ignoredStats?.length" class="tag warn-tag"
                        :title="t('upgrades.ignoredHint') + ': ' + s.best.ignoredStats.join(', ')">
                    {{ t('upgrades.partial') }}
                  </span>
                </template>
                <span v-else class="muted">{{ t('upgrades.noCandidate') }}</span>
              </td>
              <td style="text-align:right" :class="cls(s.best?.dPower)">
                <strong>{{ signed(s.best?.dPower, 2) }}</strong>
              </td>
              <td style="text-align:right" :class="cls(s.best?.dDps)">{{ signed(s.best?.dDps, 2) }}</td>
              <td style="text-align:right" :class="cls(s.best?.dEhp)">{{ signed(s.best?.dEhp) }}</td>
              <td style="text-align:right" class="muted">{{ s.candidates?.length || 0 }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <p v-if="unverified.length" class="muted" style="font-size:12px; margin-top:14px">
        ⚠ {{ t('upgrades.unverified') }}: {{ unverified.join(', ') }}
      </p>
    </template>

    <div v-else class="state">{{ t('common.noData') }}</div>
  </div>
</template>

<style scoped>
.hero-block { margin-bottom: 24px; }
.hero-block h3 { margin: 0 0 8px; }
.up { color: var(--good); }
.down { color: var(--bad, #d66); }
.warn-tag { border-color: var(--warn); color: var(--warn); margin-left: 6px; }
</style>
