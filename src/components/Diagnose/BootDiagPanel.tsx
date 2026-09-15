import { useRef } from 'react'
import { X } from 'lucide-react'
import { BootDiagResult, DiagTrigger, useBootDiag } from './BootDiag'

/**
 * 顶栏「一键诊断」弹层。
 *
 * 通过绝对定位浮在顶栏按钮下方；点击外部关闭。
 * 内容复用 BootDiag 组件（按钮 + 结果卡），不重复实现诊断逻辑。
 */
export function BootDiagPanel({ onClose }: { onClose: () => void }) {
  const { state, diag, msg, run } = useBootDiag()
  const backdropRef = useRef<HTMLDivElement>(null)

  return (
    <>
      {/* 点击背板关闭 */}
      <div
        ref={backdropRef}
        className="fixed inset-0 z-30"
        onClick={(e) => {
          if (e.target === backdropRef.current) onClose()
        }}
      />
      <div
        role="dialog"
        aria-label="开机性能一键诊断"
        className="absolute right-3 top-[calc(100%+6px)] z-40 w-[380px] max-w-[calc(100vw-24px)] rounded-lg border border-line bg-panel p-3 shadow-float animate-slide-up"
      >
        <div className="mb-2 flex items-center gap-2">
          <span className="text-mini font-medium text-ink">开机性能 · 一键诊断</span>
          <div className="flex-1" />
          <button
            type="button"
            onClick={onClose}
            className="text-ink-dim transition-colors hover:text-ink"
            title="关闭"
          >
            <X size={13} />
          </button>
        </div>

        <p className="mb-2.5 text-2xs leading-4 text-ink-dim">
          检查为什么没有开机性能数据：权限 / 策略 / 快速启动 / 从未记录。
        </p>

        <DiagTrigger state={state} onClick={() => void run()} label={state === 'idle' ? '开始诊断' : '重新诊断'} />

        {state === 'error' && <p className="mt-2 text-2xs leading-5 text-danger">{msg}</p>}

        {diag && (
          <div className="mt-3">
            <BootDiagResult diag={diag} />
          </div>
        )}
      </div>
    </>
  )
}