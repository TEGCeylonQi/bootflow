/**
 * 更新检测的前后端契约（对应 Rust 侧 `src-tauri/src/update.rs`）。
 *
 * ⚠️ **`status` 有三档，不是两档。**
 * 如果只有「已是最新 / 有新版」，那么网络不通时唯一能显示的就是「已是最新」——
 * 那是假的。用户会以为更新检查在正常工作，实际上他永远收不到更新提示。
 * 所以「没能检查成功」必须是独立的一档，且要带上原因。
 */

export type UpdateStatus = 'upToDate' | 'available' | 'failed'

/** Release 里挂的一个产物 */
export interface ReleaseAsset {
  name: string
  url: string
  size: number
}

export interface UpdateCheck {
  status: UpdateStatus
  /** 当前运行的版本，来自后端编译进二进制的版本号 */
  currentVersion: string
  latestVersion?: string | null
  releaseName?: string | null
  /** 发布页地址，用于「打开发布页」 */
  releaseUrl?: string | null
  /** ISO8601 */
  publishedAt?: string | null
  /** Release 正文（Markdown），后端已截断 */
  notes?: string | null
  assets: ReleaseAsset[]
  /** `status === 'failed'` 时一定有，人话描述 */
  reason?: string | null
  /** 这次检查发生在什么时候，ISO8601 */
  checkedAt: string
}
