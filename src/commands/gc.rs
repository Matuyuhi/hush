//! `hush gc` — 保存済み expand アーティファクトの掃除。
//!
//! 引数なし: 現在の使用量を表示。
//! `--days N`: N 日より古いアーティファクト（原文＋メタ）を削除。

use std::fs;
use std::thread;
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::sandbox;
use crate::store::Store;
use crate::ui::{self, Row};

pub fn run(days: Option<u64>) -> Result<i32> {
    // 子プロセスを起動しないので起動直後にゲート。
    sandbox::gate()?;

    let store = Store::open()?;
    let dir = store.objects_dir();

    let mut total_bytes: u64 = 0;
    let mut artifact_count: u64 = 0;
    let mut old_ids: Vec<String> = Vec::new();
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
                old_ids.push(name.to_string());
            }
        }
    }

    let mut rows = vec![Row::Center("hush gc".to_string()), Row::Rule];
    match days {
        None => {
            rows.push(Row::Line(format!(
                "  {artifact_count} artifacts, ~{} KiB",
                total_bytes / 1024
            )));
            rows.push(Row::Line(
                "  remove old ones with: hush gc --days <N>".to_string(),
            ));
        }
        Some(d) => {
            let removed = old_ids.len() as u64;
            if !old_ids.is_empty() {
                let num_threads = thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4);
                let chunk_size = old_ids.len().div_ceil(num_threads);

                let dir_ref = dir.as_path();
                thread::scope(|s| {
                    for chunk in old_ids.chunks(chunk_size) {
                        s.spawn(move || {
                            for id in chunk {
                                let obj = dir_ref.join(id);
                                let meta_path = dir_ref.join(format!("{id}.json"));
                                let _ = fs::remove_file(&obj);
                                let _ = fs::remove_file(&meta_path);
                            }
                        });
                    }
                });
            }

            rows.push(Row::Line(format!(
                "  removed {removed} artifact(s) older than {d} day(s)"
            )));
        }
    }

    println!();
    ui::render(&rows);
    Ok(0)
}
