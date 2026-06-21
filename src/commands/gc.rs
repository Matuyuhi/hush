//! `hush gc` — 保存済み expand アーティファクトの掃除。
//!
//! 引数なし: 現在の使用量を表示。
//! `--days N`: N 日より古いアーティファクト（原文＋メタ）を削除。
//! `--days N --dry-run`: 削除せず、削除対象の一覧と合計サイズだけを表示する。

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::sandbox;
use crate::store::Store;
use crate::ui::{self, Row};

/// 削除候補となる古いアーティファクト 1 件。
struct OldArtifact {
    id: String,
    size: u64,
    age_days: u64,
}

/// objects ディレクトリを走査した結果。
struct Scan {
    total_bytes: u64,
    artifact_count: u64,
    /// `days` 指定時のみ埋まる、`days` 日より古い原文アーティファクト。
    old: Vec<OldArtifact>,
}

/// objects ディレクトリのエントリを分類する。`days` を渡すと、`now` を基準に
/// それより古い原文アーティファクト（拡張子なしファイル）を `old` に集める。
/// `now` を引数に取るのは、テストで経過日数を決定的に検証できるようにするため。
fn classify(read: fs::ReadDir, days: Option<u64>, now: SystemTime) -> Result<Scan> {
    let mut total_bytes: u64 = 0;
    let mut artifact_count: u64 = 0;
    let mut old: Vec<OldArtifact> = Vec::new();

    for entry in read {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        total_bytes += meta.len();
        let path = entry.path();
        // 原文ファイル（拡張子なし）だけを「アーティファクト」として数える。
        if path.extension().is_none() {
            artifact_count += 1;
            if let Some(days) = days
                && let Ok(modified) = meta.modified()
                && let Ok(age) = now.duration_since(modified)
                && age > Duration::from_secs(days.saturating_mul(86_400))
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                old.push(OldArtifact {
                    id: name.to_string(),
                    size: meta.len(),
                    age_days: age.as_secs() / 86_400,
                });
            }
        }
    }

    Ok(Scan {
        total_bytes,
        artifact_count,
        old,
    })
}

/// 古いアーティファクト（原文＋メタ）を削除し、削除件数を返す。
fn remove_old(dir: &Path, old: &[OldArtifact]) -> u64 {
    let mut removed = 0u64;
    for a in old {
        let obj = dir.join(&a.id);
        let meta_path = dir.join(format!("{}.json", a.id));
        let _ = fs::remove_file(&obj);
        let _ = fs::remove_file(&meta_path);
        removed += 1;
    }
    removed
}

pub fn run(days: Option<u64>, dry_run: bool) -> Result<i32> {
    // 子プロセスを起動しないので起動直後にゲート。
    sandbox::gate()?;

    let store = Store::open()?;
    let dir = store.objects_dir();
    let now = SystemTime::now();

    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!();
            ui::render(&[
                Row::Center("hush gc".to_string()),
                Row::Rule,
                Row::Line("  no artifacts yet".to_string()),
            ]);
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };

    let scan = classify(read, days, now)?;

    let mut rows = vec![Row::Center("hush gc".to_string()), Row::Rule];
    match days {
        None => {
            rows.push(Row::Line(format!(
                "  {} artifacts, ~{} KiB",
                scan.artifact_count,
                scan.total_bytes / 1024
            )));
            rows.push(Row::Line(
                "  remove old ones with: hush gc --days <N> (add --dry-run to preview)".to_string(),
            ));
        }
        Some(d) if dry_run => {
            if scan.old.is_empty() {
                rows.push(Row::Line(format!("  no artifacts older than {d} day(s)")));
            } else {
                let mut total_old: u64 = 0;
                for a in &scan.old {
                    total_old += a.size;
                    // ASCII のみ（id 12 桁 / 経過日数 / 人間可読サイズ）。
                    rows.push(Row::Line(format!(
                        "  {:<12}  {:>4}d  {:>8}",
                        a.id,
                        a.age_days,
                        ui::human_bytes(a.size)
                    )));
                }
                rows.push(Row::Line(format!(
                    "  would remove {} artifact(s) (~{}) older than {d} day(s)",
                    scan.old.len(),
                    ui::human_bytes(total_old)
                )));
            }
        }
        Some(d) => {
            let removed = remove_old(dir, &scan.old);
            rows.push(Row::Line(format!(
                "  removed {removed} artifact(s) older than {d} day(s)"
            )));
        }
    }

    println!();
    ui::render(&rows);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 原文（拡張子なし）+ メタ（.json）を 1 組書く。
    fn write_artifact(dir: &Path, id: &str, body: &[u8]) {
        let mut f = fs::File::create(dir.join(id)).unwrap();
        f.write_all(body).unwrap();
        let mut m = fs::File::create(dir.join(format!("{id}.json"))).unwrap();
        m.write_all(b"{}").unwrap();
    }

    #[test]
    fn classify_counts_artifacts_and_excludes_json() {
        let dir = tempfile::tempdir().unwrap();
        write_artifact(dir.path(), "aaaaaaaaaaaa", b"hello");
        write_artifact(dir.path(), "bbbbbbbbbbbb", b"world!!");

        // days=None: old は空、artifact_count は原文だけ（.json は除外）。
        let read = fs::read_dir(dir.path()).unwrap();
        let scan = classify(read, None, SystemTime::now()).unwrap();
        assert_eq!(scan.artifact_count, 2);
        assert!(scan.old.is_empty());
        // total_bytes は原文 + メタの全ファイルを含む。
        assert!(scan.total_bytes >= (b"hello".len() + b"world!!".len()) as u64);
    }

    #[test]
    fn classify_selects_old_artifacts_only() {
        let dir = tempfile::tempdir().unwrap();
        write_artifact(dir.path(), "aaaaaaaaaaaa", b"hello");
        write_artifact(dir.path(), "bbbbbbbbbbbb", b"world!!");

        // now を十分未来にすると、全アーティファクトが「古い」と判定される。
        let future = SystemTime::now() + Duration::from_secs(100 * 86_400);
        let read = fs::read_dir(dir.path()).unwrap();
        let scan = classify(read, Some(1), future).unwrap();

        assert_eq!(scan.old.len(), 2);
        // .json はアーティファクトに含まれない。
        assert!(scan.old.iter().all(|a| !a.id.ends_with(".json")));
        let total: u64 = scan.old.iter().map(|a| a.size).sum();
        assert_eq!(total, (b"hello".len() + b"world!!".len()) as u64);
    }

    #[test]
    fn classify_excludes_recent_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        write_artifact(dir.path(), "aaaaaaaaaaaa", b"hello");

        // 10 年より古いものだけ対象 -> 直近のファイルは選ばれない。
        let read = fs::read_dir(dir.path()).unwrap();
        let scan = classify(read, Some(3650), SystemTime::now()).unwrap();
        assert!(scan.old.is_empty());
    }

    #[test]
    fn remove_old_deletes_original_and_meta() {
        let dir = tempfile::tempdir().unwrap();
        write_artifact(dir.path(), "aaaaaaaaaaaa", b"hello");
        let old = vec![OldArtifact {
            id: "aaaaaaaaaaaa".to_string(),
            size: 5,
            age_days: 100,
        }];

        let removed = remove_old(dir.path(), &old);
        assert_eq!(removed, 1);
        assert!(!dir.path().join("aaaaaaaaaaaa").exists());
        assert!(!dir.path().join("aaaaaaaaaaaa.json").exists());
    }
}
