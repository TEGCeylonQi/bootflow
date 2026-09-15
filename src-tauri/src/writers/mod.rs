//! 写原语 —— 按来源分文件，每个文件实现「一种写机制」。
//!
//! v0.2.0 开口范围（§3.5.1）：
//! - `approved.rs`  启动文件夹 / Run / RunOnce 启停（写 StartupApproved，不删实体）
//! - `task.rs`      计划任务两级开关（任务级 / 触发器级）【T41b】
//! - `service.rs`   服务启动类型（仅 AUTO↔DEMAND）【T41c】
//!
//! 每处入口第一条语句必须是 `guard::check(...)`——这由 T45 护栏保证，
//! 不允许任何 writer 绕过。

pub mod approved;
pub mod service;
pub mod task;