import { useEffect, useRef, useState } from 'react'
import { Monitor, X } from 'lucide-react'
import type { OsInfo } from '@/types/model'

/**
 * 顶栏「操作系统版本」徽标 → 点击展开的详情浮层。
 *
 * 平时只展示一行（Windows 11 · 26200），点开后给全量系统信息：
 * 主/次版本、构建号、SKU（含 DisplayVersion 24H2 这类）。
 * 放在浮层里不挤占顶栏高度；点击背板关闭。
 */
export function OsInfoChip({ os }: { os: OsInfo | null }) {
  const [open, setOpen] = useState(false)
  const backdropRef = useRef<HTMLDivElement>(null)

  // 浮层打开时按 Esc 关闭（背板只能靠点击，键盘用户需要这条路）
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  if (!os) {
    return (
      <span className="inline-flex items-center gap-1 rounded px-1.5 py-[1px] text-2xs font-medium leading-4 text-ink-dim">
        <Monitor size={11} />
        Windows · —
      </span>
    )
  }

  const isWin11 = os.major === 10 && os.build >= 22000
  const versionLabel = `Windows ${isWin11 ? '11' : os.major === 10 ? '10' : os.major}`
  const buildLabel = os.sku || versionLabel

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        title="查看系统版本详情（构建号 / 产品 SKU）"
        className={[
          'inline-flex items-center gap-1 rounded px-1.5 py-[1px] text-2xs font-medium leading-4 transition-colors',
          open
            ? 'bg-accent/15 text-accent'
            : 'text-ink-dim hover:bg-hover hover:text-ink',
        ].join(' ')}
      >
        <Monitor size={11} />
        {versionLabel} · {os.build}
      </button>

      {open && (
        <>
          {/* 点击背板关闭 */}
          <div
            ref={backdropRef}
            className="fixed inset-0 z-30"
            onClick={(e) => {
              if (e.target === backdropRef.current) setOpen(false)
            }}
          />
          <div
            role="dialog"
            aria-label="操作系统详情"
            className="absolute right-0 top-full z-40 mt-2 w-72 animate-slide-up rounded-lg border border-line bg-panel p-3 shadow-float"
          >
            <div className="mb-2 flex items-center gap-2">
              <Monitor size={13} className="text-accent" />
              <span className="text-mini font-medium text-ink">操作系统详情</span>
              <div className="flex-1" />
              <button
                type="button"
                onClick={() => setOpen(false)}
                className="text-ink-dim transition-colors hover:text-ink"
                title="关闭"
              >
                <X size={13} />
              </button>
            </div>

            <dl className="space-y-1.5 text-2xs leading-5">
              <div className="flex items-center justify-between">
                <dt className="text-ink-dim">版本</dt>
                <dd className="font-medium text-ink">{versionLabel}</dd>
              </div>
              <div className="flex items-center justify-between">
                <dt className="text-ink-dim">构建</dt>
                <dd className="tnum font-medium text-ink">{os.major}.{os.minor}.{os.build}</dd>
              </div>
              <div className="flex items-center justify-between">
                <dt className="text-ink-dim">产品</dt>
                <dd className="max-w-[60%] text-right font-medium text-ink">{buildLabel}</dd>
              </div>
            </dl>
          </div>
        </>
      )}
    </div>
  )
}