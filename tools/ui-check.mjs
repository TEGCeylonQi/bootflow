#!/usr/bin/env node
/**
 * BootFlow 界面自检 —— 不启动浏览器、不构建、不截图。
 *
 * 【为什么需要它】
 * `tsc` 管不到样式类名，`cargo build` 看不出前端令牌写没写对。
 * 之前就出过这样的事：组件里写了 `bg-panel-soft` / `text-ink-faint`，
 * 但这两个令牌**从来没在 tailwind.config.js 里定义过**。
 * Tailwind 对未知令牌是静默忽略的——不报错、不警告，样式就是不生效。
 * 后果还很隐蔽：`text-ink-faint` 失效后文字回退到继承色（最亮的那个），
 * 本该最淡的补充说明反而变成整屏最扎眼的东西，视觉层级整个反过来。
 *
 * 这类问题**靠肉眼看截图极难发现**（"看着有点怪，但说不上哪不对"），
 * 却可以被一次静态扫描精确抓住。所以做成脚本，进 CI 也好，本地随手跑也好。
 *
 * 【检查三项】
 *   1. 样式令牌是否存在（上面那类问题的正面拦截）
 *   2. TS 与 Rust 的数据契约是否一致（双份模型最容易漂移，且漂移了不报错）
 *   3. 硬编码数值清单（不判失败，只是提示可以收敛到令牌）
 *
 * 用法：node tools/ui-check.mjs   （或 npm run ui:check）
 * 退出码非 0 表示有必须处理的问题。
 */
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { pathToFileURL } from 'node:url'

const ROOT = process.cwd()
const SRC = join(ROOT, 'src')

/* ───────────────────────── 工具 ───────────────────────── */

const red = (s) => `\x1b[31m${s}\x1b[0m`
const green = (s) => `\x1b[32m${s}\x1b[0m`
const yellow = (s) => `\x1b[33m${s}\x1b[0m`
const dim = (s) => `\x1b[2m${s}\x1b[0m`
const bold = (s) => `\x1b[1m${s}\x1b[0m`

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    const s = statSync(p)
    if (s.isDirectory()) walk(p, out)
    else if (/\.(ts|tsx)$/.test(name)) out.push(p)
  }
  return out
}

const snakeToCamel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase())

/* ─────────────── 一、样式令牌校验 ─────────────── */

const theme = (await import(pathToFileURL(join(ROOT, 'tailwind.config.js')).href)).default.theme.extend

const COLORS = new Set(Object.keys(theme.colors))
const FONT_SIZES = new Set(['2xs', 'mini', 'body', 'xs', 'sm', 'base', 'lg', 'xl', '2xl'])
const RADII = new Set(['card', 'none', 'sm', 'md', 'lg', 'xl', 'full'])
const SHADOWS = new Set(Object.keys(theme.boxShadow))
const ANIMATIONS = new Set(Object.keys(theme.animation))
const DURATIONS = new Set(Object.keys(theme.transitionDuration))

/**
 * 颜色令牌的"词素"。用来判断一个陌生类名到底是"想用自定义语义色但拼错了/没定义"，
 * 还是"用的 Tailwind 自带调色板"。只有前者才报错，否则满屏误报没人会看。
 */
const COLOR_WORDS = new Set()
for (const c of COLORS) for (const w of c.split('-')) COLOR_WORDS.add(w)
// 字号名也可能出现在 text- 后面，一并纳入
for (const f of FONT_SIZES) COLOR_WORDS.add(f)

const UTIL_RE =
  /^(bg|text|border|divide|ring|fill|stroke|from|to|via|outline|decoration|shadow|rounded|animate|duration|accent|caret)-(.+)$/

/** 检查单个类名；返回 { kind, value } 或 null（表示无需关心） */
function inspectClass(raw) {
  let t = raw
  // 剥变体：hover: / md: / data-[x]: / group-hover: ... 取最后一个冒号之后即可
  const colon = t.lastIndexOf(':')
  if (colon >= 0) t = t.slice(colon + 1)
  // 剥不透明度后缀 bg-elevated/60
  t = t.split('/')[0]
  if (!t || t.startsWith('[')) return null

  const m = t.match(UTIL_RE)
  if (!m) return null
  const [, prefix, value] = m
  if (value.startsWith('[') || value.startsWith('(')) return null

  const words = value.split('-')
  const looksCustom = words.some((w) => COLOR_WORDS.has(w))
  if (!looksCustom) return null // Tailwind 自带调色板，不用管

  switch (prefix) {
    case 'shadow':
      return SHADOWS.has(value) ? null : { kind: 'boxShadow', value }
    case 'rounded':
      return RADII.has(value) ? null : { kind: 'borderRadius', value }
    case 'animate':
      return ANIMATIONS.has(value) ? null : { kind: 'animation', value }
    case 'duration':
      return DURATIONS.has(value) ? null : { kind: 'transitionDuration', value }
    case 'text':
      if (FONT_SIZES.has(value)) return null
      return COLORS.has(value) ? null : { kind: 'colors', value }
    default:
      return COLORS.has(value) ? null : { kind: 'colors', value }
  }
}

const files = walk(SRC)
const badTokens = []
const hardcoded = new Map() // 任意值 -> 出现次数

for (const file of files) {
  const text = readFileSync(file, 'utf8')
  const rel = relative(ROOT, file).replace(/\\/g, '/')

  // 从字符串字面量里挑类名候选：够用且不会误伤注释里的普通中文
  for (const strLit of text.matchAll(/(?:'|"|`)([^'"`\n]{1,400})(?:'|"|`)/g)) {
    const body = strLit[1]
    for (const tok of body.split(/\s+/)) {
      if (tok.includes('[') && tok.includes(']')) {
        const key = tok.match(/^[a-z-]+-\[[^\]]+\]$/)?.[0]
        if (key) hardcoded.set(key, (hardcoded.get(key) ?? 0) + 1)
        continue
      }
      const hit = inspectClass(tok)
      if (hit) badTokens.push({ file: rel, token: tok, ...hit })
    }
  }
}

/* ─────────── 二、TS ↔ Rust 数据契约一致性 ─────────── */

function rustStructFields(src, name) {
  const re = new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`)
  const body = src.match(re)?.[1]
  if (!body) return null
  return [...body.matchAll(/^\s*pub\s+([a-z0-9_]+)\s*:/gm)].map((m) => snakeToCamel(m[1]))
}

function tsInterfaceFields(src, name) {
  const re = new RegExp(`export interface ${name} \\{([\\s\\S]*?)\\n\\}`)
  const body = src.match(re)?.[1]
  if (!body) return null
  return [...body.matchAll(/^\s{2}([a-zA-Z0-9_]+)\??\s*:/gm)].map((m) => m[1])
}

const tsModel = readFileSync(join(SRC, 'types', 'model.ts'), 'utf8')
let rsModel = ''
try {
  rsModel = readFileSync(join(ROOT, 'src-tauri', 'src', 'model.rs'), 'utf8')
} catch {
  rsModel = ''
}

/**
 * 逐个比对的结构体清单。加入 BootTimeline / SlowService 是因为
 * 时间轴的字段是"诚实原则"的载体：`unavailableReason`、`needsElevation`
 * 一旦在某一侧漏掉，界面就会退回成一张空图，用户看到的是"开机不花时间"，
 * 而真实情况是"没权限读"。这种漂移静默且致命，必须被静态拦住。
 */
const CONTRACT = [
  'DesiredState',
  'StartupItem',
  'ScanResult',
  'BootTimeline',
  'SlowService',
  'PhaseSpan',
  'ItemTiming',
  'SignerInfo',
  'DiagnosticInfo',
  'Recommendation',
]

const contractDiffs = []
if (rsModel) {
  for (const name of CONTRACT) {
    const ts = tsInterfaceFields(tsModel, name)
    const rs = rustStructFields(rsModel, name)
    if (!ts || !rs) continue
    const tsSet = new Set(ts)
    const rsSet = new Set(rs)
    for (const f of ts) if (!rsSet.has(f)) contractDiffs.push(`${name}.${f} 只存在于 TS`)
    for (const f of rs) if (!tsSet.has(f)) contractDiffs.push(`${name}.${f} 只存在于 Rust`)
  }
}

/* ───────────────────── 输出 ───────────────────── */

console.log()
console.log(bold('BootFlow 界面自检'))
console.log(dim('─'.repeat(56)))
console.log(dim(`  扫描 ${files.length} 个源文件 · 令牌 ${COLORS.size} 色 / ${FONT_SIZES.size} 字号`))
console.log()

let failed = false

if (badTokens.length > 0) {
  failed = true
  console.log(red(`✗ 未定义的样式令牌（${badTokens.length} 处）`))
  console.log(dim('  Tailwind 会静默忽略它们——样式不生效，且不报任何错。'))
  for (const b of badTokens) {
    console.log(`   ${dim(b.file)}  ${red(b.token)}  ${dim(`← ${b.value} 不在 ${b.kind} 中`)}`)
  }
  console.log()
} else {
  console.log(green('✓ 样式令牌全部有定义'))
  console.log()
}

if (contractDiffs.length > 0) {
  failed = true
  console.log(red(`✗ TS 与 Rust 的数据契约不一致（${contractDiffs.length} 处）`))
  console.log(dim('  两份模型必须一一对应，否则字段会在传输中静默丢失。'))
  for (const d of contractDiffs) console.log(`   ${red(d)}`)
  console.log()
} else if (rsModel) {
  console.log(green(`✓ TS 与 Rust 的数据契约一致（${CONTRACT.length} 个结构体）`))
  console.log()
}

if (hardcoded.size > 0) {
  const list = [...hardcoded.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8)
  console.log(yellow(`△ 硬编码数值 ${hardcoded.size} 种（不判失败，建议收敛到令牌）`))
  for (const [k, n] of list) console.log(`   ${dim(k)}  ×${n}`)
  if (hardcoded.size > list.length) console.log(dim(`   …另有 ${hardcoded.size - list.length} 种`))
  console.log()
}

console.log(dim('─'.repeat(56)))
console.log(failed ? red(bold('自检未通过，请先处理上面的问题')) : green(bold('自检通过')))
console.log()

process.exit(failed ? 1 : 0)
