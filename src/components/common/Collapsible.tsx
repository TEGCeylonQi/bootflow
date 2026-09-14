import { useState, type ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

interface Props {
  title: string
  /** 收起状态下右侧的提示文字 */
  hint?: string
  defaultOpen?: boolean
  children: ReactNode
}

/**
 * 通用折叠分区。
 *
 * 这是「一级界面只给人话、技术细节藏二级」这条要求的载体：
 * 属性面板里所有涉及路径、命令行、证书、原始诊断码的内容都装在这里，默认收起。
 */
export function Collapsible({ title, hint, defaultOpen = false, children }: Props) {
  const [open, setOpen] = useState(defaultOpen)

  return (
    <section className="border-b border-line-subtle">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-1.5 px-3 py-2 text-left transition-colors hover:bg-hover/50"
      >
        <ChevronRight
          size={12}
          className={['shrink-0 text-ink-dim transition-transform', open ? 'rotate-90' : ''].join(' ')}
        />
        <span className="text-mini font-medium tracking-wide text-ink-muted">{title}</span>
        <span className="flex-1" />
        {!open && hint && <span className="text-2xs text-ink-dim">{hint}</span>}
      </button>

      {open && <div className="px-3 pb-3">{children}</div>}
    </section>
  )
}
