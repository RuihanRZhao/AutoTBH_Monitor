<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const stash = ref<any>(null)
const loading = ref(true)
async function load(refresh = false) {
  loading.value = true
  try { stash.value = await get('/api/stash', { appid: 3678970, refresh: refresh ? 1 : undefined }) }
  catch { stash.value = null }
  loading.value = false
}
const cur = computed(() => stash.value?.currency || { symbol: '', decimals: 2 })
onMounted(() => load())
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.stash') }}</h2>
      <button class="btn" @click="load(true)">{{ t('common.refresh') }}</button>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!stash?.found" class="state">{{ t('common.gameNotFound') }}</div>
    <template v-else>
      <div class="cards" style="margin-bottom:16px">
        <div class="card"><div class="k">{{ t('common.total') }}</div><div class="v">{{ fmtMoney(stash.totalCents, cur.symbol, cur.decimals) }}</div></div>
        <div class="card"><div class="k">{{ t('stash.gear') }}</div><div class="v">{{ fmtMoney(stash.gearCents, cur.symbol, cur.decimals) }}</div></div>
        <div class="card"><div class="k">{{ t('stash.materials') }}</div><div class="v">{{ fmtMoney(stash.matCents, cur.symbol, cur.decimals) }}</div></div>
        <div class="card"><div class="k">{{ t('stash.pending') }} / {{ t('stash.unlisted') }}</div><div class="v" style="font-size:16px">{{ stash.pendingItems }} / {{ stash.unlistedItems }}</div></div>
      </div>

      <table>
        <thead><tr><th>{{ t('common.name') }}</th><th>{{ t('common.quantity') }}</th><th>{{ t('common.price') }}</th><th>{{ t('common.total') }}</th></tr></thead>
        <tbody>
          <tr v-for="it in stash.items" :key="it.hash">
            <td>
              <span :style="{ color: it.gradeColor || 'inherit' }">{{ it.wikiName || it.name }}</span>
              <span class="tag" style="margin-left:6px">{{ it.kind }}</span>
            </td>
            <td>{{ it.qty }}</td>
            <td>
              <template v-if="it.pricePending" class="muted">{{ t('stash.pending') }}</template>
              <template v-else-if="it.hasMarketListing === false" class="muted">{{ t('stash.unlisted') }}</template>
              <template v-else>{{ fmtMoney(it.priceCents, cur.symbol, cur.decimals) }}</template>
            </td>
            <td>{{ it.hasMarketListing === false || it.pricePending ? '—' : fmtMoney(it.priceCents * it.qty, cur.symbol, cur.decimals) }}</td>
          </tr>
        </tbody>
      </table>
    </template>
  </div>
</template>
