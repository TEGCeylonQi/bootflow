/**
 * 图表里按底色明暗挑前景色的工具。
 *
 * 【为什么需要它】
 * 开机相位是一道从深蓝到橙的渐变配色（见 `constants.ts` 的 PHASE_COLOR）。
 * 同一个文字色不可能在两端都清楚：深蓝底上写深灰等于没写。
 * 之前那版甘特图统一用深色文字，深蓝那几段上的标签基本读不出来——
 * 而这种问题**在截图里极容易被放过**（"看着有点糊，说不上哪不对"）。
 */
export function readableOn(hex: string, dark = '#0d1117', light = '#e6edf3'): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex)
  if (!m) return light
  const n = Number.parseInt(m[1], 16)
  const lum = 0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)
  return lum > 150 ? dark : light
}
