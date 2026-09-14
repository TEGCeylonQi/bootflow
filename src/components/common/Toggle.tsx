import type { ReactNode } from 'react'

interface Props {
  checked: boolean
  onChange: (v: boolean) => void
  disabled?: boolean
  label: string
  /** 关闭状态的说明，用于解释"为什么点不了" */
  disabledHint?: string
}

/**
 * 开关。
 *
 * 用原生 button + role="switch" 而不是 checkbox：
 * 这是"立即生效的开关"语义，不是"提交时才读取的表单字段"，
 * 屏幕阅读器和键盘用户对两者的预期不同。
 */
export function Toggle({ checked, onChange, disabled, label, disabledHint }: Props) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      title={disabled ? (disabledHint ?? '当前不可修改') : undefined}
      onClick={() => onChange(!checked)}
      className={[
        'relative h-[18px] w-[32px] shrink-0 rounded-full border transition-colors duration-fast',
        disabled ? 'cursor-not-allowed opacity-40' : 'cursor-pointer',
      ].join(' ')}
      style={{
        background: checked && !disabled ? '#3fb95033' : '#0d1117',
        borderColor: checked && !disabled ? '#3fb950' : '#30363d',
      }}
    >
      <span
        className="absolute top-[2px] h-[12px] w-[12px] rounded-full transition-all duration-fast"
        style={{
          left: checked ? 17 : 2,
          background: checked && !disabled ? '#3fb950' : '#6e7681',
        }}
      />
    </button>
  )
}

/** 带标签的一行编排控件：左侧标签，右侧控件 */
export function ControlRow({
  label,
  hint,
  children,
}: {
  label: string
  hint?: string
  children: ReactNode
}) {
  return (
    <div className="flex items-center gap-2 py-1">
      <span className="w-16 shrink-0 text-mini text-ink-muted" title={hint}>
        {label}
      </span>
      <div className="flex min-w-0 flex-1 items-center gap-1">{children}</div>
    </div>
  )
}
