<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const ins = ref<any>(null)
const loading = ref(true)
const s = computed(() => ins.value?.insights || null)
const meta = computed(() => s.value?.meta || null)
const liveOff = computed(() => meta.value?.combatSource && meta.value.combatSource !== 'live game memory')

async function load() {
  loading.value = true
  try { ins.value = await get('/api/insights') } catch { ins.value = null }
  loading.value = false
}
function num(n: any, d = 0) { return n == null ? '—' : Number(n).toLocaleString(undefined, { maximumFractionDigits: d }) }
function expShort(e: number) {
  if (e == null) return '—'
  if (e >= 1e9) return (e / 1e9).toFixed(2) + 'B'
  if (e >= 1e6) return (e / 1e6).toFixed(2) + 'M'
  if (e >= 1e3) return (e / 1e3).toFixed(1) + 'K'
  return Math.round(e).toString()
}
onMounted(load)
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.heroes') }}</h2>
      <button class="btn" @click="load">{{ t('common.refresh') }}</button>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!s?.heroes?.length" class="state">{{ t('common.gameNotFound') }}</div>
    <template v-else>
      <!-- party combat summary -->
      <div class="cards" style="margin-bottom:18px">
        <div class="card">
          <div class="k">{{ t('heroes.partyDps') }}</div>
          <div class="v">{{ num(meta?.partyDPS, 1) }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('heroes.partyEhp') }}</div>
          <div class="v">{{ num(meta?.partyEHP) }}</div>
          <div class="muted" style="font-size:11px">{{ t('heroes.weakestLink') }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('heroes.carry') }}</div>
          <div class="v" style="font-size:18px">
            {{ meta?.carryHero ?? '—' }}
            <span v-if="meta?.carryShare" class="muted" style="font-size:13px">
              {{ (meta.carryShare * 100).toFixed(0) }}%
            </span>
          </div>
        </div>
        <div class="card">
          <div class="k">{{ t('heroes.unspent') }}</div>
          <div class="v" :style="s.heroSummary.unspentAbilityPoints > 0 ? 'color:var(--warn)' : ''">
            {{ s.heroSummary.unspentAbilityPoints }}
          </div>
        </div>
      </div>

      <p v-if="liveOff" class="muted" style="font-size:12px; margin:-8px 0 12px">
        ⚠ {{ t('heroes.needGame') }}
      </p>

      <table>
        <thead>
          <tr>
            <th>{{ t('heroes.hero') }}</th><th>{{ t('heroes.level') }}</th>
            <th>DPS</th><th>EHP</th><th>POWER</th>
            <th>{{ t('overview.gear') }}</th><th>{{ t('heroes.points') }}</th><th>EXP</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="h in s.heroes" :key="h.heroKey">
            <td>
              <strong>Hero {{ h.heroKey }}</strong>
              <span v-if="h.inParty" class="tag" style="margin-left:6px; border-color:var(--good); color:var(--good)">
                #{{ h.partySlot + 1 }}
              </span>
            </td>
            <td>{{ h.level }}</td>
            <td>{{ num(h.dps, 1) }}</td>
            <td>{{ num(h.ehp) }}</td>
            <td :style="h.power ? 'font-weight:600' : ''">{{ num(h.power) }}</td>
            <td><span :style="h.equippedCount < 10 ? 'color:var(--warn)' : ''">{{ h.equippedCount }}/10</span></td>
            <td>
              {{ h.allocatedAbilityPoint }}
              <span v-if="h.abilityPoint > 0" class="tag" style="margin-left:4px; border-color:var(--warn); color:var(--warn)">
                +{{ h.abilityPoint }}
              </span>
            </td>
            <td class="muted">{{ expShort(h.exp) }}</td>
          </tr>
        </tbody>
      </table>

      <!-- mitigation breakdown for fielded heroes -->
      <h3 style="margin:20px 0 8px">{{ t('heroes.survivability') }}</h3>
      <table>
        <thead>
          <tr><th>{{ t('heroes.hero') }}</th><th>HP</th><th>{{ t('heroes.armor') }}</th><th>{{ t('heroes.dodge') }}</th><th>{{ t('heroes.armorMit') }}</th><th>EHP</th></tr>
        </thead>
        <tbody>
          <tr v-for="h in s.heroes.filter((x:any) => x.ehp != null)" :key="h.heroKey">
            <td>Hero {{ h.heroKey }}</td>
            <td>{{ num(h.maxHp) }}</td>
            <td>{{ num(h.armor) }}</td>
            <td>{{ h.dodgePercent?.toFixed(1) }}%</td>
            <td>{{ (h.armorMitigation * 100).toFixed(1) }}%</td>
            <td>{{ num(h.ehp) }}</td>
          </tr>
        </tbody>
      </table>
      <p class="muted" style="font-size:12px; margin-top:8px">
        {{ t('heroes.ehpNote') }}<span v-if="s.heroes.find((x:any) => x.stageLevel)"> (stage lv {{ s.heroes.find((x:any) => x.stageLevel).stageLevel }})</span>
      </p>

      <p v-if="s.engine?.pending" class="muted" style="margin-top:10px; font-size:12px">
        {{ t('overview.enginePending') }}: {{ s.engine.missing.join(', ') }}
      </p>
    </template>
  </div>
</template>
