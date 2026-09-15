import { useEffect, useMemo, useRef, useState } from 'react'
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  Sparkles,
} from 'lucide-react'
import { cleanInstallCache, installUpdate, openReleasePage } from '@/api/commands'
import { selectHasUpdate, useUpdateStore } from '@/store/useUpdateStore'
import type { UpdateCheck, ReleaseAsset } from '@/types/update'

const OK = '#3fb950'
const WARN = '#d29922'
const IDLE = '#8b949e'

/** 弹层宽度。够放下更新说明，又不至于在 1440 宽的窗口里显得突兀 */
const PANEL_W = 380
/** 更新说明最多占多高，超出滚动——正文可能很长，不能让它顶开整个面板 */
const NOTES_MAX_H = 220

/**
 * 顶栏的「检查更新」入口。
 *
 * 【为什么状态一共有四档，而不是「有新版 / 没新版」两档】
 * `检查中` 和 `检查未完成` 都必须独立可见：
 * - 没有「检查中」，用户点完按钮后界面毫无反应，会以为按钮坏了；
 * - 没有「未完成」，网络不通时唯一能落到的状态就是「已是最新」，
 *   而那是假的——他会以为更新检查在正常工作，实际上永远收不到提示。
 *
 * 检查失败时沿用工具的「诚实原则」：把原因原样说出来，
 * 而不是用一个看起来正常的绿色对勾把它盖过去。
 */
export function UpdateBadge() {
  const result = useUpdateStore((s) => s.result)
  const checking = useUpdateStore((s) => s.checking)
  const check = useUpdateStore((s) => s.check)
  const ignore = useUpdateStore((s) => s.ignore)
  const hasUpdate = useUpdateStore(selectHasUpdate)

  const [open, setOpen] = useState(false)
  const boxRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return

    const onDown = (e: MouseEvent) => {
      if (!boxRef.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }

    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  const tone = toneOf(result, checking, hasUpdate)

  const onClick = () => {
    const next = !open
    setOpen(next)
    // 第一次打开时顺手检查一次：用户点这个按钮，要的就是"现在有没有新版"，
    // 让他再点一次「检查」是多余的一步
    if (next && !result && !checking) void check()
  }

  return (
    <div className="relative" ref={boxRef}>
      <button
        type="button"
        onClick={onClick}
        title={tone.hint}
        aria-expanded={open}
        className="flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs transition-colors"
        style={{
          borderColor: hasUpdate ? `${OK}66` : '#30363d',
          background: hasUpdate ? `${OK}1a` : '#21262d',
          color: tone.color,
        }}
      >
        {tone.icon}
        {tone.label}
      </button>

      {open && (
        <div
          className="absolute right-0 z-50 mt-1.5 overflow-hidden rounded-lg border border-line bg-panel shadow-float"
          style={{ width: PANEL_W }}
        >
          <Header result={result} checking={checking} />

          <div className="max-h-[420px] overflow-y-auto px-3 py-3">
            <Body
              result={result}
              checking={checking}
              onRetry={() => void check()}
              onOpenPage={() => {
                if (result?.releaseUrl) void openReleasePage(result.releaseUrl)
              }}
            />
          </div>

          {result?.status === 'available' && (
            <Footer
              onIgnore={() => {
                if (result.latestVersion) ignore(result.latestVersion)
                setOpen(false)
              }}
              onCheckAgain={() => void check()}
              checking={checking}
            />
          )}
        </div>
      )}
    </div>
  )
}

// ─────────────────────────────────────────────────────────────
// 顶栏按钮的四种状态
// ─────────────────────────────────────────────────────────────

interface Tone {
  color: string
  label: string
  icon: React.ReactNode
  hint: string
}

function toneOf(result: UpdateCheck | null, checking: boolean, hasUpdate: boolean): Tone {
  if (checking) {
    return {
      color: IDLE,
      label: '检查中',
      icon: <Loader2 size={13} className="animate-spin" />,
      hint: '正在向 GitHub 询问最新版本',
    }
  }

  if (!result) {
    return {
      color: IDLE,
      label: '检查更新',
      icon: <Download size={13} />,
      hint: '看看有没有新版本',
    }
  }

  if (hasUpdate && result.latestVersion) {
    return {
      color: OK,
      label: `v${result.latestVersion} 可用`,
      icon: <Sparkles size={13} />,
      hint: `发现新版本 v${result.latestVersion}，点击查看`,
    }
  }

  if (result.status === 'upToDate') {
    return {
      color: IDLE,
      label: '已是最新',
      icon: <CheckCircle2 size={13} />,
      hint: `当前 v${result.currentVersion} 已是最新版本`,
    }
  }

  return {
    color: WARN,
    label: '检查未完成',
    icon: <AlertTriangle size={13} />,
    hint: result.reason ?? '这次没能问到最新版本',
  }
}

// ─────────────────────────────────────────────────────────────
// 面板
// ─────────────────────────────────────────────────────────────

function Header({ result, checking }: { result: UpdateCheck | null; checking: boolean }) {
  return (
    <div className="flex items-center justify-between border-b border-line px-3 py-2.5">
      <span className="text-xs font-semibold text-ink">版本更新</span>
      <span className="tnum text-2xs text-ink-dim">
        {checking ? '检查中…' : result ? `当前 v${result.currentVersion}` : '尚未检查'}
      </span>
    </div>
  )
}

function Body({
  result,
  checking,
  onRetry,
  onOpenPage,
}: {
  result: UpdateCheck | null
  checking: boolean
  onRetry: () => void
  onOpenPage: () => void
}) {
  if (checking && !result) return <Checking />

  if (!result) {
    return <p className="text-mini text-ink-muted">点上面的按钮就能查。</p>
  }

  if (result.status === 'available') return <Available result={result} onOpenPage={onOpenPage} />
  if (result.status === 'upToDate') return <UpToDate result={result} onRetry={onRetry} />
  return <Failed result={result} onRetry={onRetry} />
}

function Checking() {
  return (
    <div className="flex items-center gap-2 py-2 text-mini text-ink-muted">
      <Loader2 size={14} className="animate-spin" />
      正在向 GitHub 询问最新版本…
    </div>
  )
}

function Available({ result, onOpenPage }: { result: UpdateCheck; onOpenPage: () => void }) {
  // 安装流程状态机：idle → downloading → done / error → idle（可重试）。
  // 注意没有独立的 "installing" 档：后端的 install_update 一次调用就完成
  // 「下载 → 拉起安装向导 → 清理缓存」，中间没有可插桩的节点，就不假装有。
  const [installState, setInstallState] = useState<'idle' | 'downloading' | 'done' | 'error'>(
    'idle',
  )
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  // 默认挑排在最前的产物（后端已把安装包排到第一位）
  const [chosen, setChosen] = useState<ReleaseAsset | null>(result.assets[0] ?? null)
  const busy = installState === 'downloading'

  const download = async () => {
    if (!chosen || busy) return
    setInstallState('downloading')
    setErrorMsg(null)
    try {
      await installUpdate(chosen.url)
      setInstallState('done')
    } catch (e) {
      setErrorMsg(e instanceof Error ? e.message : String(e))
      setInstallState('error')
    }
  }

  // 退出面板时复位：下次打开又是一副干净的样子，不会残留"下载完成"的旧状态
  useEffect(() => () => setInstallState('idle'), [])

  return (
    <div className="space-y-3">
      <div>
        <div className="flex items-baseline gap-2">
          <span className="text-sm font-semibold" style={{ color: OK }}>
            v{result.latestVersion}
          </span>
          <span className="tnum text-2xs text-ink-dim">当前 v{result.currentVersion}</span>
        </div>
        {result.releaseName && (
          <div className="mt-0.5 text-mini text-ink-muted">{result.releaseName}</div>
        )}
        {result.publishedAt && (
          <div className="mt-0.5 text-2xs text-ink-dim">发布于 {fmtDateTime(result.publishedAt)}</div>
        )}
      </div>

      {result.notes && (
        <div
          className="overflow-y-auto rounded-md border border-line bg-base px-2.5 py-2"
          style={{ maxHeight: NOTES_MAX_H }}
        >
          <Notes text={result.notes} />
        </div>
      )}

      <div>
        <div className="mb-1 text-2xs text-ink-dim">这一版发布了这些文件</div>
        <div className="space-y-1">
          {result.assets.map((a) => (
            <div key={a.name} className="flex items-center justify-between gap-2 text-2xs">
              <span className="truncate font-mono text-ink-muted" title={a.name}>
                {a.name}
              </span>
              <span className="shrink-0 text-ink-dim">{fmtSize(a.size)}</span>
            </div>
          ))}
        </div>
      </div>

      {/* 下载并安装的入口 */}
      {installState === 'done' ? (
        <div className="flex items-center gap-2 rounded-md border border-line bg-base px-2.5 py-2 text-2xs text-ink-muted">
          <CheckCircle2 size={13} style={{ color: OK }} />
          安装向导已打开，完成安装后自动清理缓存
        </div>
      ) : installState === 'error' ? (
        <div className="space-y-2">
          <div className="flex items-start gap-2 rounded-md border border-line bg-base px-2.5 py-2 text-2xs leading-relaxed text-ink-muted">
            <AlertTriangle size={13} className="mt-0.5 shrink-0" style={{ color: WARN }} />
            <div className="min-w-0">
              <div className="text-mini text-ink">下载并安装没有成功</div>
              <div className="mt-0.5 break-words">{errorMsg}</div>
              <div className="mt-1 text-ink-dim">可以选择重试，或打开发布页手动下载。</div>
            </div>
          </div>
          <PrimaryButton onClick={download} disabled={!chosen || busy}>
            <RefreshCw size={13} />
            重试
          </PrimaryButton>
        </div>
      ) : installState === 'downloading' ? (
        <div className="flex items-center gap-2 text-2xs text-ink-muted">
          <Loader2 size={13} className="animate-spin" />
          正在下载并准备安装…
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <PrimaryButton onClick={download} disabled={!chosen || busy}>
            <Download size={13} />
            下载并安装
          </PrimaryButton>
          {result.assets.length > 1 && (
            <select
              value={chosen?.name ?? ''}
              onChange={(e) => {
                const hit = result.assets.find((a) => a.name === e.target.value)
                if (hit) setChosen(hit)
              }}
              disabled={busy}
              className="min-w-0 flex-1 rounded-md border border-line bg-elevated px-1.5 py-1 text-2xs text-ink-muted"
              aria-label="选择下载的安装包"
            >
              {result.assets.map((a) => (
                <option key={a.name} value={a.name}>
                  {a.name}
                </option>
              ))}
            </select>
          )}
        </div>
      )}

      <p className="text-2xs leading-relaxed text-ink-dim">
        下载后直接运行安装包覆盖安装即可，启动项配置不受影响。
        安装完成后缓存会自动清理。
      </p>

      <div className="flex items-center justify-between">
        <SecondaryButton onClick={onOpenPage} disabled={!result.releaseUrl}>
          <ExternalLink size={12} />
          打开发布页
        </SecondaryButton>
        <button
          type="button"
          onClick={() => void cleanInstallCache().then(() => setInstallState('idle'))}
          className="text-2xs text-ink-dim transition-colors hover:text-ink-muted"
          title="清理上次下载残留的安装包"
        >
          清理下载缓存
        </button>
      </div>
    </div>
  )
}

function UpToDate({ result, onRetry }: { result: UpdateCheck; onRetry: () => void }) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <CheckCircle2 size={15} style={{ color: OK }} />
        <span className="text-mini text-ink">
          已经是最新版本（v{result.currentVersion}）
        </span>
      </div>
      <div className="text-2xs text-ink-dim">检查时间 {fmtDateTime(result.checkedAt)}</div>
      <SecondaryButton onClick={onRetry}>
        <RefreshCw size={12} />
        再检查一次
      </SecondaryButton>
    </div>
  )
}

function Failed({ result, onRetry }: { result: UpdateCheck; onRetry: () => void }) {
  return (
    <div className="space-y-3">
      <div className="flex gap-2">
        <AlertTriangle size={15} className="mt-0.5 shrink-0" style={{ color: WARN }} />
        <div className="min-w-0 space-y-1">
          <div className="text-mini text-ink">这次没能检查到最新版本</div>
          <div className="break-words text-2xs leading-relaxed text-ink-muted">
            {result.reason ?? '未知原因'}
          </div>
        </div>
      </div>

      {/*
       * 这句话是必须写的。用户看到"检查失败"时，最自然的联想是
       * "那我这个版本是不是旧的？"——必须明确告诉他：这一档什么都没说明，
       * 既不代表有新版本，也不代表没有。
       */}
      <p className="rounded-md border border-line bg-base px-2.5 py-2 text-2xs leading-relaxed text-ink-muted">
        这不代表你用的是最新版，也不代表有新版本——只是这次没能问到。
        已经装好的程序不受影响，随时可以重试。
      </p>

      <SecondaryButton onClick={onRetry}>
        <RefreshCw size={12} />
        重试
      </SecondaryButton>
    </div>
  )
}

function Footer({
  onIgnore,
  onCheckAgain,
  checking,
}: {
  onIgnore: () => void
  onCheckAgain: () => void
  checking: boolean
}) {
  return (
    <div className="flex items-center justify-between border-t border-line bg-base px-3 py-2">
      <button
        type="button"
        onClick={onIgnore}
        className="text-2xs text-ink-dim transition-colors hover:text-ink-muted"
        title="不再提示这个版本；下一个版本发布时会重新提醒"
      >
        忽略此版本
      </button>
      <button
        type="button"
        onClick={onCheckAgain}
        disabled={checking}
        className="flex items-center gap-1 text-2xs text-ink-dim transition-colors hover:text-ink-muted disabled:opacity-50"
      >
        <RefreshCw size={11} className={checking ? 'animate-spin' : undefined} />
        重新检查
      </button>
    </div>
  )
}

// ─────────────────────────────────────────────────────────────
// 小零件
// ─────────────────────────────────────────────────────────────

function PrimaryButton({
  onClick,
  disabled,
  children,
}: {
  onClick: () => void
  disabled?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex w-full items-center justify-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50"
      style={{ borderColor: `${OK}66`, background: `${OK}1f`, color: OK }}
    >
      {children}
    </button>
  )
}

function SecondaryButton({
  onClick,
  disabled,
  children,
}: {
  onClick: () => void
  disabled?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 rounded-md border border-line bg-elevated px-2.5 py-1 text-2xs text-ink transition-colors hover:border-accent hover:text-accent disabled:cursor-not-allowed disabled:opacity-50"
    >
      {children}
    </button>
  )
}

/**
 * 更新说明的极简渲染。
 *
 * 不引 Markdown 库：Release 正文是我们自己写的，只会用到标题、列表、
 * 行内粗体/代码、表格这几样。为它们装一个解析器（外加一个 HTML 渲染器
 * 和随之而来的注入风险）不划算。
 *
 * 遇到不认识的行**不丢弃**，按普通段落渲染——说明内容不该因为
 * 界面的渲染能力而悄悄少一截。
 */
function Notes({ text }: { text: string }) {
  const nodes = useMemo(() => renderNotes(text), [text])
  return <div className="space-y-1 text-mini leading-relaxed text-ink-muted">{nodes}</div>
}

function stripInline(s: string): string {
  return s
    .replace(/\*\*(.+?)\*\*/g, '$1')
    .replace(/`(.+?)`/g, '$1')
    .replace(/\[(.+?)\]\((.+?)\)/g, '$1')
}

function renderNotes(md: string): React.ReactNode[] {
  const out: React.ReactNode[] = []

  md.split('\n').forEach((raw, i) => {
    const line = raw.trim()
    if (!line) return

    if (/^-{3,}$/.test(line)) {
      out.push(<hr key={i} className="my-2 border-line" />)
      return
    }

    if (/^#{1,6}\s/.test(line)) {
      out.push(
        <div key={i} className="pt-1 text-mini font-semibold text-ink">
          {stripInline(line.replace(/^#+\s*/, ''))}
        </div>,
      )
      return
    }

    if (/^[-*]\s+/.test(line)) {
      out.push(
        <div key={i} className="flex gap-1.5">
          <span className="text-ink-dim">·</span>
          <span>{stripInline(line.replace(/^[-*]\s+/, ''))}</span>
        </div>,
      )
      return
    }

    // 表格在这么窄的弹层里放不下，降级成「文件名 + 说明」两栏一行
    if (line.startsWith('|')) {
      if (/^\|[\s:|-]+\|$/.test(line)) return
      const cells = line
        .split('|')
        .map((c) => stripInline(c.trim()))
        .filter(Boolean)
      if (cells.length < 2) return
      out.push(
        <div key={i} className="flex gap-2">
          <span className="shrink-0 font-mono text-2xs text-ink">{cells[0]}</span>
          <span>{cells.slice(1).join(' ')}</span>
        </div>,
      )
      return
    }

    out.push(<p key={i}>{stripInline(line)}</p>)
  })

  return out
}

function fmtDateTime(iso?: string | null): string {
  if (!iso) return '时间未知'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return '时间未知'
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

function fmtSize(bytes: number): string {
  if (!bytes || bytes <= 0) return ''
  const mb = bytes / 1024 / 1024
  return mb >= 1 ? `${mb.toFixed(2)} MB` : `${Math.round(bytes / 1024)} KB`
}
