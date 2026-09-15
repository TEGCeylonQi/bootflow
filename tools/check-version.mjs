#!/usr/bin/env node
/**
 * 版本号一致性检查。
 *
 * 版本号在三个地方各存了一份，而且没有哪一处能自动同步另外两处：
 *
 *   package.json                前端包版本（npm 用）
 *   src-tauri/tauri.conf.json   安装包版本，也是 release.yml 里 tag 名的来源
 *   src-tauri/Cargo.toml        Rust 包版本 —— 后端 `check_update` 比较的就是它
 *
 * 三者不一致的后果很隐蔽：发出去的安装包是最新的，但程序里"当前版本"
 * 读的是另一份，于是它会把用户手上这个刚装好的新版反复提示成"有新版本"。
 * 这种错没人会立刻发现，所以必须静态拦住。
 *
 * 顺带校验格式：后端要用语义化版本比较，`0.1.1` 之外还得支持 `1.0.0-beta.2`。
 * 写成 `v0.1` 或 `0.1.1.2` 会让更新检测直接失效，在这里就该被拒绝。
 */
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..')

const red = (s) => `\x1b[31m${s}\x1b[0m`
const green = (s) => `\x1b[32m${s}\x1b[0m`
const dim = (s) => `\x1b[2m${s}\x1b[0m`
const bold = (s) => `\x1b[1m${s}\x1b[0m`

const read = (p) => readFileSync(join(ROOT, p), 'utf8')

/** 语义化版本：主.次.修订，可选预发布标识 */
const SEMVER = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/

function packageJsonVersion() {
  return JSON.parse(read('package.json')).version ?? null
}

function tauriConfVersion() {
  return JSON.parse(read('src-tauri/tauri.conf.json')).version ?? null
}

function cargoVersion() {
  // [package] 段的 version，取第一个匹配即可（文件里还有各依赖的 version）
  const m = read('src-tauri/Cargo.toml').match(/^version\s*=\s*"([^"]+)"/m)
  return m?.[1] ?? null
}

const sources = [
  ['package.json', packageJsonVersion()],
  ['src-tauri/tauri.conf.json', tauriConfVersion()],
  ['src-tauri/Cargo.toml', cargoVersion()],
]

const problems = []

for (const [file, version] of sources) {
  if (!version) problems.push(`${file}：读不到版本号`)
  else if (!SEMVER.test(version)) problems.push(`${file}：版本号「${version}」不是语义化版本`)
}

const distinct = [...new Set(sources.map(([, v]) => v))]
if (distinct.length > 1) {
  problems.push(
    `三处版本号不一致：${sources.map(([f, v]) => `${f}=${v}`).join('，')}`,
  )
}

console.log()
console.log(bold('BootFlow 版本号检查'))
console.log(dim('─'.repeat(56)))

if (problems.length > 0) {
  console.log(red(`✗ 版本号有问题（${problems.length} 处）`))
  console.log(dim('  三处必须完全一致——否则程序会把刚装上的新版提示成"有新版本"。'))
  for (const p of problems) console.log(`   ${red(p)}`)
  console.log()
  process.exit(1)
}

const version = distinct[0]
console.log(green(`✓ 三处版本号一致：${version}`))
console.log(dim(`  package.json · tauri.conf.json · Cargo.toml`))
console.log(dim(`  发版：改这三处 → git tag v${version} → git push origin main --tags`))
console.log()
