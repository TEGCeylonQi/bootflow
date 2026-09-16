#!/usr/bin/env node
/**
 * Rust 侧本地闸门：clippy（警告即失败）+ 单元测试。
 *
 * 【为什么单独有这个脚本】
 * v0.2.0 开发期 CI 连红三次（run #11 / #12 / #13）都源于同一个盲区：
 * 本地只跑了 `cargo check --all-targets`，而 **`cargo check` 跑的是 rustc 的
 * lint，不包含 clippy 的 lint**。两者是两套东西——
 *   - rustc 的 `dead_code` 之类，`cargo check` 会报；
 *   - clippy 的 `items_after_test_module` / `question_mark` 之类，
 *     `cargo check` 一个字都不说。
 * 于是「本地零告警」与「CI clippy 红」可以同时成立，而 CI 是 `-D warnings`，
 * 一红就红到底。**红灯久了就没人看 CI**，所以缺的这道闸必须补上。
 *
 * 命令与 `.github/workflows/ci.yml` 的 rust job 保持一致，避免本地绿、CI 红。
 * 用的是 `cargo test`（全量，与 CI 相同）——带 `#[ignore]` 的真机用例不会被跑到，
 * 那些依赖「这台机器装了什么」，断言数量会在 runner 上假红灯（见 ci.yml 的说明）。
 *
 * 用法：
 *   node tools/verify-rust.mjs          # clippy + test
 *   node tools/verify-rust.mjs --quick  # 只跑 clippy（改动只涉及 lint 时够用）
 */
import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { homedir } from 'node:os'

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..')
const TAURI_DIR = join(ROOT, 'src-tauri')
const QUICK = process.argv.includes('--quick')

/**
 * 找 cargo。
 *
 * ⚠️ 本机（Git Bash）里 `cargo` **不在 PATH 里**，要手动
 * `export PATH="$HOME/.cargo/bin:$PATH"`。npm script 走的是 cmd，
 * 更不会继承那句 export——所以脚本自己去找，别让使用者记这条。
 * 找不到时给一句能直接照抄的提示，而不是丢一个 ENOENT 出来。
 */
function findCargo() {
  const candidates = [
    process.env.CARGO,
    join(homedir(), '.cargo', 'bin', 'cargo.exe'),
    join(homedir(), '.cargo', 'bin', 'cargo'),
    'cargo',
  ].filter(Boolean)

  for (const c of candidates) {
    if (c === 'cargo' || existsSync(c)) {
      const probe = spawnSync(c, ['--version'], { encoding: 'utf8' })
      if (probe.status === 0) return c
    }
  }
  return null
}

const cargo = findCargo()
if (!cargo) {
  console.error(
    '找不到 cargo。\n' +
      '如果是 Git Bash，先执行：export PATH="$HOME/.cargo/bin:$PATH"\n' +
      '（本机 cargo 装在用户级 ~/.cargo，不在默认 PATH 里）',
  )
  process.exit(2)
}

const steps = [
  {
    name: 'Clippy（警告即失败，与 CI 同一命令）',
    args: ['clippy', '--all-targets', '--', '-D', 'warnings'],
  },
]
if (!QUICK) {
  steps.push({ name: '单元测试（全量，与 CI 同一命令）', args: ['test'] })
}

for (const step of steps) {
  console.log(`\n──── ${step.name} ────`)
  const r = spawnSync(cargo, step.args, { cwd: TAURI_DIR, stdio: 'inherit' })
  if (r.status !== 0) {
    console.error(`\n[失败] ${step.name}（退出码 ${r.status}）`)
    console.error('本地闸门没通过，别推——CI 是同一条命令，推上去必红。')
    process.exit(r.status ?? 1)
  }
}

console.log('\n[通过] Rust 侧闸门全绿（clippy + 测试）。')
