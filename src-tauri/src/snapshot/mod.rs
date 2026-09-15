//! 快照与写护栏模块 —— v0.2.0「可写可控」的地基。
//!
//! 分层：
//! - `model.rs`      快照数据结构（含 schema 版本、SnapshotTarget、SnapshotRecord）
//! - `store.rs`      快照落盘到 `%APPDATA%\BootFlow\snapshots\`，原子写 + round-trip 校验
//! - `guard.rs`      写权限护栏（禁改区 Locked 项拒绝所有写；编译期 + 运行期双保险）
//! - `plan.rs`       （T43）预演引擎，纯函数零副作用
//! - `txn.rs`        （T42）事务与乐观锁
//! - `rollback.rs`   （T44）回滚引擎
//! - `changelog.rs`  （T46）变更日志
//! - `export_script.rs`（T47）独立回滚脚本

pub mod guard;
pub mod model;
pub mod store;

// T40 验收：round-trip 测试、保留策略、schema 版本检查 —— 已在 store.rs 内。

// T45 验收：
// ① 编译期：guard::mark_locked 对 SystemHook 强制 Locked（scanners/builder.rs 调用）
// ② 运行期：guard::check 对 Locked 项返回 Denied
// ③ 单测覆盖：见 guard.rs tests（含全部 item 类型）