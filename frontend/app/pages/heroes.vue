<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const ins = ref<any>(null)
const loading = ref(true)
const s = computed(() => ins.value?.insights || null)

async function load() {
  loading.value = true
  try { ins.value = await get('/api/insights') } catch { ins.value = null }
  loading.value = false
}
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
      <div class="cards" style="margin-bottom:18px">
        <div class="card">
          <div class="k">{{ t('heroes.unlocked') }}</div>
          <div class="v">{{ s.heroSummary.unlocked }}/{{ s.heroSummary.total }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('heroes.unspent') }}</div>
          <div class="v" :style="s.heroSummary.unspentAbilityPoints > 0 ? 'color:var(--warn)' : ''">
            {{ s.heroSummary.unspentAbilityPoints }}
          </div>
        </div>
      </div>

      <table>
        <thead>
          <tr>
            <th>{{ t('heroes.hero') }}</th><th>{{ t('heroes.level') }}</th><th>EXP</th>
            <th>{{ t('overview.gear') }}</th><th>{{ t('heroes.points') }}</th><th>{{ t('heroes.status') }}</th>
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
            <td class="muted">{{ expShort(h.exp) }}</td>
            <td>
              <span :style="h.equippedCount < 10 ? 'color:var(--warn)' : ''">{{ h.equippedCount }}/10</span>
            </td>
            <td>
              {{ h.allocatedAbilityPoint }}
              <span v-if="h.abilityPoint > 0" class="tag" style="margin-left:4px; border-color:var(--warn); color:var(--warn)">
                +{{ h.abilityPoint }}
              </span>
            </td>
            <td class="muted">{{ h.unlocked ? t('heroes.owned') : t('heroes.locked') }}</td>
          </tr>
        </tbody>
      </table>

      <p v-if="s.engine?.pending" class="muted" style="margin-top:14px; font-size:12px">
        {{ t('heroes.engineNote') }}
      </p>
    </template>
  </div>
</template>
