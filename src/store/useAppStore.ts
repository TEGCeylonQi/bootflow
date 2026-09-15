import { create } from 'zustand'
import type { BootRecord, BootTimeline, ItemKind, OsInfo, RiskLevel, StartupItem } from '@/types/model'
import {
  getBootRecords,
  getBootTimeline,
  getIcons,
  scanAll,
  type IconRequest,
} from '@/api/commands'
import { isSevere, isProblem, resolveKind } from '@/lib/item'

/**
 * 开屏默认选中哪一项。
 *
 * 优先级：严重问题 → 一般问题 → 第一个**非系统组件** → 第一项。
 *
 * 最后一档刻意跳过系统组件：它们默认不出现在清单与画布上（见
 * `useFilteredItems`），选中一个用户看不见的项，会让右侧详情面板
 * 和左边清单对不上——用户会以为界面坏了，而不是以为自己在看别的东西。
 */
function pickInitialSelection(items: StartupItem[]): string | null {
  const flagged = items.find(isSevere) ?? items.find(isProblem)
  if (flagged) return flagged.id

  const visible = items.find((i) => resolveKind(i) !== 'system')
  return (visible ?? items[0])?.id ?? null
}

export type ViewMode = 'swimlane' | 'gantt'
export type ScanStatus = 'idle' | 'scanning' | 'ready' | 'error'

/**
 * 值得取图标的扩展名。
 *
 * `.dll` / `.ps1` 这类要么取不到有意义的图标，要么 Shell 会返回一个
 * 通用文档图标——那还不如让前端显示兜底色块，至少色块是按名称稳定推导的，
 * 用户扫一眼能靠颜色区分不同项。
 */
const ICON_EXTS = ['exe', 'com', 'bat', 'cmd', 'lnk', 'msc', 'cpl']

function iconExtOf(path: string): string {
  const dot = path.lastIndexOf('.')
  return dot < 0 ? '' : path.slice(dot + 1).toLowerCase()
}

/** 分栏宽度夹取。拖拽时分栏不能宽到把中间画布挤没，也不能窄到内容全被截断 */
function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, Math.round(v)))
}

interface AppState {
  // ——— 扫描结果 ———
  items: StartupItem[]
  os: OsInfo | null
  elevated: boolean
  scannedAt: string | null
  bootTimeline: BootTimeline | null
  /**
   * 「开机自记账」记录（旧 → 新）。
   *
   * 与 `bootTimeline` 是两条独立数据通路，两者**不是二选一**：
   * `bootTimeline` 来自系统事件日志（要提权、且只在慢启动时才有），
   * 这里来自 BootFlow 自己写的文件（普通权限、每次开机必有一条）。
   * 界面按「有分段用分段，没分段用总时长」的组合来展示。
   */
  bootRecords: BootRecord[]
  /** 各来源可容忍的部分失败 */
  errors: string[]
  status: ScanStatus
  errorMsg: string | null

  // ——— 界面状态 ———
  selectedId: string | null
  query: string
  /** 空数组代表「全选」。按类型筛选——「应用程序 / 后台服务」是用户能理解的维度 */
  kindFilter: ItemKind[]
  riskFilter: RiskLevel[]
  /** 只看需要处理的项（High + Medium） */
  onlyProblems: boolean
  /**
   * 是否把 Windows 自带组件一并显示。**默认关**。
   *
   * 不是筛选条件而是视图偏好，所以 `clearFilters` 不会重置它——
   * 用户点「清除筛选」的意思是"把我搜的、选的条件去掉"，
   * 不该顺带把 40 多个系统组件倒回眼前。
   */
  hideSystem: boolean
  view: ViewMode

  // ——— 多选（编排的前提：批量操作得先能批量选中）———
  /**
   * 已勾选的项。空数组代表「没有勾选任何项」。
   *
   * ⚠️ 注意与 `kindFilter` 的区别：那里的空数组代表"全选"（因为空筛选器
   * 天然就是不过滤），这里的空数组代表"什么都没选"。两个空数组含义相反，
   * 是这套代码里唯一容易读错的地方，所以两边都写了注释。
   */
  checkedIds: string[]
  /** Shift 范围选择的锚点：上一次点击的那一项 */
  anchorId: string | null

  // ——— 布局：分栏宽度（用户调过就记住）———
  sidebarWidth: number
  propertyWidth: number

  // ——— 动作 ———
  scan: () => Promise<void>
  /** 扫描完成后异步补图标。失败不影响任何东西，界面自行回退兜底图标。 */
  loadIcons: () => Promise<void>
  /**
   * 单独重读开机耗时时间轴（顶栏提权后 / 耗时分析页「重新读取」用）。
   * 扫描与诊断走同一数据源；权限提升后这里能立即看到真实数据，
   * 不必整窗重扫。
   */
  refreshTimeline: () => Promise<void>
  /**
   * 直接写入时间轴（诊断命令捎回 timeline 时用，避免二次读取）。
   *
   * 诊断 `diagnose_boot_performance` 内部已经读过事件日志并 build 好
   * timeline 一并返回，前端拿到后直接落地到 store——耗时分析页即刻
   * 反映与诊断同一个读取结果，不需要前端再调一次 `get_boot_timeline`。
   */
  setBootTimeline: (t: BootTimeline) => void
  /**
   * 重读「开机自记账」记录。
   *
   * 独立于 `refreshTimeline`：这条通路**不需要提权**，所以扫描时就该顺带拉一次，
   * 提权后也不必重读它。分开的理由是失败域不同——一个失败不该牵连另一个。
   */
  refreshBootRecords: () => Promise<void>
  select: (id: string | null) => void
  setQuery: (q: string) => void
  toggleKind: (k: ItemKind) => void
  toggleRisk: (r: RiskLevel) => void
  setOnlyProblems: (v: boolean) => void
  toggleSystem: () => void
  clearFilters: () => void
  setView: (v: ViewMode) => void

  toggleCheck: (id: string, visibleIds: string[], mods?: { range?: boolean; additive?: boolean }) => void
  setChecked: (ids: string[]) => void
  clearChecked: () => void
  setPaneWidth: (pane: 'sidebar' | 'property', width: number) => void
}

export const useAppStore = create<AppState>()((set, get) => ({
  items: [],
  os: null,
  elevated: false,
  scannedAt: null,
  bootTimeline: null,
  bootRecords: [],
  errors: [],
  status: 'idle',
  errorMsg: null,

  selectedId: null,
  query: '',
  kindFilter: [],
  riskFilter: [],
  onlyProblems: false,
  hideSystem: true,
  view: 'swimlane',
  checkedIds: [],
  anchorId: null,
  sidebarWidth: 300,
  propertyWidth: 340,

  scan: async () => {
    set({ status: 'scanning', errorMsg: null })
    try {
      // 自记账只是想读一个本地 JSON，和扫描并行即可。
      // 单独 catch 成空数组：读不到它不该把整次扫描拉进 error 态。
      const [result, records] = await Promise.all([
        scanAll(),
        getBootRecords().catch(() => [] as BootRecord[]),
      ])
      set({
        items: result.items,
        os: result.os,
        elevated: result.elevated,
        scannedAt: result.scannedAt,
        bootTimeline: result.bootTimeline,
        bootRecords: records,
        errors: result.errors,
        status: 'ready',
        // 首次扫描后自动定位到最严重的一项，让用户开屏就能看到诊断结论长什么样。
        // 找不到问题项时才退回第一项可显示的项。
        selectedId: pickInitialSelection(result.items),
        // 重扫后清空勾选：项 id 由路径派生，程序卸载/重装后会变，
        // 留着旧 id 会让批量操作指向一批已经不存在的项
        checkedIds: [],
        anchorId: null,
      })

      // 图标不进扫描关键路径：一次 Shell 调用虽然只有几十毫秒，
      // 但几十上百项累起来就是好几秒，用户会以为软件卡住了。
      // 先把列表渲染出来，图标后补。
      void get().loadIcons()
    } catch (e) {
      set({
        status: 'error',
        errorMsg: e instanceof Error ? e.message : String(e),
      })
    }
  },

  loadIcons: async () => {
    const requests: IconRequest[] = get()
      .items.filter((i) => i.resolvedPath && ICON_EXTS.includes(iconExtOf(i.resolvedPath)))
      .map((i) => ({ id: i.id, path: i.resolvedPath }))

    if (requests.length === 0) return

    try {
      const map = await getIcons(requests)
      if (Object.keys(map).length === 0) return

      set((st) => ({
        items: st.items.map((i) => (map[i.id] ? { ...i, iconData: map[i.id] } : i)),
      }))
    } catch (e) {
      // 图标是纯装饰，取不到不该让用户看到错误提示
      console.warn('[BootFlow] 图标提取失败，改用兜底图标', e)
    }
  },

  refreshTimeline: async () => {
    try {
      const timeline = await getBootTimeline()
      set({ bootTimeline: timeline })
    } catch (e) {
      // 单独刷新失败不应把整个界面拉进 error 态——保留旧值，仅上报
      console.warn('[BootFlow] 重读开机耗时失败，沿用旧值：', e)
    }
  },

  setBootTimeline: (timeline) => set({ bootTimeline: timeline }),

  refreshBootRecords: async () => {
    try {
      const records = await getBootRecords()
      set({ bootRecords: records })
    } catch (e) {
      // 与 refreshTimeline 同理：单独一条通路失败不该清空界面，保留旧值并上报
      console.warn('[BootFlow] 重读开机自记账失败，沿用旧值：', e)
    }
  },

  select: (id) => set({ selectedId: id, anchorId: id }),
  setQuery: (q) => set({ query: q }),

  toggleKind: (k) =>
    set((st) => ({
      kindFilter: st.kindFilter.includes(k)
        ? st.kindFilter.filter((x) => x !== k)
        : [...st.kindFilter, k],
    })),

  toggleRisk: (r) =>
    set((st) => ({
      riskFilter: st.riskFilter.includes(r)
        ? st.riskFilter.filter((x) => x !== r)
        : [...st.riskFilter, r],
    })),

  setOnlyProblems: (v) => set({ onlyProblems: v }),

  toggleSystem: () => set((st) => ({ hideSystem: !st.hideSystem })),

  clearFilters: () => set({ query: '', kindFilter: [], riskFilter: [], onlyProblems: false }),

  setView: (v) => set({ view: v }),

  /**
   * 勾选/取消勾选一项，支持 Shift 范围选与 Ctrl 累加选。
   *
   * `visibleIds` 由调用方传入当前**可见**的项顺序，而不是从 store 里读 items：
   * 范围选择必须落在用户眼前那一列上。如果拿全量 items 算区间，
   * 中间夹着的、被筛选隐藏的项会被静默勾上——用户随后执行批量操作，
   * 就会改到一些他从没看见过的东西。这是这类功能最容易出的严重事故。
   *
   * 因此批量操作方（ChangeDock / 批量条）也一律只处理
   * 「既被勾选、又在当前可见集合里」的项，两道保险。
   */
  toggleCheck: (id, visibleIds, mods) =>
    set((st) => {
      if (mods?.range && st.anchorId) {
        const a = visibleIds.indexOf(st.anchorId)
        const b = visibleIds.indexOf(id)
        if (a >= 0 && b >= 0) {
          const [lo, hi] = a < b ? [a, b] : [b, a]
          const acc = new Set(mods.additive ? st.checkedIds : [])
          for (const x of visibleIds.slice(lo, hi + 1)) acc.add(x)
          // 锚点保持不变，用户可以连续 Shift 微调区间边界
          return { checkedIds: [...acc] }
        }
      }

      if (mods?.additive) {
        return {
          checkedIds: st.checkedIds.includes(id)
            ? st.checkedIds.filter((x) => x !== id)
            : [...st.checkedIds, id],
          anchorId: id,
        }
      }

      // 无修饰键：只留这一项；若本来就只选中它，则整体取消（便于快速清空）
      const only = st.checkedIds.length === 1 && st.checkedIds[0] === id
      return { checkedIds: only ? [] : [id], anchorId: id }
    }),

  setChecked: (ids) => set({ checkedIds: ids, anchorId: ids[ids.length - 1] ?? null }),

  clearChecked: () => set({ checkedIds: [], anchorId: null }),

  setPaneWidth: (pane, width) =>
    set(pane === 'sidebar'
      ? { sidebarWidth: clamp(width, 220, 460) }
      : { propertyWidth: clamp(width, 260, 520) }),
}))
