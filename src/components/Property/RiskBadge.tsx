import { Lock, TriangleAlert, Info, ShieldCheck } from 'lucide-react'
import { RISK_META } from '@/constants'
import type { RiskLevel } from '@/types/model'

const ICONS: Record<RiskLevel, typeof Lock> = {
  Locked: Lock,
  High: TriangleAlert,
  Medium: Info,
  Safe: ShieldCheck,
}

export function RiskBadge({ level, showDesc = false }: { level: RiskLevel; showDesc?: boolean }) {
  const meta = RISK_META[level]
  const Icon = ICONS[level]

  return (
    <div className="flex flex-col gap-1">
      <span
        className="inline-flex w-fit items-center gap-1.5 rounded border px-2 py-0.5 text-2xs font-semibold"
        style={{ color: meta.color, borderColor: `${meta.color}66`, background: `${meta.color}1a` }}
      >
        <Icon size={11} />
        {meta.label}
      </span>
      {showDesc && <p className="text-2xs leading-4 text-ink-dim">{meta.desc}</p>}
    </div>
  )
}
