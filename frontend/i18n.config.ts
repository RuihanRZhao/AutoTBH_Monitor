// vue-i18n runtime config. UI chrome strings for AutoTBH_Monitor.
// Game content (item/monster/stage names) is localized server-side via the wiki catalog's
// 16-language name maps; these strings cover the app shell / navigation / labels.
export default defineI18nConfig(() => ({
  legacy: false,
  fallbackLocale: 'en',
  messages: {
    en: {
      app: { title: 'AutoTBH_Monitor', subtitle: 'Read-only TBH: Task Bar Hero companion' },
      nav: {
        overview: 'Overview', stash: 'Sell Desk', market: 'Market', meter: 'Live Meter', farm: 'Farm',
        heroes: 'Heroes', runes: 'Runes', bestiary: 'Bestiary', crafting: 'Crafting',
        updates: 'Updates', settings: 'Settings',
      },
      meter: {
        start: 'Start meter', stop: 'Stop meter', state: 'Reader state',
        attached: 'Attached to game', detached: 'Game not running', runs: 'Runs',
        damage: 'Total damage', kills: 'Kills', stage: 'Stage', history: 'Run history',
        time: 'Time', clear: 'Clear time',
      },
      common: {
        loading: 'Loading…', refresh: 'Refresh', retry: 'Retry', offline: 'Offline',
        noData: 'No data', total: 'Total', quantity: 'Qty', price: 'Price', name: 'Name',
        currency: 'Currency', language: 'Language', search: 'Search',
        gameNotFound: 'Game save not found — install TBH and play once.',
      },
      overview: {
        bestMove: 'Best next move', stashValue: 'Stash value', items: 'Items', priced: 'Priced',
        gold: 'Gold', stage: 'Stage', playtime: 'Play time', party: 'Party', gear: 'Gear',
        points: 'Points', totalLevels: 'Total levels', pets: 'Pets', attributes: 'Attributes',
        nodes: 'nodes', storage: 'Stash slots', bag: 'Bag', lifetime: 'Lifetime stats',
        enginePending: 'Needs the simulation engine (not yet ported)',
      },
      heroes: {
        hero: 'Hero', level: 'Level', points: 'Ability points', status: 'Status',
        unlocked: 'Unlocked', unspent: 'Unspent points', owned: 'Owned', locked: 'Locked',
        partyDps: 'Party DPS', partyEhp: 'Party EHP', weakestLink: 'weakest link',
        carry: 'Carry', survivability: 'Survivability', armor: 'Armour', dodge: 'Dodge',
        armorMit: 'Armour mitigation',
        needGame: 'Combat numbers need the game running — the save carries no resolved stats.',
        ehpNote: 'EHP = HP / [(1 − dodge) × (1 − armour mitigation)], measured against your current stage level.',
      },
      market: { items: 'Market items', listings: 'listings', value: 'Value' },
      stash: { title: 'Your stash', gear: 'Gear', materials: 'Materials', pending: 'Pending', unlisted: 'Unlisted' },
      bestiary: { monsters: 'Monsters', stages: 'Stages', gold: 'Gold', exp: 'EXP', life: 'Life', atk: 'ATK' },
      crafting: { recipes: 'Recipes', materials: 'Materials', tier: 'Tier' },
      updates: { patchnotes: 'Patch notes', news: 'News' },
      settings: { title: 'Settings', theme: 'Theme', dark: 'Dark', light: 'Light', about: 'About' },
    },
    'zh-Hans': {
      app: { title: 'AutoTBH_Monitor', subtitle: '只读的 TBH: Task Bar Hero 助手' },
      nav: {
        overview: '总览', stash: '出售台', market: '市场', meter: '实时面板', farm: '刷图',
        heroes: '英雄', runes: '符文', bestiary: '图鉴', crafting: '合成',
        updates: '更新', settings: '设置',
      },
      meter: {
        start: '启动面板', stop: '停止面板', state: '读取器状态',
        attached: '已连接游戏', detached: '游戏未运行', runs: '记录数',
        damage: '总伤害', kills: '击杀', stage: '关卡', history: '战斗记录',
        time: '时间', clear: '通关耗时',
      },
      common: {
        loading: '加载中…', refresh: '刷新', retry: '重试', offline: '离线',
        noData: '暂无数据', total: '总计', quantity: '数量', price: '价格', name: '名称',
        currency: '货币', language: '语言', search: '搜索',
        gameNotFound: '未找到游戏存档 — 请安装 TBH 并至少游玩一次。',
      },
      overview: {
        bestMove: '下一步最佳操作', stashValue: '仓库价值', items: '物品', priced: '已定价',
        gold: '金币', stage: '当前关卡', playtime: '游戏时长', party: '出战队伍', gear: '装备',
        points: '天赋点', totalLevels: '总等级', pets: '宠物', attributes: '天赋',
        nodes: '项', storage: '仓库格', bag: '背包', lifetime: '生涯统计',
        enginePending: '以下能力需要模拟引擎（尚未移植）',
      },
      heroes: {
        hero: '英雄', level: '等级', points: '已加天赋点', status: '状态',
        unlocked: '已解锁', unspent: '未分配天赋点', owned: '已拥有', locked: '未解锁',
        partyDps: '队伍 DPS', partyEhp: '队伍 EHP', weakestLink: '取最短板',
        carry: '核心输出', survivability: '生存力', armor: '护甲', dodge: '闪避',
        armorMit: '护甲减伤',
        needGame: '战力数值需要游戏运行 —— 存档里没有解析后的属性。',
        ehpNote: 'EHP = HP / [(1 − 闪避) × (1 − 护甲减伤)]，以当前所在关卡等级为基准。',
      },
      market: { items: '市场物品', listings: '在售', value: '价值' },
      stash: { title: '你的仓库', gear: '装备', materials: '材料', pending: '待定价', unlisted: '无挂单' },
      bestiary: { monsters: '怪物', stages: '关卡', gold: '金币', exp: '经验', life: '生命', atk: '攻击' },
      crafting: { recipes: '配方', materials: '材料', tier: '阶级' },
      updates: { patchnotes: '更新日志', news: '新闻' },
      settings: { title: '设置', theme: '主题', dark: '深色', light: '浅色', about: '关于' },
    },
  },
}))
