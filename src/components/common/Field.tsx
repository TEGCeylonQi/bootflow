import clsx from 'clsx'
import type { ReactNode } from 'react'

interface Props {
  label: string
  children: ReactNode
  /** 路径、命令行等需要等宽字体与自动换行 */
  mono?: boolean
  labelWidth?: string
}

/** 属性面板里的「标签 — 值」行，统一对齐 */
export function Field({ label, children, mono, labelWidth = 'w-14' }: Props) {
  return (
    <div className="flex gap-2 py-[3px]">
      <span className={clsx('shrink-0 pt-[1px] text-2xs text-ink-dim', labelWidth)}>{label}</span>
      <span
        className={clsx(
          'min-w-0 flex-1 selectable text-2xs leading-4 text-ink-muted',
          mono && 'break-all font-mono',
        )}
      >
        {children}
      </span>
    </div>
  )
}

/** 面板内的分区标题 */
export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-b border-line-subtle px-3 py-2.5">
      <h3 className="mb-1.5 text-2xs font-semibold uppercase tracking-wide text-ink-dim">{title}</h3>
      {children}
    </section>
  )
}
