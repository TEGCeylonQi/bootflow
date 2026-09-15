// T40 阶段性标记：模块已实现并单测全绿，但尚未接入 commands 层（T41+ 才接线）。
// 接入后删除本 allow，让 dead_code 警告恢复可见。
#![allow(dead_code)]

//! 快照存储 —— 把 `Snapshot` 写到 `%APPDATA%\BootFlow\snapshots\` 并读回。
//!
//! 硬性要求：
//! 1. **原子写** —— 先写临时文件再 `rename`，程序中断也不会留下半截 JSON。
//! 2. **写完能读回** —— 保存后立即读回，与内存值逐字节比对（round-trip）。
//! 3. **保留策略** —— 保留最近 N 份，旧快照按文件名（含时间前缀）清理。
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

/// 列出全部快照 id（按文件修改时间倒序，新的在前）。
pub fn list() -> Result<Vec<String>> {
    let dir = snapshot_dir()?;
    let mut entries: Vec<(String, std::time::SystemTime)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json") && !name.starts_with('.') {
                let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                entries.push((name.trim_end_matches(".json").to_string(), mtime));
            }
        }
    }
    entries.sort_by_key(|a| std::cmp::Reverse(a.1));
    Ok(entries.into_iter().map(|(id, _)| id).collect())
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
    /// 所有需要改 APPDATA 的测试必须持有这把锁串行执行。
    static APPDATA_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// 在锁内把 `APPDATA` 指到独立的临时目录，返回守卫以保证锁跨整个测试持有。
    fn with_temp_appdata<F: FnOnce()>(test_name: &str, f: F) {
        let _guard = APPDATA_LOCK.lock().unwrap();
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