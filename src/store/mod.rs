//! expand アーティファクトのストア。
//!
//! 圧縮で原文の一部を削った場合、原文を `~/.local/share/hush/objects/` に保存し、
//! `hush expand <id>` で取り出せるようにする。これによりモデルが情報不足で
//! コマンドを再実行する事態を防ぐ（遅延展開）。
//!
//! レイアウト:
//!   objects/<id>        原文バイト列
//!   objects/<id>.json   メタデータ

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths;

mod id;

#[derive(Serialize, Deserialize, Debug)]
pub struct Meta {
    pub schema_version: u32,
    pub id: String,
    pub command: Vec<String>,
    pub cwd: String,
    pub created_unix: u64,
    pub exit_code: i32,
    pub byte_len: usize,
    pub line_count: usize,
    pub filter: String,
}

pub struct Store {
    objects: PathBuf,
}

impl Store {
    /// ストアを開く（ディレクトリが無ければ作成）。
    pub fn open() -> Result<Self> {
        let objects = paths::objects_dir()?;
        fs::create_dir_all(&objects)
            .map_err(|e| Error::Store(format!("{} を作成できません: {e}", objects.display())))?;
        Ok(Store { objects })
    }

    /// 原文を保存して ID を返す。同一内容が既にあれば書き込みをスキップ（dedup）。
    pub fn put(
        &self,
        original: &[u8],
        command: &[String],
        cwd: &str,
        exit_code: i32,
        filter: &str,
        line_count: usize,
    ) -> Result<String> {
        let id = id::content_id(original);

        // content-addressed なので「既存 = 同一内容」。create_new(O_EXCL) で原子的に
        // 作成し、AlreadyExists は dedup として成功扱い（TOCTOU 回避）。原文とメタを
        // 独立に書くので、片方だけ欠けた状態（クラッシュ等）も次回 put で自己修復される。
        write_new(&self.objects.join(&id), original)
            .map_err(|e| Error::Store(format!("原文を書き込めません: {e}")))?;

        let meta = Meta {
            schema_version: 1,
            id: id.clone(),
            command: command.to_vec(),
            cwd: cwd.to_string(),
            created_unix: now_unix(),
            exit_code,
            byte_len: original.len(),
            line_count,
            filter: filter.to_string(),
        };
        let json = serde_json::to_vec_pretty(&meta)?;
        write_new(&self.objects.join(format!("{id}.json")), &json)
            .map_err(|e| Error::Store(format!("メタデータを書き込めません: {e}")))?;

        Ok(id)
    }

    /// ID から原文を取り出す。
    pub fn get(&self, id: &str) -> Result<Vec<u8>> {
        validate_id(id)?;
        let obj = self.objects.join(id);
        match fs::read(&obj) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound(format!("ID {id} の原文が見つかりません")))
            }
            Err(e) => Err(Error::Store(format!("原文を読み込めません: {e}"))),
        }
    }

    /// objects ディレクトリ。
    pub fn objects_dir(&self) -> &PathBuf {
        &self.objects
    }
}

/// パストラバーサル防止。ID は英数字のみ（content_id は hex なので満たす）。
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::NotFound(format!("不正な ID 形式: {id:?}")));
    }
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// O_EXCL で新規作成して書き込む。既存(AlreadyExists)は dedup として成功扱い。
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => f.write_all(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}
