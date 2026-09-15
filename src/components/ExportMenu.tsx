import { useEffect, useRef, useState } from 'react'
import { Check, Download, FileJson, FileSpreadsheet, FileText } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { EXPORT_META, download, filenameFor, serialize, type ExportFormat } from '@/views/ReportExport'

/**
 * 「导出报告」。
 *
 * 【为什么导出的永远是全部项，而不是当前筛选结果】
 * 这份文件的用户不是坐在屏幕前的本人，而是**拿到文件的那个人**——
 * 帮他看电脑的朋友、修电脑的师傅、群里的热心人。他们看不到屏幕上的筛选条，
 * 一份"少了三十项"的报告在他们手里就是一份会误导人的报告。
 * 所以导出恒定为全量，筛选器管不着它。
 */
const FORMATS: { key: ExportFormat; icon: typeof FileText; label: string; hint: string }[] = [
  {
    key: 'markdown',
    icon: FileText,
    label: 'Markdown 报告',
    hint: '给人看：有结论、有依据、有建议，可以直接发出去',
  },
  {
    key: 'csv',
    icon: FileSpreadsheet,
    label: 'CSV 表格',
    hint: '给表格看：一行一项，能在 Excel 里排序筛选',
  },
  {
    key: 'json',
    icon: FileJson,
    label: 'JSON 数据',
    hint: '给程序看：完整结构，保留原始数据',
  },
]

export function ExportMenu() {
  const items = useAppStore((s) => s.items)
  const os = useAppStore((s) => s.os)
  const elevated = useAppStore((s) => s.elevated)
  const scannedAt = useAppStore((s) => s.scannedAt)
  const bootTimeline = useAppStore((s) => s.bootTimeline)
  const errors = useAppStore((s) => s.errors)
  const status = useAppStore((s) => s.status)

  const [open, setOpen] = useState(false)
  const [justDone, setJustDone] = useState<ExportFormat | null>(null)
  const [justDoneMessage, setJustDoneMessage] = useState<string | null>(null)
  const boxRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (!boxRef.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  const ready = status === 'ready' && !!os && !!bootTimeline

  const run = (f: ExportFormat) => {
    if (!ready || !os || !bootTimeline) return
    const content = serialize(f, {
      items,
      os,
      elevated,
      scannedAt: scannedAt ?? new Date().toISOString(),
      bootTimeline,
      errors,
    })
    download(filenameFor(f, scannedAt), content, EXPORT_META[f].mime)
    setOpen(false)
    setJustDone(f)
    setJustDoneMessage(`已保存到「下载」文件夹：${filenameFor(f, scannedAt)}`)
    window.setTimeout(() => {
      setJustDone((cur) => (cur === f ? null : cur))
      setJustDoneMessage(null)
    }, 2600)
  }

  return (
    <div className="relative" ref={boxRef}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        disabled={!ready}
        title={ready ? '把这份体检结果存成文件' : '扫描完成后才能导出'}
        className="flex items-center gap-1.5 rounded-md border border-line bg-elevated px-2.5 py-1 text-xs text-ink transition-colors hover:border-accent hover:text-accent disabled:cursor-not-allowed disabled:opacity-60"
      >
        {justDone ? <Check size={13} style={{ color: '#3fb950' }} /> : <Download size={13} />}
        {justDone ? '已导出' : '导出报告'}
      </button>

      {justDoneMessage && (
        <div className="absolute right-0 top-full z-30 mt-1 w-max max-w-xs animate-slide-up rounded-md border border-ok/40 bg-elevated px-2.5 py-1.5 text-2xs leading-4 text-ink shadow-float">
          <span className="flex items-start gap-1.5">
            <Check size={12} className="mt-[1px] shrink-0 text-ok" />
            <span className="break-all">{justDoneMessage}</span>
          </span>
        </div>
      )}

      {open && (
        <div className="absolute right-0 top-full z-30 mt-1 w-72 animate-slide-up rounded-card border border-line bg-panel p-1">
          {FORMATS.map(({ key, icon: Icon, label, hint }) => (
            <button
              key={key}
              type="button"
              onClick={() => run(key)}
              className="flex w-full items-start gap-2 rounded px-2 py-1.5 text-left transition-colors hover:bg-hover"
            >
              <Icon size={14} className="mt-[3px] shrink-0 text-ink-dim" />
              <span className="min-w-0">
                <span className="block text-xs text-ink">{label}</span>
                <span className="block text-2xs leading-4 text-ink-dim">{hint}</span>
              </span>
            </button>
          ))}

          <p className="border-t border-line-subtle px-2 pb-1 pt-1.5 text-2xs leading-4 text-ink-dim">
            导出内容包含全部 {items.length} 项（不受当前筛选影响）。
            <br />
            报告只描述现状，不含任何修改操作。
          </p>
        </div>
      )}
    </div>
  )
}
