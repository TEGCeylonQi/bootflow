// T40 阶段性标记：模块已实现并单测全绿，但尚未接入 commands 层（T41+ 才接线）。
// 接入后删除本 allow，让 dead_code 警告恢复可见。
#![allow(dead_code)]

//! 快照存储 —— 把 `Snapshot` 写到 `%APPDATA%\BootFlow\snapshots\` 并读回。
//!
//! 硬性要求：
//! 1. **原子写** —— 先写临时文件再 `rename`，程序中断也不会留下半截 JSON。
//! 2. **写完能读回** —— 保存后立即读回，与内存值逐字节比对（round-trip）。
//! 3. **保留策略** —— 保留最近 N 份，旧快照按正文里的 `created_at` 清理。
//! 4. **损坏/不兼容快照不致命** —— `load` 返回 `Err`，调用方按需忽略（扫描继续）。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};
use crate::snapshot::model::{Snapshot, SnapshotReason, SNAPSHOT_SCHEMA_VERSION};

/// 快照目录名（`%APPDATA%\BootFlow\snapshots`）。
pub const SNAPSHOT_DIR_NAME: &str = "snapshots";

/// `%APPDATA%\BootFlow\snapshots`
pub fn snapshot_dir() -> Result<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .ok_or_else(|| AppError::Other("缺少 %APPDATA% 环境变量".into()))?;
    Ok(Path::new(&base).join("BootFlow").join(SNAPSHOT_DIR_NAME))
}

/// 默认保留最近 20 份快照。
pub fn default_retention() -> usize {
    20
}

/// 写入一份快照（原子写 + 立即读回校验）。
pub fn save(snapshot: &Snapshot, retention: usize) -> Result<PathBuf> {
    let dir = snapshot_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::Other(format!("无法创建快照目录 {}：{e}", dir.display())))?;

    let json = serde_json::to_vec_pretty(snapshot)
        .map_err(|e| AppError::Other(format!("序列化快照失败：{e}")))?;
    let final_path = dir.join(format!("{}.json", snapshot.id));
    let tmp_path = dir.join(format!(".{}.tmp", snapshot.id));

    // 原子写：先写临时文件，再 rename 到最终名（同目录 rename 原子）。
    {
        let mut f = fs::File::create(&tmp_path)
            .map_err(|e| AppError::Other(format!("创建临时快照文件失败：{e}")))?;
        f.write_all(&json)
            .map_err(|e| AppError::Other(format!("写入临时快照失败：{e}")))?;
    }
    fs::rename(&tmp_path, &final_path)
        .map_err(|e| AppError::Other(format!("快照落盘失败（{} → {}）：{e}", tmp_path.display(), final_path.display())))?;

    // 写完立即读回，逐字节比对（round-trip）。
    let disk = fs::read(&final_path)
        .map_err(|e| AppError::Other(format!("快照读回失败：{e}")))?;
    if disk != json {
        return Err(AppError::Other(format!(
            "快照 round-trip 不一致：{} 写入后读回不同",
            final_path.display()
        )));
    }

    // 保留策略：清理超出部分的旧快照。
    prune_old(&dir, retention)?;

    Ok(final_path)
}

/// 读取一份快照（按 id，不含 `.json` 后缀）。
///
/// 重名覆盖时不参与干流程；此函数只负责读取并做 schema 版本检查。
pub fn load(id: &str) -> Result<Snapshot> {
    let dir = snapshot_dir()?;
    let path = dir.join(format!("{id}.json"));
    let bytes = fs::read(&path)
        .map_err(|e| AppError::Other(format!("读取快照 {id} 失败：{e}")))?;
    let snap: Snapshot = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Other(format!("解析快照 {id} 失败：{e}")))?;
    if snap.schema_version != SNAPSHOT_SCHEMA_VERSION {
        return Err(AppError::Other(format!(
            "快照 {id} 的 schema 版本 {} 与当前 {} 不兼容",
            snap.schema_version, SNAPSHOT_SCHEMA_VERSION
        )));
    }
    Ok(snap)
}

/// 列出全部快照 id（按 `created_at` 倒序，新的在前）。
///
/// 判新旧**不能**看文件 mtime：Windows 上文件时间戳的精度约 15.6ms，
/// 连续保存多份快照时（机器繁忙时更明显）它们的 mtime 会**完全相同**，
/// 排序随之退化成目录枚举顺序——那是不确定的，保留策略会删错，
/// 实测出现过「该删的没删、不该删的被删」。
///
/// 快照正文里的 `created_at` 是我们自己写进去的权威时间，始终可靠；
/// 文件名是纯 UUID（哈希），本身不含任何时间信息，用不上。
pub fn list() -> Result<Vec<String>> {
    let dir = snapshot_dir()?;
    // (id, created_at, mtime 兜底)
    let mut entries: Vec<(String, String, std::time::SystemTime)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json") && !name.starts_with('.') {
                let mtime = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                // 读不出来或不是快照（损坏文件）时 created_at 留空，排序时沉到最后。
                let created_at = fs::read(e.path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Snapshot>(&b).ok())
                    .map(|s| s.created_at)
                    .unwrap_or_default();
                entries.push((
                    name.trim_end_matches(".json").to_string(),
                    created_at,
                    mtime,
                ));
            }
        }
    }
    // `created_at` 是 RFC3339（统一 UTC，无时区歧义），字典序即时间序。
    // 只有两边都读不出 created_at 时才退回 mtime——这时也只能尽力而为。
    entries.sort_by(|a, b| match (a.1.is_empty(), b.1.is_empty()) {
        (false, false) => b.1.cmp(&a.1),
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        (true, true) => b.2.cmp(&a.2),
    });
    Ok(entries.into_iter().map(|(id, _, _)| id).collect())
}

/// 删除某份快照（回滚/清理用）。
pub fn delete(id: &str) -> Result<()> {
    let dir = snapshot_dir()?;
    let path = dir.join(format!("{id}.json"));
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| AppError::Other(format!("删除快照 {id} 失败：{e}")))?;
    }
    Ok(())
}

/// 保留最近 `retention` 份快照，删除更旧的。忽略非快照文件与同名 `.tmp`。
fn prune_old(_dir: &Path, retention: usize) -> Result<()> {
    if retention == 0 {
        return Ok(());
    }
    let all = list()?;
    if all.len() <= retention {
        return Ok(());
    }
    for id in &all[retention..] {
        let _ = delete(id); // 失败不阻断，下次再清
    }
    Ok(())
}

/// 从一次扫描构建"改前基线"快照（v0.2.0 首个快照入口）。
///
/// `item_records`：由调用方（commands.rs）把 `StartupItem` 列表转成
/// `SnapshotRecord` 后传入。此函数只负责打包元数据与落盘。
pub fn build_scan_baseline(item_records: Vec<crate::snapshot::model::SnapshotRecord>) -> Result<PathBuf> {
    let now = chrono::Utc::now().to_rfc3339();
    let snap = Snapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        id: crate::snapshot::model::snapshot_id("BootFlow", &now),
        created_at: now,
        description: "扫描前基线".to_string(),
        reason: SnapshotReason::Scan,
        records: item_records,
    };
    save(&snap, default_retention())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::model::{SnapshotRecord, SnapshotTarget};

    /// 环境变量 `APPDATA` 是进程级全局；并行测试都改它会互相踩踏。
    /// 用**全局共享**的那把锁（见 `crate::testenv`）——按模块各持一把等于没锁，
    /// 因为 `snapshot::changelog` 也在改同一个 `APPDATA`。
    fn with_temp_appdata<F: FnOnce()>(test_name: &str, f: F) {
        let _guard = crate::testenv::lock();
        let dir = std::env::temp_dir().join(format!(
            "bootflow-{}-{}",
            test_name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("APPDATA", &dir);
        f();
    }

    fn sample() -> Snapshot {
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: "test-1".into(),
            created_at: "2026-09-15T00:00:00Z".into(),
            description: "测试快照".into(),
            reason: SnapshotReason::Scan,
            records: vec![SnapshotRecord {
                id: "item-1".into(),
                display_name: "测试应用".into(),
                target: SnapshotTarget {
                    source: "RunUser".into(),
                    location: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run".into(),
                    value_name: Some("TestApp".into()),
                },
                approved_raw: Some("0202000000000000a0b0c0d0e0f0".into()),
                task_enabled: None,
                trigger_enabled: None,
                service_start_type: None,
                scope: "user".into(),
                risk: "Safe".into(),
            }],
        }
    }

    #[test]
    fn save_then_load_round_trips_byte_for_byte() {
        with_temp_appdata("roundtrip", || {
            let snap = sample();
            let path = save(&snap, 5).unwrap();
            assert!(path.exists());

            let loaded = load("test-1").unwrap();
            assert_eq!(loaded.schema_version, SNAPSHOT_SCHEMA_VERSION);
            assert_eq!(loaded.records.len(), 1);
            assert_eq!(
                loaded.records[0].approved_raw.as_deref(),
                Some("0202000000000000a0b0c0d0e0f0")
            );

            // 逐字节：磁盘与内存序列化结果一致（round-trip）
            let on_disk = fs::read(&path).unwrap();
            let re_serialized = serde_json::to_vec_pretty(&snap).unwrap();
            assert_eq!(on_disk, re_serialized);
        });
    }

    #[test]
    fn retention_prunes_oldest_snapshots() {
        with_temp_appdata("retention", || {
            for i in 0..5u32 {
                let mut snap = sample();
                snap.id = format!("ret-{i}");
                snap.created_at = format!("2026-09-15T00:0{i}:00Z");
                save(&snap, 3).unwrap();
            }
            let ids = list().unwrap();
            assert!(ids.len() <= 3, "应保留最多 3 份，实际 {}", ids.len());
            assert!(!ids.contains(&"ret-0".to_string()), "ret-0 应被清理");
            assert!(!ids.contains(&"ret-1".to_string()), "ret-1 应被清理");
            // ret-4 / ret-3 / ret-2 至少留下
            assert!(ids.contains(&"ret-4".to_string()));
            assert!(ids.contains(&"ret-3".to_string()));
            assert!(ids.contains(&"ret-2".to_string()));
        });
    }

    /// `list()` 必须按快照正文里的 `created_at` 判新旧，**不能**按写入先后（=文件 mtime）。
    ///
    /// 特意逆序写入：先写时间最晚的，最后写时间最早的。
    /// 若实现退回按 mtime 排序，顺序会整个反过来，这条就会红。
    #[test]
    fn list_orders_by_created_at_not_by_write_order() {
        with_temp_appdata("order", || {
            for (id, ts) in [
                ("ord-new", "2026-09-15T00:03:00Z"),
                ("ord-mid", "2026-09-15T00:02:00Z"),
                ("ord-old", "2026-09-15T00:01:00Z"),
            ] {
                let mut snap = sample();
                snap.id = id.into();
                snap.created_at = ts.into();
                save(&snap, 10).unwrap(); // retention 足够大，不触发清理
            }

            let ids = list().unwrap();
            assert_eq!(
                ids,
                vec!["ord-new", "ord-mid", "ord-old"],
                "应按 created_at 倒序，而不是按写入先后"
            );
        });
    }

    /// 保留策略在「写入顺序与时间顺序相反」时同样要删对。
    ///
    /// 这是踩过的坑：Windows 文件 mtime 精度约 15.6ms，连续写多份快照
    /// （尤其机器繁忙时）它们的 mtime 会完全相同，排序随之退化，
    /// 旧实现会删错——留下旧的、删掉新的。
    #[test]
    fn retention_keeps_newest_even_when_written_in_reverse() {
        with_temp_appdata("retrev", || {
            // i 从 4 递减到 0：最后写入的是 created_at 最早的 rr-0。
            for i in (0..5u32).rev() {
                let mut snap = sample();
                snap.id = format!("rr-{i}");
                snap.created_at = format!("2026-09-15T00:0{i}:00Z");
                save(&snap, 3).unwrap();
            }

            let ids = list().unwrap();
            assert_eq!(ids.len(), 3, "应保留 3 份，实际 {}", ids.len());
            assert!(ids.contains(&"rr-4".to_string()), "最新的一份必须留下");
            assert!(ids.contains(&"rr-3".to_string()));
            assert!(ids.contains(&"rr-2".to_string()));
            assert!(!ids.contains(&"rr-0".to_string()), "最旧的一份必须被清理");
        });
    }

    /// 损坏/非快照的 `.json` 不能让 `list()` 崩，也不该顶掉正常快照。
    #[test]
    fn list_tolerates_corrupt_json_and_sorts_it_last() {
        with_temp_appdata("corrupt", || {
            let mut snap = sample();
            snap.id = "good".into();
            snap.created_at = "2026-09-15T00:00:00Z".into();
            save(&snap, 10).unwrap();

            let snapdir = snapshot_dir().unwrap();
            fs::write(snapdir.join("broken.json"), b"{ this is not json").unwrap();

            let ids = list().unwrap();
            assert!(ids.contains(&"good".to_string()), "正常快照仍应列出");
            assert_eq!(
                ids.last().map(String::as_str),
                Some("broken"),
                "读不出 created_at 的文件应沉到最后"
            );
        });
    }

    #[test]
    fn rejects_future_schema_version() {
        with_temp_appdata("schema", || {
            let mut snap = sample();
            snap.id = "future".into();
            snap.schema_version = SNAPSHOT_SCHEMA_VERSION + 1; // 模拟未来版本快照
            let snapdir = snapshot_dir().unwrap();
            fs::create_dir_all(&snapdir).unwrap();
            fs::write(snapdir.join("future.json"), serde_json::to_vec(&snap).unwrap()).unwrap();

            let err = load("future").unwrap_err();
            assert!(
                err.to_string().contains("不兼容"),
                "应以不兼容拒绝，实际：{err}"
            );
        });
    }

    #[test]
    fn delete_removes_snapshot_file() {
        with_temp_appdata("delete", || {
            // 手动放一份
            let snapdir = snapshot_dir().unwrap();
            fs::create_dir_all(&snapdir).unwrap();
            fs::write(snapdir.join("gone.json"), b"{}").unwrap();

            delete("gone").unwrap();
            assert!(!snapdir.join("gone.json").exists());
        });
    }
}