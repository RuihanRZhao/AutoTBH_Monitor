<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const version = ref<any>(null)
const stash = ref<any>(null)
const insights = ref<any>(null)
const cur = computed(() => stash.value?.currency || useState('currency').value || { symbol: '', decimals: 2 })
const loading = ref(true)

async function load() {
  loading.value = true
  const [v, s, i] = await Promise.allSettled([
    get('/api/version'),
    get('/api/stash', { appid: 3678970 }),
    get('/api/insights'),
  ])
  version.value = v.status === 'fulfilled' ? v.value : null
  stash.value = s.status === 'fulfilled' ? s.value : null
  insights.value = i.status === 'fulfilled' ? i.value : null
  loading.value = false
}
onMounted(load)
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.overview') }}</h2>
      <button class="btn" @click="load">{{ t('common.refresh') }}</button>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <template v-else>
      <div v-if="insights?.insights?.bestMove || insights?.insights?.headline" class="card" style="margin-bottom:14px">
        <div class="k">{{ t('overview.bestMove') }}</div>
        <div class="v" style="font-size:16px">
          {{ insights.insights.headline || insights.insights.bestMove?.label || '—' }}
        </div>
      </div>

      <div class="cards">
        <div class="card">
          <div class="k">{{ t('overview.stashValue') }}</div>
          <div class="v">{{ stash?.found ? fmtMoney(stash.totalCents, cur.symbol, cur.decimals) : '—' }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.items') }}</div>
          <div class="v">{{ stash?.totalItems ?? '—' }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.priced') }}</div>
          <div class="v">{{ stash?.pricedItems ?? '—' }}</div>
        </div>
        <div class="card">
          <div class="k">Bestiary / Runes / Stages</div>
          <div class="v" style="font-size:16px">
            {{ version?.wikiMonsters ?? '—' }} · {{ version?.wikiRunes ?? '—' }} · {{ version?.wikiStages ?? '—' }}
          </div>
        </div>
      </div>

      <p v-if="!stash?.found" class="muted" style="margin-top:16px">{{ t('common.gameNotFound') }}</p>
    </template>
  </div>
</template>
