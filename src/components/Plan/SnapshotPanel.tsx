import { useEffect, useMemo, useState } from 'react'
import { Download, RotateCcw, X } from 'lucide-react'
import { useSnapshotStore } from '@/store/useSnapshotStore'
import type { SnapshotSummary } from '@/types/snapshot'

/**
 * 快照与回滚面板。
 *
 * 【为什么叫"快照与回滚"，而不是"修改历史"】
 * 快照是**状态**（某一时刻的全量原值），回滚是**动作**（把当前状态还原到
 * 某份快照）。历史是事件流，那是 changelog 的事；这里解决的是
 * 「我怎么回到改之前」。两者不要混。
 *
 * 【回滚的边界，界面上必须说清楚】
 * - 只恢复启停状态：实体被删/服务被卸载的项不会「复活」，会如实跳过并告知。
 * - 回滚本身也产生新快照，所以回滚错了还能再回滚——永远有退路。
 * - 回滚是**真实写操作**，确认按钮给足心理预期（会改系统）。
 */
export function SnapshotPanel({ onClose }: { onClose: () => void }) {
  const snapshots = useSnapshotStore((s) => s.snapshots)
  const busy = useSnapshotStore((s) => s.busy)
  const toast = useSnapshotStore((s) => s.toast)
  const error = useSnapshotStore((s) => s.error)
  const list = useSnapshotStore((s) => s.list)
  const rollback = useSnapshotStore((s) => s.rollback)
  const exportSnapshot = useSnapshotStore((s) => s.export)

  const [pending, setPending] = useState<SnapshotSummary | null>(null)

  useEffect(() => {
    void list()
  }, [list])

  /** 按 created_at 倒序（新在前），坏快照（无法读取）排最后 */
  const ordered = useMemo(
    () =>
      [...snapshots].sort((a, b) => {
        if (a.createdAt && b.createdAt) return b.createdAt.localeCompare(a.createdAt)
        return a.createdAt ? -1 : 1
      }),
    [snapshots],
  )

  const doRollback = async (target: SnapshotSummary) => {
    setPending(null)
    await rollback(target.id)
  }

  const doExport = async (target: SnapshotSummary) => {
    const bundle = await exportSnapshot(target.id)
    if (!bundle) return
    if (!bundle.ps1 && !bundle.reg) {
      return
    }
    // 交给系统默认下载（浏览器模式）或让用户选位置（Tauri 内由前端触发保存）
    const stamp = new Date().toISOString().slice(0, 10)
    const blob = new Blob([bundle.ps1], { type: 'text/plain;charset=utf-8' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `bootflow-rollback-${target.id.slice(0, 8)}-${stamp}.ps1`
    a.click()
    URL.revokeObjectURL(url)
  }

  return (
    <div className="absolute inset-x-0 bottom-full z-40 mb-2 max-h-[62vh] animate-slide-up overflow-y-auto rounded-lg border border-line bg-panel shadow-float scroll-thin">
      <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-line-subtle bg-panel/95 px-4 py-2 backdrop-blur">
        <span className="text-mini text-ink">快照与回滚</span>
        <span className="tnum text-mini text-ink-dim">{ordered.length} 份</span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => void list()}
          className="text-2xs text-ink-muted transition-colors hover:text-ink"
        >
          刷新
        </button>
        <button
          type="button"
          onClick={onClose}
          className="text-ink-dim transition-colors hover:text-ink"
          title="关闭"
        >
          <X size={13} />
        </button>
      </div>

      {toast && (
        <div className="flex items-center gap-2 border-b border-line-subtle px-4 py-2 text-2xs text-plan">
          <RotateCcw size={12} />
          {toast}
        </div>
      )}

      {error && (
        <div className="flex items-center gap-2 border-b border-line-subtle px-4 py-2 text-2xs text-danger">
          {error}
        </div>
      )}

      {busy === 'list' && ordered.length === 0 ? (
        <div className="px-4 py-3 text-mini text-ink-faint">正在读取快照…</div>
      ) : ordered.length === 0 ? (
        <div className="px-4 py-3 text-mini text-ink-faint">
          还没有任何快照。应用过一次修改后，这里就会出现可回滚的基线。
        </div>
      ) : (
        <ul className="divide-y divide-line-subtle">
          {ordered.map((s) => (
            <li key={s.id} className="flex items-start gap-2 px-4 py-2.5">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <ReasonBadge reason={s.reason} />
                  <span className="truncate text-mini text-ink">{s.description}</span>
                </div>
                <div className="mt-0.5 flex items-center gap-2 text-2xs text-ink-faint">
                  <span>{s.createdAt ? new Date(s.createdAt).toLocaleString('zh-CN') : '读取失败'}</span>
                  <span className="text-ink-dim">{s.recordCount} 项</span>
                  <span className="tnum text-ink-faint">{s.id.slice(0, 8)}</span>
                </div>
              </div>

              {pending?.id === s.id ? (
                <div className="flex items-center gap-2">
                  <span className="text-2xs text-ink-dim">确认回滚到这份快照？</span>
                  <button
                    type="button"
                    disabled={busy === 'rollback'}
                    onClick={() => void doRollback(s)}
                    className="rounded border border-line px-2 py-0.5 text-2xs font-medium text-ink transition-colors hover:border-danger hover:text-danger disabled:opacity-50"
                  >
                    {busy === 'rollback' ? '回滚中…' : '确认回滚'}
                  </button>
                  <button
                    type="button"
                    onClick={() => setPending(null)}
                    className="text-2xs text-ink-faint transition-colors hover:text-ink"
                  >
                    取消
                  </button>
                </div>
              ) : (
                <div className="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    onClick={() => void doExport(s)}
                    disabled={busy === 'export'}
                    className="flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-ink-muted transition-colors hover:bg-hover hover:text-ink disabled:opacity-40"
                    title="导出为独立恢复脚本（PowerShell + .reg）"
                  >
                    <Download size={12} />
                    导出
                  </button>
                  <button
                    type="button"
                    onClick={() => setPending(s)}
                    className="flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-ink-muted transition-colors hover:bg-hover hover:text-ink"
                    title="把系统状态还原到这份快照"
                  >
                    <RotateCcw size={12} />
                    回滚
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/** 快照原因的人话标签 */
function ReasonBadge({ reason }: { reason: string }) {
  const map: Record<string, { label: string; color: string }> = {
    扫描基线: { label: '扫描基线', color: '#58a6ff' },
    修改前: { label: '修改前', color: '#a371f7' },
    回滚: { label: '回滚', color: '#d29922' },
    导出: { label: '导出', color: '#6e7681' },
    未知: { label: '未知', color: '#6e7681' },
  }
  const meta = map[reason] ?? { label: reason, color: '#6e7681' }
  return (
    <span
      className="shrink-0 rounded px-1 text-2xs leading-4"
      style={{ color: meta.color, background: `${meta.color}1f` }}
    >
      {meta.label}
    </span>
  )
}