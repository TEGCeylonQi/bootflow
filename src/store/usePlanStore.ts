import { create } from 'zustand'
import type { AppMode, ChangeKind, PlanEntry, PlanPatch } from '@/types/plan'
import { CHANGE_LABEL, describeChange, snapshotOf } from '@/types/plan'
import type { StartupItem } from '@/types/model'

/**
 * 编排草稿。
 *
 * 【核心约束：这里没有任何一条会改动系统】
 * 草稿层只负责把"用户想做什么"记录下来，它不写注册表、不调服务、不动计划任务。
 * 真正让改动生效是 v1.5 的写操作层。这样分层的好处是：
 * 编辑过程**天然无害且完全可逆**——撤销就是丢掉草稿，不需要回滚任何真实状态，
 * 也就不存在"改坏了回不去"的风险。
 */

/** 撤销栈的最大深度。够覆盖一次完整编排的所有动作，又不至于吃内存 */
const UNDO_LIMIT = 100

type EntryMap = Record<string, PlanEntry>

interface PlanState {
  mode: AppMode
  /** 待应用的变更，按 itemId 索引——同一项只保留最新一条 */
  entries: EntryMap
  /** 变更篮的展示顺序（itemId 数组），按记录时间升序 */
  order: string[]

  undoStack: EntryMap[]
  redoStack: EntryMap[]

  /** 最近一次生成方案的时间与条数，用于给用户"事情办完了"的确认 */
  exportedAt: string | null
  exportedCount: number

  setMode: (m: AppMode) => void
  toggleMode: () => void

  /** 记录一条变更。同一项已有变更时覆盖，不会堆出两条互相矛盾的记录 */
  stage: (item: StartupItem, kind: ChangeKind, next: PlanPatch) => void
  /**
   * 一次记录多条变更，作为**一个**撤销单元。
   *
   * 拖拽重排会同时改动一整组的顺序。如果逐条 stage，每次都会压一次撤销栈，
   * 用户按一次 Ctrl+Z 只退回去一格，得连按好几次才能把一次拖拽撤销干净——
   * 而在他眼里那是"一个动作"。所以批量写入必须共用一份快照。
   */
  stageMany: (changes: { item: StartupItem; kind: ChangeKind; next: PlanPatch }[]) => void
  /** 撤销某一项的变更 */
  unstage: (itemId: string) => void
  clear: () => void

  undo: () => void
  redo: () => void

  /** 产出编排方案文档（可保存、可复制的纯文本） */
  exportPlan: (items: StartupItem[]) => string
  /** 生成方案后清空草稿并记录结果 */
  commitExport: (count: number) => void

  has: (itemId: string) => boolean
  entryOf: (itemId: string) => PlanEntry | undefined
  /** 变更条数 */
  count: () => number
}

function pushUndo(state: PlanState): Pick<PlanState, 'undoStack' | 'redoStack'> {
  const undoStack = [...state.undoStack, state.entries]
  return {
    undoStack: undoStack.length > UNDO_LIMIT ? undoStack.slice(-UNDO_LIMIT) : undoStack,
    // 产生新动作后旧的重做路径失效——这是撤销/重做的标准语义
    redoStack: [],
  }
}

export const usePlanStore = create<PlanState>()((set, get) => ({
  mode: 'inspect',
  entries: {},
  order: [],
  undoStack: [],
  redoStack: [],
  exportedAt: null,
  exportedCount: 0,

  setMode: (m) => set({ mode: m }),
  toggleMode: () => set((s) => ({ mode: s.mode === 'inspect' ? 'orchestrate' : 'inspect' })),

  stage: (item, kind, next) =>
    set((s) => {
      const before = snapshotOf(item)
      const entry: PlanEntry = { itemId: item.id, kind, next, before, at: Date.now() }
      return {
        ...pushUndo(s),
        entries: { ...s.entries, [item.id]: entry },
        order: s.order.includes(item.id) ? s.order : [...s.order, item.id],
      }
    }),

  unstage: (itemId) =>
    set((s) => {
      if (!s.entries[itemId]) return s
      const entries = { ...s.entries }
      delete entries[itemId]
      return {
        ...pushUndo(s),
        entries,
        order: s.order.filter((id) => id !== itemId),
      }
    }),

  stageMany: (changes) =>
    set((s) => {
      if (changes.length === 0) return s
      const at = Date.now()
      const entries = { ...s.entries }
      const order = [...s.order]
      changes.forEach((c, i) => {
        entries[c.item.id] = {
          itemId: c.item.id,
          kind: c.kind,
          next: c.next,
          before: snapshotOf(c.item),
          // 同一批内用递增时间戳，保证它们在变更篮里保持传入顺序
          at: at + i,
        }
        if (!order.includes(c.item.id)) order.push(c.item.id)
      })
      return { ...pushUndo(s), entries, order }
    }),

  clear: () =>
    set((s) =>
      s.order.length === 0 ? s : { ...pushUndo(s), entries: {}, order: [] },
    ),

  undo: () =>
    set((s) => {
      if (s.undoStack.length === 0) return s
      const prev = s.undoStack[s.undoStack.length - 1]
      return {
        entries: prev,
        order: s.order.filter((id) => id in prev),
        undoStack: s.undoStack.slice(0, -1),
        redoStack: [...s.redoStack, s.entries],
      }
    }),

  redo: () =>
    set((s) => {
      if (s.redoStack.length === 0) return s
      const next = s.redoStack[s.redoStack.length - 1]
      // 重做时把新增回来的项追加到顺序末尾，保证它重新出现在变更篮里
      const added = Object.keys(next).filter((id) => !s.order.includes(id))
      return {
        entries: next,
        order: [...s.order, ...added],
        redoStack: s.redoStack.slice(0, -1),
        undoStack: [...s.undoStack, s.entries],
      }
    }),

  exportPlan: (items) => buildPlanDocument(get(), items),

  commitExport: (count) =>
    set({
      entries: {},
      order: [],
      undoStack: [],
      redoStack: [],
      exportedAt: new Date().toISOString(),
      exportedCount: count,
    }),

  has: (itemId) => !!get().entries[itemId],
  entryOf: (itemId) => get().entries[itemId],
  count: () => get().order.length,
}))

/* ─────────────────────────────────────────────────────────────
   方案文档：把草稿变成一个用户能带走的东西
   ───────────────────────────────────────────────────────────── */

/** 方案文档里的单条记录 */
interface PlanDocumentEntry {
  id: string
  name: string
  kind: ChangeKind
  kindLabel: string
  /** 人话描述 */
  summary: string
  /** 变更前 → 变更后 */
  before: Record<string, unknown>
  after: Record<string, unknown>
  /** 启动方式与注册位置，便于用户自己核对 */
  source: string
  location: string
  command: string
}

interface PlanDocument {
  schema: 'bootflow.plan/v1'
  generatedAt: string
  /** 生成时的操作系统信息，便于日后比对 */
  itemCount: number
  changes: PlanDocumentEntry[]
}

/**
 * 产出编排方案。
 *
 * 同时给两份：JSON 给机器（将来直接喂给执行层），Markdown 给人看。
 * 只给 JSON 的话，用户保存下来是一堆看不懂的东西；
 * 只给 Markdown 的话，将来无法可靠地被程序读回。
 */
function buildPlanDocument(state: PlanState, items: StartupItem[]): string {
  const byId = new Map(items.map((i) => [i.id, i]))
  const changes: PlanDocumentEntry[] = []

  for (const id of state.order) {
    const entry = state.entries[id]
    const item = byId.get(id)
    if (!entry || !item) continue

    const name = item.displayName?.trim() || item.name
    changes.push({
      id,
      name,
      kind: entry.kind,
      kindLabel: CHANGE_LABEL[entry.kind],
      summary: describeChange(entry, name),
      before: entry.before as unknown as Record<string, unknown>,
      after: entry.next as unknown as Record<string, unknown>,
      source: item.source,
      location: item.location,
      command: item.command,
    })
  }

  const doc: PlanDocument = {
    schema: 'bootflow.plan/v1',
    generatedAt: new Date().toISOString(),
    itemCount: changes.length,
    changes,
  }

  const md = [
    `# BootFlow 编排方案`,
    ``,
    `生成时间：${new Date().toLocaleString('zh-CN')}`,
    `共计 ${changes.length} 项变更。`,
    ``,
    `> 本方案由 BootFlow 生成，记录的是**意图**而非已生效的改动。`,
    `> 执行能力将在 v1.5 开放，届时本文件可直接导入并逐步应用（每步可撤销）。`,
    ``,
    `---`,
    ``,
    ...changes.map((c, i) => {
      const before = JSON.stringify(c.before)
      const after = JSON.stringify(c.after)
      return [
        `### ${i + 1}. ${c.summary}`,
        ``,
        `- 动作：${c.kindLabel}`,
        `- 变更前：\`${before}\``,
        `- 变更后：\`${after}\``,
        `- 启动方式：${c.source}`,
        `- 注册位置：\`${c.location}\``,
        `- 命令行：\`${c.command}\``,
        ``,
      ].join('\n')
    }),
    `---`,
    ``,
    `<!-- 以下为机器可读部分，勿手工编辑 -->`,
    ``,
    '```json',
    JSON.stringify(doc, null, 2),
    '```',
  ].join('\n')

  return md
}
