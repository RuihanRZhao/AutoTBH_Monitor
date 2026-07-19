<script setup lang="ts">
const { t, locale } = useI18n()
const { get } = useApi()
const data = ref<any>(null)
const loading = ref(true)
function mapLang(l: string) { return ({ en: 'en-US', 'zh-Hans': 'zh-Hans' } as Record<string,string>)[l] || l }
async function load() {
  loading.value = true
  try { data.value = await get('/api/crafting', { lang: mapLang(locale.value) }) } catch { data.value = null }
  loading.value = false
}
onMounted(load)
</script>
<template>
  <div>
    <div class="page-head"><h2>{{ t('nav.crafting') }}</h2><button class="btn" @click="load">{{ t('common.refresh') }}</button></div>
    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!data?.craft?.length" class="state">{{ t('common.noData') }}</div>
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
  </div>
</template>
