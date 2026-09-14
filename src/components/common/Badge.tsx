import clsx from 'clsx'
import type { ReactNode } from 'react'

interface BadgeProps {
  children: ReactNode
  /** 文字色，通常传状态色 */
  color?: string
  /** 背景色，不传时按 color 低透明度生成 */
  bg?: string
  className?: string
  title?: string
}

/** 通用小徽章：来源标签、风险标签、相位标签都用它 */
export function Badge({ children, color, bg, className, title }: BadgeProps) {
  return (
    <span
      title={title}
      className={clsx(
        'inline-flex items-center gap-1 rounded px-1.5 py-[1px] text-2xs font-medium leading-4 whitespace-nowrap',
        className,
      )}
      style={{
        color: color ?? '#8b949e',
        background: bg ?? (color ? `${color}1f` : '#21262d'),
      }}
    >
      {children}
    </span>
  )
}

interface DotProps {
  color: string
  size?: number
  className?: string
  title?: string
}

/** 状态色点，用在清单行首与分组标题 */
export function Dot({ color, size = 6, className, title }: DotProps) {
  return (
    <span
      title={title}
      className={clsx('inline-block shrink-0 rounded-full', className)}
      style={{ width: size, height: size, background: color }}
    />
  )
}
