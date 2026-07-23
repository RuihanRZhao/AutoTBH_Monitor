<script setup lang="ts">
const { t, locale } = useI18n()
const { get } = useApi()
const status = ref<any>(null)
const nodes = ref<any[]>([])
const loading = ref(true)
const q = ref('')

async function load() {
  loading.value = true
  const [st, tree] = await Promise.allSettled([get('/api/rune-status'), get('/api/rune-tree')])
  status.value = st.status === 'fulfilled' ? st.value : null
  const j = tree.status === 'fulfilled' ? tree.value : []
  nodes.value = Array.isArray(j) ? j : (j.nodes || j.runes || [])
  loading.value = false
}
const filtered = computed(() => {
  const s = q.value.trim().toLowerCase()
  return nodes.value.filter((n) => !s || JSON.stringify(n).toLowerCase().includes(s)).slice(0, 400)
})
function nm(o: any) { return o?.[locale.value] || o?.['en-US'] || o }
function num(n: any) { return n == null ? '—' : Number(n).toLocaleString() }
onMounted(load)
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.runes') }} <span class="muted">({{ nodes.length }})</span></h2>
      <button class="btn" @click="load">{{ t('common.refresh') }}</button>
    </div>
    <div v-if="loading" class="state">{{ t('common.loading') }}</div>

    <template v-else>
      <!-- Player's rune status (save + wiki) -->
      <template v-if="status?.ok">
        <div class="cards" style="margin-bottom:14px">
          <div class="card"><div class="k">{{ t('runes.owned') }}</div><div class="v">{{ status.ownedRunes }}/{{ status.total }}</div></div>
          <div class="card"><div class="k">{{ t('overview.totalLevels') }}</div><div class="v">{{ status.totalLevels }}</div></div>
          <div class="card"><div class="k">{{ t('runes.maxed') }}</div><div class="v">{{ status.maxed }}</div></div>
          <div class="card"><div class="k">{{ t('runes.affordable') }}</div><div class="v" :style="status.affordableCount ? 'color:var(--good)' : ''">{{ status.affordableCount }}</div></div>
        </div>

        <h3>{{ t('runes.upgrades') }}</h3>
        <table v-if="status.upgrades?.length">
          <thead>
            <tr>
              <th>{{ t('runes.rune') }}</th><th>{{ t('heroes.level') }}</th>
              <th>{{ t('runes.cost') }}</th><th>{{ t('runes.effect') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="u in status.upgrades.slice(0, 30)" :key="u.runeKey" :class="{ afford: u.affordable }">
              <td>
                {{ nm(u.name) }}
                <span v-if="u.combat" class="tag" style="border-color:var(--warn); color:var(--warn)">{{ t('runes.combat') }}</span>
                <span v-if="u.isNew" class="tag muted">{{ t('runes.new') }}</span>
              </td>
              <td>{{ u.level }} → {{ u.nextLevel }}<span class="muted">/{{ u.max }}</span></td>
              <td :style="u.affordable ? 'color:var(--good)' : ''">
                {{ num(u.cost) }} <span class="muted">{{ u.goldCost ? '⦿' : t('runes.mats') }}</span>
              </td>
              <td class="muted" style="font-size:12px">{{ u.stat }} +{{ u.value }}</td>
            </tr>
          </tbody>
        </table>
        <p class="muted" style="font-size:11px; margin-top:6px">{{ t('runes.noRoi') }}</p>
      </template>

      <!-- Searchable full rune tree (offline wiki) -->
      <div class="page-head" style="margin-top:22px">
        <h3>{{ t('runes.allRunes') }}</h3>
        <input v-model="q" class="btn" :placeholder="t('common.search')" style="min-width:180px" />
      </div>
      <div v-if="!nodes.length" class="state">{{ t('common.noData') }}</div>
      <div v-else class="grid-list">
        <div v-for="(n, i) in filtered" :key="n.key ?? n.id ?? i" class="card">
          <strong>{{ nm(n.name) || n.title || n.key || ('Node ' + (n.id ?? i)) }}</strong>
          <div class="muted" style="font-size:12px; margin-top:4px">
            {{ nm(n.desc) || n.description || n.stat || '' }}
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
h3 { margin: 0 0 8px; }
tr.afford td { background: color-mix(in srgb, var(--good) 8%, transparent); }
</style>
