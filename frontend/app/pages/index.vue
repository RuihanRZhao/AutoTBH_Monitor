<script setup lang="ts">
const { t } = useI18n()
const { get } = useApi()

const version = ref<any>(null)
const stash = ref<any>(null)
const ins = ref<any>(null)
const loading = ref(true)

const s = computed(() => ins.value?.insights || null)
const cur = computed(() => stash.value?.currency || { symbol: '', decimals: 2 })

async function load() {
  loading.value = true
  const [v, st, i] = await Promise.allSettled([
    get('/api/version'),
    get('/api/stash', { appid: 3678970 }),
    get('/api/insights'),
  ])
  version.value = v.status === 'fulfilled' ? v.value : null
  stash.value = st.status === 'fulfilled' ? st.value : null
  ins.value = i.status === 'fulfilled' ? i.value : null
  loading.value = false
}
function num(n: any) { return n == null ? '—' : Number(n).toLocaleString() }
onMounted(load)
</script>

<template>
  <div>
    <div class="page-head">
      <h2>{{ t('nav.overview') }}</h2>
      <button class="btn" @click="load">{{ t('common.refresh') }}</button>
    </div>

    <div v-if="loading" class="state">{{ t('common.loading') }}</div>
    <div v-else-if="!s" class="state">{{ t('common.gameNotFound') }}</div>
    <template v-else>
      <!-- next best move -->
      <div v-if="s.headline" class="card" style="margin-bottom:14px; border-left:3px solid var(--accent)">
        <div class="k">{{ t('overview.bestMove') }}</div>
        <div class="v" style="font-size:16px">{{ s.headline }}</div>
        <ul v-if="s.todo?.length > 1" style="margin:8px 0 0; padding-left:18px" class="muted">
          <li v-for="(td, idx) in s.todo.slice(1)" :key="idx" style="font-size:12px">{{ td.text }}</li>
        </ul>
      </div>

      <!-- headline numbers -->
      <div class="cards" style="margin-bottom:16px">
        <div class="card">
          <div class="k">{{ t('overview.stashValue') }}</div>
          <div class="v">{{ stash?.found ? fmtMoney(stash.totalCents, cur.symbol, cur.decimals) : '—' }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.gold') }}</div>
          <div class="v">{{ num(s.gold) }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.stage') }}</div>
          <div class="v">{{ s.progression.currentStageKey }}</div>
          <div class="muted" style="font-size:11px">max {{ s.progression.maxCompletedStage }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.playtime') }}</div>
          <div class="v">{{ s.progression.playTimeHours }}h</div>
        </div>
      </div>

      <!-- party -->
      <h3 style="margin:0 0 8px">{{ t('overview.party') }}</h3>
      <div class="cards" style="margin-bottom:16px">
        <div v-for="h in s.heroes.filter((x:any) => x.inParty)" :key="h.heroKey" class="card">
          <div class="k">#{{ h.partySlot + 1 }} · Hero {{ h.heroKey }}</div>
          <div class="v">Lv {{ h.level }}</div>
          <div class="muted" style="font-size:11px">
            {{ t('overview.gear') }} {{ h.equippedCount }}/10 · {{ t('overview.points') }} {{ h.allocatedAbilityPoint }}
          </div>
        </div>
      </div>

      <!-- collection progress -->
      <div class="cards" style="margin-bottom:16px">
        <div class="card">
          <div class="k">{{ t('nav.runes') }}</div>
          <div class="v">{{ s.runes.leveled }}/{{ s.runes.total }}</div>
          <div class="muted" style="font-size:11px">{{ t('overview.totalLevels') }} {{ s.runes.totalLevels }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.pets') }}</div>
          <div class="v">{{ s.pets.unlocked }}/{{ s.pets.total }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.attributes') }}</div>
          <div class="v">{{ s.attributes.totalLevels }}</div>
          <div class="muted" style="font-size:11px">{{ s.attributes.total }} {{ t('overview.nodes') }}</div>
        </div>
        <div class="card">
          <div class="k">{{ t('overview.storage') }}</div>
          <div class="v" style="font-size:16px">
            {{ s.storage.stash.used }}/{{ s.storage.stash.slots }}
          </div>
          <div class="muted" style="font-size:11px">
            {{ t('overview.bag') }} {{ s.storage.inventory.used }}/{{ s.storage.inventory.slots }}
          </div>
        </div>
      </div>

      <!-- lifetime -->
      <h3 style="margin:0 0 8px">{{ t('overview.lifetime') }}</h3>
      <div class="cards">
        <div v-for="(val, key) in s.lifetime" :key="key" class="card">
          <div class="k">{{ key }}</div>
          <div class="v" style="font-size:18px">{{ num(val) }}</div>
        </div>
      </div>

      <p v-if="s.engine?.pending" class="muted" style="margin-top:16px; font-size:12px">
        {{ t('overview.enginePending') }}: {{ s.engine.missing.join(', ') }}
      </p>
    </template>
  </div>
</template>
