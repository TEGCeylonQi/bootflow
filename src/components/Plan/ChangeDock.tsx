import { useMemo, useState } from 'react'
import {
  ChevronDown,
  ChevronUp,
  CircleAlert,
  Copy,
  Download,
  Layers,
  Play,
  Redo2,
  Trash2,
  Undo2,
  X,
} from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import { useSnapshotStore } from '@/store/useSnapshotStore'
import { CHANGE_LABEL, describeChange, diffRowsOf } from '@/types/plan'
import type { DryRunOutcome, EditInput } from '@/types/snapshot'
import { dryRunEdits, applyEdits } from '@/api/commands'
import { AppIcon } from '@/components/common/Icon'
import { SnapshotPanel } from '@/components/Plan/SnapshotPanel'

const PLAN_COLOR = '#a371f7'

/**
 * 变更篮 —— 编排意图的落脚点。
 *
 * 【设计取舍：为什么是一个"篮子"，而不是直接改】
 * 用户对启动项的判断往往是分批形成的：先看到几个明显该关的，再慢慢翻出一些
 * 可留可不留的。如果每点一下就立即写入系统，他会一直处在"我是不是又改坏了一个"
 * 的不安里，而且没法把一批改动**作为一个整体**来审视。
 *
 * 篮子把「表达意图」和「让意图生效」拆开：编辑期间零风险、随时整体放弃。
 * 本版本与过去最大的不同：**底下真的会改系统了**，但前面隔着一道预演闸门——
 * 点「应用」先看到「将要发生什么」，确认后才执行；执行会建改前快照，
 * 改完随时可以从快照回滚。每一步都可逆。
 */
export function ChangeDock() {
  const mode = usePlanStore((s) => s.mode)
  const order = usePlanStore((s) => s.order)
  const entries = usePlanStore((s) => s.entries)
  const undo = usePlanStore((s) => s.undo)
  const redo = usePlanStore((s) => s.redo)
  const unstage = usePlanStore((s) => s.unstage)
  const clear = usePlanStore((s) => s.clear)
  const undoStack = usePlanStore((s) => s.undoStack)
  const redoStack = usePlanStore((s) => s.redoStack)
  const items = useAppStore((s) => s.items)
  const setChecked = useAppStore((s) => s.setChecked)
  const scan = useAppStore((s) => s.scan)
  const recordApply = useSnapshotStore((s) => s.recordApply)
  const busy = useSnapshotStore((s) => s.busy)

  const [open, setOpen] = useState(false)
  const [flash, setFlash] = useState<string | null>(null)
  const [preview, setPreview] = useState<DryRunOutcome | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [applying, setApplying] = useState(false)
  const [showSnapshots, setShowSnapshots] = useState(false)

  const byId = useMemo(() => new Map(items.map((i) => [i.id, i])), [items])

  const rows = useMemo(
    () =>
      order.flatMap((id) => {
        const entry = entries[id]
        const item = byId.get(id)
        if (!entry || !item) return []
        const name = item.displayName?.trim() || item.name
        return [{ entry, item, name, summary: describeChange(entry, name), diff: diffRowsOf(entry) }]
      }),
    [order, entries, byId],
  )

  const say = (msg: string) => {
    setFlash(msg)
    window.setTimeout(() => setFlash((cur) => (cur === msg ? null : cur)), 3200)
  }

  /** 草稿 → 后端 EditInput 列表（一条只带真正要改的字段） */
  function editsOf(): EditInput[] {
    const out: EditInput[] = []
    for (const id of order) {
      const entry = entries[id]
      const item = byId.get(id)
      if (!entry || !item) continue
      const edit: EditInput = { itemId: id }
      if (entry.next.enabled !== undefined) edit.enabled = entry.next.enabled
      // (delay / priority / order 本版本后端不接受，前端不产生对应字段)
      out.push(edit)
    }
    return out
  }

  const buildText = () => usePlanStore.getState().exportPlan(items)

  const copyPlan = async () => {
    const text = buildText()
    try {
      await navigator.clipboard.writeText(text)
      say(`已复制 ${rows.length} 项变更，可直接粘贴保存`)
    } catch {
      // 剪贴板权限在部分环境下会被拒；降级成文件保存，不让用户白点一下
      downloadPlan()
      say('剪贴板不可用，已改为保存文件')
    }
  }

  const downloadPlan = () => {
    const text = buildText()
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')
    const blob = new Blob([text], { type: 'text/markdown;charset=utf-8' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `bootflow-plan-${stamp}.md`
    a.click()
    URL.revokeObjectURL(url)
    say(`已保存方案文件（${rows.length} 项）`)
  }

  /** 应用前先预演：把「将要发生什么」摆到用户面前 */
  const runPreview = async () => {
    const edits = editsOf()
    if (edits.length === 0) return
    setPreviewError(null)
    try {
      const outcome = await dryRunEdits(items, edits)
      setPreview(outcome)
    } catch (e) {
      setPreviewError(e instanceof Error ? e.message : String(e))
    }
  }

  /** 预演确认后真正应用。成功后：重扫让清单反映真实状态，清空草稿。 */
  const runApply = async () => {
    if (!preview) return
    setApplying(true)
    try {
      const outcome = await applyEdits(items, editsOf())
      recordApply(outcome)
      setPreview(null)
      setOpen(false)
      clear()
      setChecked([])
      say(`已应用（快照 ${outcome.snapshotId.slice(0, 8)}，可在回滚区找回）`)
      // 写操作已真实发生，重扫刷新现状
      void scan()
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      setPreviewError(msg)
    } finally {
      setApplying(false)
    }
  }

  // 非编排模式且没有草稿时，这一条完全不该出现——它不属于体检视图
  if (mode === 'inspect' && rows.length === 0) return null

  const hasChanges = rows.length > 0

  return (
    <div className="relative shrink-0">
      {/* ——— 预演确认弹层 ——— */}
      {preview && (
        <div className="absolute inset-x-0 bottom-full z-30 mb-2 max-h-[60vh] overflow-y-auto rounded-lg border border-line bg-panel shadow-float scroll-thin">
          <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-line-subtle bg-panel/95 px-4 py-2 backdrop-blur">
            <span className="text-mini text-ink">确认这次修改</span>
            <span className="tnum text-mini text-ink-dim">{preview.steps.length} 项</span>
            <div className="flex-1" />
            <span className="text-2xs text-ink-faint">应用前自动建立改前快照，可随时回滚</span>
            <button
              type="button"
              onClick={() => setPreview(null)}
              className="text-ink-dim transition-colors hover:text-ink"
              title="关闭"
            >
              <X size={13} />
            </button>
          </div>

          <ul className="divide-y divide-line-subtle">
            {preview.steps.map((a) => (
              <li key={`${a.itemId}-${a.field}`} className="flex items-start gap-2.5 px-4 py-2">
                <AppIcon name={a.displayName} size={20} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-mini text-ink">{a.displayName}</span>
                    <SafeBadge risk={a.risk} />
                  </div>
                  <p className="mt-0.5 text-2xs leading-4 text-ink-muted">
                    <span className="line-through text-ink-faint">{a.before}</span>
                    <span className="mx-1 text-ink-faint">→</span>
                    <span style={{ color: PLAN_COLOR }}>{a.after}</span>
                    <span className="ml-2">{a.consequence}</span>
                  </p>
                </div>
              </li>
            ))}
            {preview.denied.length > 0 && (
              <li className="flex items-start gap-2 px-4 py-2">
                <CircleAlert size={13} className="mt-1 shrink-0 text-amber-400" />
                <div className="min-w-0">
                  <p className="text-2xs text-ink-dim">以下项被护栏拒绝，不会执行：</p>
                  {preview.denied.map((d) => (
                    <p key={d} className="mt-0.5 text-2xs leading-4 text-ink-faint">{d}</p>
                  ))}
                </div>
              </li>
            )}
            {preview.noop && (
              <li className="px-4 py-3 text-mini text-ink-faint">
                没有任何实际变更（目标状态与现状一致）。
              </li>
            )}
          </ul>

          {previewError && (
            <div className="border-t border-line-subtle px-4 py-2 text-2xs text-danger">
              {previewError}
            </div>
          )}

          <div className="sticky bottom-0 flex items-center gap-2 border-t border-line-subtle bg-panel/95 px-4 py-2 backdrop-blur">
            <span className="text-2xs text-ink-faint">
              写操作会立即生效；失败自动回滚，不留「改了一半」。
            </span>
            <div className="flex-1" />
            <button
              type="button"
              onClick={() => setPreview(null)}
              className="rounded border border-line px-3 py-1 text-mini text-ink-muted transition-colors hover:text-ink"
            >
              取消
            </button>
            <button
              type="button"
              disabled={applying || preview.steps.length === 0}
              onClick={() => void runApply()}
              className="flex items-center gap-1.5 rounded px-3 py-1 text-mini font-medium text-white transition-opacity disabled:opacity-50"
              style={{ background: PLAN_COLOR }}
            >
              <Play size={12} />
              {applying ? '应用中…' : `应用这 ${preview.steps.length} 项`}
            </button>
          </div>
        </div>
      )}

      {/* ———— 快照 / 回滚 面板 ———— */}
      {showSnapshots && <SnapshotPanel onClose={() => setShowSnapshots(false)} />}

      {/* ——— 展开的变更列表（向上浮出，不挤压画布） ——— */}
      {open && hasChanges && (
        <div className="absolute inset-x-0 bottom-full max-h-[46vh] animate-slide-up overflow-y-auto border-t border-line bg-panel shadow-float scroll-thin">
          <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-line-subtle bg-panel/95 px-4 py-1.5 backdrop-blur">
            <span className="text-mini text-ink-muted">待应用的变更</span>
            <span className="tnum text-mini text-ink-dim">{rows.length} 项</span>
            <div className="flex-1" />
            <span className="text-2xs text-ink-faint">
              应用前有预演确认 · 执行即建快照
            </span>
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="text-ink-dim transition-colors hover:text-ink"
              title="关闭"
            >
              <X size={13} />
            </button>
          </div>

          <ul className="divide-y divide-line-subtle">
            {rows.map(({ entry, item, name, summary, diff }) => (
              <li key={entry.itemId} className="flex items-start gap-2.5 px-4 py-2 hover:bg-hover/40">
                <AppIcon name={name} iconData={item.iconData} size={20} />

                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span
                      className="shrink-0 rounded px-1 text-2xs leading-4"
                      style={{ color: PLAN_COLOR, background: `${PLAN_COLOR}1f` }}
                    >
                      {CHANGE_LABEL[entry.kind]}
                    </span>
                    <span className="truncate text-mini text-ink">{summary}</span>
                  </div>

                  {/* 差异行：只列真正变化的字段，全列等于没列 */}
                  <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5">
                    {diff.map((d) => (
                      <span key={d.label} className="text-2xs text-ink-dim">
                        {d.label}
                        <span className="mx-1 text-ink-faint line-through">{d.from}</span>
                        <span className="text-ink-faint">→ </span>
                        <span style={{ color: PLAN_COLOR }}>{d.to}</span>
                      </span>
                    ))}
                    {diff.length === 0 && (
                      <span className="text-2xs text-ink-faint">与原状态一致（无实际改动）</span>
                    )}
                  </div>
                </div>

                <button
                  type="button"
                  onClick={() => unstage(entry.itemId)}
                  title="撤掉这一条"
                  className="mt-0.5 shrink-0 text-ink-dim transition-colors hover:text-danger"
                >
                  <X size={13} />
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* ——— 底部条 ——— */}
      <div
        className="flex h-9 items-center gap-2 border-t px-4 text-mini"
        style={{
          borderColor: hasChanges ? `${PLAN_COLOR}4d` : '#21262d',
          background: hasChanges ? `${PLAN_COLOR}0d` : '#161b22',
        }}
      >
        <Layers size={13} style={{ color: hasChanges ? PLAN_COLOR : '#6e7681' }} />

        {hasChanges ? (
          <>
            <button
              type="button"
              onClick={() => setOpen((v) => !v)}
              className="flex items-center gap-1.5 text-ink transition-colors hover:text-plan"
            >
              <span>
                <span className="tnum font-medium">{rows.length}</span> 项待应用的变更
              </span>
              {open ? <ChevronDown size={12} /> : <ChevronUp size={12} />}
            </button>

            {flash && (
              <span className="animate-slide-up truncate text-2xs" style={{ color: PLAN_COLOR }}>
                {flash}
              </span>
            )}

            <div className="flex-1" />

            <IconAction
              icon={<Undo2 size={12} />}
              label="撤销"
              hint="撤销上一步改动（Ctrl+Z）"
              disabled={undoStack.length === 0}
              onClick={undo}
            />
            <IconAction
              icon={<Redo2 size={12} />}
              label="重做"
              hint="重做（Ctrl+Shift+Z）"
              disabled={redoStack.length === 0}
              onClick={redo}
            />

            <span className="mx-0.5 h-3.5 w-px bg-line" />

            <IconAction
              icon={<Trash2 size={12} />}
              label="放弃全部"
              hint="清空所有待应用的变更"
              onClick={() => {
                clear()
                setOpen(false)
              }}
            />

            <span className="mx-0.5 h-3.5 w-px bg-line" />

            <button
              type="button"
              onClick={() => void runPreview()}
              className="flex items-center gap-1 rounded px-2 py-0.5 text-2xs font-medium text-white transition-opacity hover:opacity-80"
              style={{ background: PLAN_COLOR }}
            >
              <Play size={12} />
              应用
            </button>

            <IconAction
              icon={<Copy size={12} />}
              label="复制方案"
              hint="把这份编排方案复制成文本，可保存备用"
              onClick={() => void copyPlan()}
            />
            <IconAction
              icon={<Download size={12} />}
              label="保存方案"
              hint="导出为 Markdown 文件"
              onClick={downloadPlan}
            />
          </>
        ) : (
          <>
            <span className="text-ink-dim">
              编排模式：勾选启动项后在右侧调整处置方式，改动会先收在这里
            </span>
            <div className="flex-1" />
            {busy === 'rollback' && (
              <span className="animate-pulse text-2xs text-ink-dim">正在回滚…</span>
            )}
            <button
              type="button"
              onClick={() => setShowSnapshots((v) => !v)}
              className="flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-ink-muted transition-colors hover:bg-hover hover:text-ink"
              title="历史快照、回滚与导出"
            >
              <Layers size={12} />
              快照与回滚
            </button>
            <span className="mx-0.5 h-3.5 w-px bg-line" />
            <span className="text-2xs text-ink-faint">
              空格勾选 · Ctrl+A 全选 · ↑↓ 移动 · Esc 退出编排
            </span>
          </>
        )}
      </div>
    </div>
  )
}

/** 预演步骤的风险徽章：Locked 之外都淡色处理，避免每行都带红 */
function SafeBadge({ risk }: { risk: string }) {
  if (risk !== 'Locked' && risk !== 'High') return null
  const color = risk === 'Locked' ? '#f85149' : '#d29922'
  return (
    <span className="shrink-0 rounded px-1 text-2xs" style={{ color, background: `${color}1f` }}>
      {risk === 'Locked' ? '禁改区' : '高风险'}
    </span>
  )
}

function IconAction({
  icon,
  label,
  hint,
  disabled,
  onClick,
}: {
  icon: React.ReactNode
  label: string
  hint: string
  disabled?: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={hint}
      className="flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-ink-muted transition-colors hover:bg-hover hover:text-ink disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent"
    >
      {icon}
      {label}
    </button>
  )
}