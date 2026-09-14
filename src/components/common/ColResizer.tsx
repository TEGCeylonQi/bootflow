import { useCallback, useRef } from 'react'

interface Props {
  /** 当前宽度 */
  width: number
  onChange: (w: number) => void
  /** 被调整的栏位于手柄的哪一侧 */
  side: 'left' | 'right'
  min: number
  max: number
  /** 双击复位到该宽度 */
  resetWidth: number
  label: string
}

/**
 * 分栏拖拽手柄。
 *
 * 用 Pointer Events 而不是 mouse 事件：它把鼠标、触控笔、触摸统一成一套，
 * 并且配合 `setPointerCapture` 能在指针滑出窗口时继续跟踪——
 * 用 mouse 事件的话，用户拖得快一点、光标离开窗口，拖拽就会"粘住"不掉。
 *
 * 键盘也要能用：手柄本身可聚焦，←→ 每次调 16px。分栏如果只能靠鼠标拖，
 * 键盘用户就完全失去了这个能力。
 */
export function ColResizer({ width, onChange, side, min, max, resetWidth, label }: Props) {
  const dragging = useRef(false)
  const startX = useRef(0)
  const startW = useRef(0)
  const el = useRef<HTMLDivElement>(null)

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      dragging.current = true
      startX.current = e.clientX
      startW.current = width
      e.currentTarget.setPointerCapture(e.pointerId)
      el.current?.setAttribute('data-dragging', 'true')
    },
    [width],
  )

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (!dragging.current) return
      const dx = e.clientX - startX.current
      // side='left' 表示被拖的栏在手柄左侧：向右拖 → 变宽
      onChange(side === 'left' ? startW.current + dx : startW.current - dx)
    },
    [onChange, side],
  )

  const end = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    dragging.current = false
    el.current?.removeAttribute('data-dragging')
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId)
    }
  }, [])

  const nudge = (delta: number) => onChange(width + delta)

  return (
    <div
      ref={el}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(width)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      title={`拖动调整宽度（双击复位，方向键微调）`}
      className="col-resizer group relative bg-transparent"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onDoubleClick={() => onChange(resetWidth)}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 48 : 16
        if (e.key === 'ArrowLeft') {
          e.preventDefault()
          nudge(side === 'left' ? -step : step)
        } else if (e.key === 'ArrowRight') {
          e.preventDefault()
          nudge(side === 'left' ? step : -step)
        }
      }}
    >
      {/* 静默状态下只有一条发丝线；悬停/拖拽时高亮，不抢视线 */}
      <span className="pointer-events-none absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-line-subtle transition-colors group-hover:bg-accent/60" />
    </div>
  )
}
