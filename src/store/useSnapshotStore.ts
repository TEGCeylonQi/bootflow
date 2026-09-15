import { create } from 'zustand'
import type { ApplyOutcome, SnapshotSummary } from '@/types/snapshot'
import { listSnapshots, rollbackTo, exportSnapshot } from '@/api/commands'

/**
 * 快照 / 回滚 / 导出 的操作层。
 *
 * 【为什么单独一个 store，而不是塞进 useAppStore】
 * 应用（apply）是一次性的、以当前草稿为中心的；而快照是**历史资产**，
 * 跨会话存活（存在 `%APPDATA%\BootFlow\snapshots\`）。两者生命周期不同，
 * 放一起会让 useAppStore 越来越重，也让「写操作」与「历史浏览」的
 * 状态纠缠不清。
 *
 * 【正在进行的操作用 field 表达，而不是一堆 boolean】
 * applying / loading / exporting 都是「同一时刻只会有一个写操作」的互斥
 * 语义，用字符串枚举比三个 bool 更不容易出现「同时在跑两个」的脏状态。
 */
export type SnapshotBusy = 'list' | 'rollback' | 'export' | null

interface SnapshotState {
  /** 快照列表（新→旧） */
  snapshots: SnapshotSummary[]
  /** 正在进行哪个操作（null = 空闲） */
  busy: SnapshotBusy
  /** 最近一次应用修改的返回值（含快照 id，便于引导回滚） */
  lastApply: ApplyOutcome | null
  /** 最近一次操作的结果提示（人话） */
  toast: string | null
  /** 最近一次操作的错误（人话，非 null 表示上一步失败） */
  error: string | null

  list: () => Promise<void>
  /** 应用成功后被调用：记录本次快照 id，供应用后回滚 */
  recordApply: (outcome: ApplyOutcome) => void
  /**
   * 回滚到某快照。会把最新快照列表刷新（回滚本身也产生新快照）。
   * 返回是否成功（false = 出错，error 已填充）。
   */
  rollback: (targetId: string) => Promise<boolean>
  /**
   * 导出快照为独立脚本。返回「文件内容 + 建议文件名」；是否写盘由调用方决定
   * （浏览器下载 + Tauri 对话框走两条路）。写盘失败也算失败。
   */
  export: (targetId: string) => Promise<{ ps1: string; reg: string } | null>
  clearToast: () => void
  clearError: () => void
}

export const useSnapshotStore = create<SnapshotState>()((set, get) => ({
  snapshots: [],
  busy: null,
  lastApply: null,
  toast: null,
  error: null,

  list: async () => {
    set({ busy: 'list', error: null })
    try {
      const snapshots = await listSnapshots()
      set({ snapshots, busy: null })
    } catch (e) {
      set({ busy: null, error: e instanceof Error ? e.message : String(e) })
    }
  },

  recordApply: (outcome) => set({ lastApply: outcome }),

  rollback: async (targetId) => {
    set({ busy: 'rollback', error: null, toast: null })
    try {
      const outcome = await rollbackTo(targetId)
      // 回滚成功：让用户看到「回滚也生成了新快照」，并保持列表最新
      await get().list()
      set({
        busy: null,
        toast:
          outcome.restored > 0
            ? `已回滚 ${outcome.restored} 项${outcome.skipped.length > 0 ? `，跳过 ${outcome.skipped.length} 项（未变化/不存在）` : ''}`
            : '无需回滚（当前状态与快照一致）',
      })
      return true
    } catch (e) {
      set({ busy: null, error: e instanceof Error ? e.message : String(e) })
      return false
    }
  },

  export: async (targetId) => {
    set({ busy: 'export', error: null })
    try {
      const bundle = await exportSnapshot(targetId)
      set({ busy: null })
      return bundle
    } catch (e) {
      set({ busy: null, error: e instanceof Error ? e.message : String(e) })
      return null
    }
  },

  clearToast: () => set({ toast: null }),
  clearError: () => set({ error: null }),
}))