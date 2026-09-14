import { useState } from 'react'

interface Props {
  name: string
  /** base64 PNG，由 Rust 侧批量提取。缺失时回退为首字母色块 */
  iconData?: string
  size?: number
}

/** 由名称稳定推导色相，保证同一个程序每次颜色一致 */
function hashHue(s: string): number {
  let h = 0
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360
  return h
}

export function AppIcon({ name, iconData, size = 20 }: Props) {
  const [broken, setBroken] = useState(false)

  if (iconData && !broken) {
    return (
      <img
        src={`data:image/png;base64,${iconData}`}
        alt=""
        width={size}
        height={size}
        draggable={false}
        onError={() => setBroken(true)}
        className="shrink-0 rounded-[3px] object-contain"
        style={{ width: size, height: size }}
      />
    )
  }

  const ch = name.trim().charAt(0).toUpperCase() || '?'
  const hue = hashHue(name)
  return (
    <div
      aria-hidden
      className="flex shrink-0 items-center justify-center rounded-[4px] font-semibold"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.55),
        background: `hsl(${hue} 38% 26%)`,
        color: `hsl(${hue} 70% 76%)`,
      }}
    >
      {ch}
    </div>
  )
}
