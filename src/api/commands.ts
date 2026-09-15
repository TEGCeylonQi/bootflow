/**
 * 与 Rust 后端的唯一调用面。
 *
 * 关键设计：浏览器里跑（`npm run dev`）时自动回退到 mock 数据，
 * 这样前端可以完全脱离 Rust 独立开发与预览；一旦运行在 Tauri 容器里
 * (`__TAURI_INTERNALS__` 存在) 则走真实 invoke。
 */
import type { BootTimeline, OsInfo, ScanResult, SourceKind, StartupItem } from '@/types/model'
import type { UpdateCheck } from '@/types/update'

/** 检测是否运行在 Tauri 容器内 */
export const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

async function invokeSafe<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(cmd, args)
}

/** 模拟扫描延迟，让加载动效可见 */
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms))

/**
 * 开发用开关：地址后加 `?timeline=denied`，切到「读不到开机性能日志」那份数据。
 *
 * 有这个开关，是因为那条分支不是臆想出来的边界情况——**本机非提权运行就是这个结果**。
 * 系统日志通道的权限列表里没有普通用户，读取直接被拒。如果只凭"有数据"的那份
 * mock 开发，很容易把读不到写成"显示一张空图"，而空图在用户眼里等于
 * "你的开机不花时间"。让这条分支随时可复现，比事后靠想象力补界面可靠得多。
 */
function mockTimelineDenied(): boolean {
  if (typeof window === 'undefined') return false
  return new URLSearchParams(window.location.search).get('timeline') === 'denied'
}

export async function scanAll(): Promise<ScanResult> {
  if (!isTauri()) {
    const mod = await import('@/mock/mockData')
    await delay(900)
    const data = mockTimelineDenied() ? mod.MOCK_SCAN_RESULT_TIMELINE_DENIED : mod.MOCK_SCAN_RESULT
    return structuredClone(data)
  }
  return invokeSafe<ScanResult>('scan_all')
}

export async function scanSource(source: SourceKind): Promise<StartupItem[]> {
  if (!isTauri()) {
    const { MOCK_ITEMS } = await import('@/mock/mockData')
    await delay(300)
    return MOCK_ITEMS.filter((i) => i.source === source)
  }
  return invokeSafe<StartupItem[]>('scan_source', { source })
}

export async function getBootTimeline(): Promise<BootTimeline> {
  if (!isTauri()) {
    const mod = await import('@/mock/mockData')
    const data = mockTimelineDenied() ? mod.MOCK_SCAN_RESULT_TIMELINE_DENIED : mod.MOCK_SCAN_RESULT
    return structuredClone(data.bootTimeline)
  }
  return invokeSafe<BootTimeline>('get_boot_timeline')
}

export async function getOsInfo(): Promise<OsInfo> {
  if (!isTauri()) {
    const { MOCK_SCAN_RESULT } = await import('@/mock/mockData')
    return MOCK_SCAN_RESULT.os
  }
  return invokeSafe<OsInfo>('get_os_info')
}

export async function checkElevation(): Promise<boolean> {
  if (!isTauri()) return false
  return invokeSafe<boolean>('check_elevation')
}

/** 触发 UAC 重新以管理员身份拉起自身（参数会透传） */
export async function requestElevation(): Promise<void> {
  if (!isTauri()) {
    console.warn('[BootFlow] 浏览器环境无法提权，此调用被忽略')
    return
  }
  return invokeSafe<void>('request_elevation')
}

/** 取图标的请求。后端在扫描阶段不提取图标，所以要把路径一起回传。 */
export interface IconRequest {
  id: string
  path: string
  /** 像素尺寸，缺省由后端决定（32） */
  size?: number
}

/** 后进：批量取图标。返回 id -> base64 PNG 的映射（取不到的项不会出现在结果里） */
export async function getIcons(requests: IconRequest[]): Promise<Record<string, string>> {
  // 浏览器模式没有 Shell，取不到真实图标——直接返回空，
  // 由 AppIcon 的兜底色块顶上，不去伪造图标
  if (!isTauri() || requests.length === 0) return {}
  return invokeSafe<Record<string, string>>('get_icons', { requests })
}

/**
 * 检查有没有新版本。
 *
 * 后端的 `check_update` 设计上不会抛错——「连不上」「被限流」都会变成
 * `status: 'failed'` 的正常返回，由界面如实呈现。所以这里也不需要 try/catch，
 * **绝不能把失败悄悄咽掉**，那就等于谎报「已是最新」。
 *
 * 浏览器开发模式下回退到 mock，用 `?update=latest` / `?update=failed` 切换状态。
 */
export async function checkUpdate(): Promise<UpdateCheck> {
  if (!isTauri()) {
    const { mockUpdateResult } = await import('@/mock/mockUpdate')
    await delay(700)
    return mockUpdateResult()
  }
  return invokeSafe<UpdateCheck>('check_update')
}

/**
 * 用系统默认浏览器打开发布页。
 *
 * 地址只会来自后端返回的 `releaseUrl`；后端 `open_release_page` 还会再校验一次
 * 域名是否属于 GitHub。前端不做判断，也不允许传任意地址。
 */
export async function openReleasePage(url: string): Promise<void> {
  if (!isTauri()) {
    window.open(url, '_blank', 'noopener,noreferrer')
    return
  }
  return invokeSafe<void>('open_release_page', { url })
}

/**
 * 在事件查看器里打开「开机诊断」通道，让用户亲眼看到 Windows 自己的记录。
 *
 * 当程序因权限不足读不到开机耗时、或系统还没有记录时，前端提供这个入口：
 * 一键把用户带到 Windows 事件查看器的对应日志，而不是让他在系统里瞎翻。
 *
 * 浏览器开发模式下是空操作（没有事件查看器可开），由 UI 决定如何降级。
 */
export async function openBootLog(): Promise<void> {
  if (!isTauri()) return
  return invokeSafe<void>('open_boot_log')
}
