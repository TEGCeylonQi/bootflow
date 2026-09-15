import { useState } from 'react'
import { useAppStore } from '@/store/useAppStore'

/**
 * 「每次开机自记账」开关。
 *
 * ## 为什么这一项要有开关，而且**默认关**
 *
 * 它要在系统里常驻一个自启条目（提权时是登录计划任务，否则是 `HKCU\...\Run`）。
 * 哪怕它完全无害——不弹窗、不联网、只往本地写一个小 JSON——"我装了个工具，
 * 它悄悄给我加了开机自启"本身就是越界：用户没同意，也没地方关掉。
 *
 * 所以它是 opt-in。关掉之后其余功能**照常工作**：
 * 「每项在开机后第几秒出现」来自进程采样，不需要自启条目、也不需要任何权限。
 * 这个区别必须写在界面上——否则用户会以为关掉开关就什么都看不到了。
 *
 * ## 状态的唯一来源是后端返回值
 *
 * 切换时不做乐观更新。后端只在**系统层面确实改完之后**才落盘设置值，
 * 所以拿它的返回值才可信；乐观更新会出现"开关显示已打开、系统里其实没有"
 * 的最坏情形，而用户没有任何办法发现。
 */
export function BootRecordingToggle() {
  const settings = useAppStore((s) => s.settings)
  const busy = useAppStore((s) => s.settingsBusy)
  const setBootRecording = useAppStore((s) => s.setBootRecording)
  const [error, setError] = useState<string | null>(null)

  // settings 还没读回来时按"关闭"显示（那是后端的默认值），而不是显示成已开启。
  const on = settings?.bootRecording ?? false

  const toggle = async () => {
    setError(null)
    try {
      await setBootRecording(!on)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <div className="rounded-card border border-line bg-base px-3 py-2.5">
      <div className="flex items-center gap-2">
        <span className="text-xs font-medium text-ink">每次开机记一条用时</span>
        <span className="text-2xs text-ink-dim">可选功能 · 默认关闭</span>
        <div className="flex-1" />
        <button
          type="button"
          role="switch"
          aria-checked={on}
          disabled={busy}
          onClick={() => void toggle()}
          title={
            on
              ? '关闭后本程序会从系统里移除它登记的自启条目（计划任务与注册表 Run 都会清掉）'
              : '开启后本程序会在系统里登记一个自启条目，每次登录时静默写一条本次开机用时'
          }
          className={[
            'relative h-[18px] w-[32px] shrink-0 rounded-full transition-colors',
            busy ? 'cursor-wait opacity-60' : 'cursor-pointer',
          ].join(' ')}
          style={{ background: on ? '#58a6ff' : '#30363d' }}
        >
          <span
            className="absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white transition-all"
            style={{ left: on ? 16 : 2 }}
          />
        </button>
        <span className="w-[26px] shrink-0 text-2xs" style={{ color: on ? '#58a6ff' : '#8b949e' }}>
          {busy ? '…' : on ? '已开' : '已关'}
        </span>
      </div>

      <p className="mt-1.5 text-2xs leading-5 text-ink-muted">
        开启后，BootFlow 会在系统里登记一个<span className="text-ink">自启条目</span>
        （已提权时为登录计划任务，否则为当前用户的注册表启动项），
        每次登录时静默记下"本次开机从内核启动到登录完成用了多久"，写进
        <span className="text-ink-dim"> %LOCALAPPDATA%\BootFlow\boot-records.json</span>。
        不弹窗、不联网、不采集任何其他信息。关掉时会把这个自启条目一并移除。
      </p>

      <p className="mt-1 text-2xs leading-5 text-ink-dim">
        它只负责上面那条
        <span className="text-ink-muted">总时长曲线</span>。
        下面的「每项在开机后第几秒出现」<span className="text-ink-muted">不依赖这个开关</span>
        ——那份数据来自进程采样，不需要自启条目，也不需要管理员权限，关掉开关同样看得到。
      </p>

      {error && (
        <p className="mt-1 text-2xs leading-5" style={{ color: '#f85149' }}>
          切换失败：{error}
        </p>
      )}
    </div>
  )
}
