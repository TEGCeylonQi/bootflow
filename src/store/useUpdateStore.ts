import { create } from 'zustand'
import { checkUpdate } from '@/api/commands'
import type { UpdateCheck } from '@/types/update'

/**
 * 「忽略此版本」记在本地。
 *
 * 为什么需要它：更新提示如果每次开机都跳出来，用户第三次就会开始无视它，
 * 到真正该升级的时候反而看不见了。给他一个"我知道这个版本了"的出口，
 * 剩下的提示才有分量。
 *
 * 注意它是**按版本号**忽略的，不是"以后都别提示"——
 * 下一个版本发布时，提示会重新出现。
 */
const IGNORED_KEY = 'bootflow.update.ignored-version'

function loadIgnored(): string | null {
  try {
    return window.localStorage.getItem(IGNORED_KEY)
  } catch {
    // 隐私模式等场景下 localStorage 会抛异常，静默降级为"没有忽略过"
    return null
  }
}

function saveIgnored(version: string | null): void {
  try {
    if (version) window.localStorage.setItem(IGNORED_KEY, version)
    else window.localStorage.removeItem(IGNORED_KEY)
  } catch {
    /* 存不下就算了，不值得为此打断用户 */
  }
}

interface UpdateState {
  result: UpdateCheck | null
  checking: boolean
  /** 用户明确表示"这个版本我知道了"的版本号 */
  ignored: string | null

  check: () => Promise<void>
  ignore: (version: string) => void
}

export const useUpdateStore = create<UpdateState>()((set, get) => ({
  result: null,
  checking: false,
  ignored: loadIgnored(),

  check: async () => {
    // 防重入：启动时的自动检查与用户手动点击可能撞在一起
    if (get().checking) return

    set({ checking: true })
    try {
      const result = await checkUpdate()
      set({ result, checking: false })
    } catch (e) {
      // 后端的 `check_update` 设计上不会抛错（失败会变成 status: failed），
      // 走到这里说明是 IPC 层出了问题。同样按"没检查成功"处理，不假装已是最新。
      set({
        checking: false,
        result: {
          status: 'failed',
          currentVersion: get().result?.currentVersion ?? '未知',
          assets: [],
          reason: e instanceof Error ? e.message : String(e),
          checkedAt: new Date().toISOString(),
        },
      })
    }
  },

  ignore: (version) => {
    saveIgnored(version)
    set({ ignored: version })
  },
}))

/**
 * 是否有**值得提示**的新版本。
 *
 * 同时排除掉"用户已忽略的那一个版本"——顶栏徽标靠它决定要不要亮。
 */
export function selectHasUpdate(s: UpdateState): boolean {
  const r = s.result
  if (!r || r.status !== 'available') return false
  return !!r.latestVersion && r.latestVersion !== s.ignored
}
