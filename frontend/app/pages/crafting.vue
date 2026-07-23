<script setup lang="ts">
const { t, locale } = useI18n()
const { get } = useApi()
const data = ref<any>(null)
const plan = ref<any>(null)
const loading = ref(true)
function mapLang(l: string) { return ({ en: 'en-US', 'zh-Hans': 'zh-Hans' } as Record<string,string>)[l] || l }
async function load() {
  loading.value = true
  // Recipes (offline) and the player's own loose-item plan (save-derived) load independently.
  const [c, p] = await Promise.allSettled([
    get('/api/crafting', { lang: mapLang(locale.value) }),
    get('/api/crafting-plan'),
  ])
  data.value = c.status === 'fulfilled' ? c.value : null
  plan.value = p.status === 'fulfilled' ? p.value : null
  loading.value = false
}
onMounted(load)
function num(n: any) { return n == null ? '—' : Number(n).toLocaleString() }
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.crafting') }}</h2>
      <button class="btn" @click="load">{{ t('common.refresh') }}</button>
    </div>
    <div v-if="loading" class="state">{{ t('common.loading') }}</div>

    <template v-else>
      <!-- Your loose items: synthesis + alchemy (save-derived) -->
      <section v-if="plan?.ok" class="loose">
        <h3>{{ t('crafting.looseTitle') }}</h3>

        <div class="cards" style="margin-bottom:14px">
          <div class="card">
            <div class="k">{{ t('crafting.sellGold') }}</div>
            <div class="v">{{ num(plan.alchemy?.sellGold) }} ⦿</div>
            <div class="muted" style="font-size:11px">{{ plan.alchemy?.pricedItems }}/{{ plan.alchemy?.looseItems }} {{ t('crafting.priced') }}</div>
          </div>
          <div class="card">
            <div class="k">{{ t('crafting.cubeExp') }}</div>
            <div class="v">{{ num(plan.alchemy?.cubeExp) }}</div>
          </div>
          <div class="card">
            <div class="k">{{ t('crafting.cascadeTop') }}</div>
            <div class="v" style="font-size:16px">{{ plan.synthesis?.cascade?.topGradeReachable || '—' }}</div>
            <div class="muted" style="font-size:11px">{{ plan.synthesis?.cascade?.totalFuses }} {{ t('crafting.fuses') }}</div>
          </div>
        </div>

        <table v-if="plan.synthesis?.rows?.length">
          <thead>
            <tr>
              <th>{{ t('crafting.grade') }}</th><th>{{ t('common.quantity') }}</th>
              <th>{{ t('crafting.fusesNow') }}</th><th>{{ t('crafting.produces') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in plan.synthesis.rows" :key="r.grade">
              <td>{{ r.grade }}</td>
              <td>{{ r.have }}</td>
              <td>
                <span :style="r.fusesNow > 0 ? 'color:var(--good)' : ''">{{ r.fusesNow }}</span>
                <span v-if="r.leftover" class="muted" style="font-size:11px"> (+{{ r.leftover }})</span>
              </td>
              <td class="muted">{{ r.producesGrade || '—' }}</td>
            </tr>
          </tbody>
        </table>
        <p class="muted" style="font-size:11px; margin-top:6px">
          {{ t('crafting.synthRule') }} · {{ t('crafting.alchemyUnverified') }}
        </p>
      </section>

      <!-- Recipe reference (offline wiki data) -->
      <h3 style="margin-top:22px">{{ t('crafting.recipes') }}</h3>
      <div v-if="!data?.craft?.length" class="state">{{ t('common.noData') }}</div>
      <div v-else class="grid-list">
        <div v-for="r in data.craft" :key="r.key" class="card">
          <strong>{{ r.type }}</strong> <span class="tag">{{ t('crafting.tier') }} {{ r.tier }}</span>
          <ul style="margin:8px 0 0; padding-left:18px">
            <li v-for="m in r.materials" :key="m.id" :style="{ color: m.gradeColor || 'inherit' }">
              {{ m.name }} × {{ m.count }}
            </li>
          </ul>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
h3 { margin: 0 0 8px; }
.loose { margin-bottom: 8px; }
</style>
