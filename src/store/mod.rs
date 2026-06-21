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
    /// 原文のバイト数。
    pub byte_len: usize,
    /// 原文の行数。
    pub line_count: usize,
    /// 圧縮後本文のバイト数（フッタ除く）。旧フォーマットには無い。
    #[serde(default)]
    pub compact_bytes: usize,
    /// 圧縮後に表示した行数。旧フォーマットには無い。
    #[serde(default)]
    pub compact_lines: usize,
    pub filter: String,
}

/// `Store::put` に渡すメタ情報（引数過多を避けるためのまとめ）。
pub struct PutMeta<'a> {
    pub command: &'a [String],
    pub cwd: &'a str,
    pub exit_code: i32,
    pub filter: &'a str,
    pub orig_lines: usize,
    pub compact_bytes: usize,
    pub compact_lines: usize,
}

pub struct Store {
    objects: PathBuf,
}

impl Store {
    /// ストアを開く（ディレクトリが無ければ作成）。
    pub fn open() -> Result<Self> {
        let objects = paths::objects_dir()?;
        fs::create_dir_all(&objects)
            .map_err(|e| Error::Store(format!("cannot create {}: {e}", objects.display())))?;
        Ok(Store { objects })
    }

    /// 原文を保存して ID を返す。同一内容が既にあれば書き込みをスキップ（dedup）。
    pub fn put(&self, original: &[u8], m: PutMeta) -> Result<String> {
        let id = id::content_id(original);

        // content-addressed なので「既存 = 同一内容」。create_new(O_EXCL) で原子的に
        // 作成し、AlreadyExists は dedup として成功扱い（TOCTOU 回避）。原文とメタを
        // 独立に書くので、片方だけ欠けた状態（クラッシュ等）も次回 put で自己修復される。
        write_new(&self.objects.join(&id), original)
            .map_err(|e| Error::Store(format!("cannot write original: {e}")))?;

        let meta = Meta {
            schema_version: 2,
            id: id.clone(),
            command: m.command.to_vec(),
            cwd: m.cwd.to_string(),
            created_unix: now_unix(),
            exit_code: m.exit_code,
            byte_len: original.len(),
            line_count: m.orig_lines,
            compact_bytes: m.compact_bytes,
            compact_lines: m.compact_lines,
            filter: m.filter.to_string(),
        };
        let json = serde_json::to_vec_pretty(&meta)?;
        write_new(&self.objects.join(format!("{id}.json")), &json)
            .map_err(|e| Error::Store(format!("cannot write metadata: {e}")))?;

        Ok(id)
    }

    /// ID から原文を取り出す。
    pub fn get(&self, id: &str) -> Result<Vec<u8>> {
        validate_id(id)?;
        let obj = self.objects.join(id);
        match fs::read(&obj) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound(format!("no stored original for id {id}")))
            }
            Err(e) => Err(Error::Store(format!("cannot read original: {e}"))),
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
        return Err(Error::NotFound(format!("invalid id: {id:?}")));
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_id_accepts_alphanumeric_and_rejects_invalid_chars() {
        // Valid cases (alphanumeric, typical hex output)
        assert!(validate_id("abcdef123456").is_ok());
        assert!(validate_id("0123456789").is_ok());
        assert!(validate_id("a").is_ok());
        assert!(validate_id("Z").is_ok());
        assert!(validate_id("A1z9").is_ok());

        // Invalid cases: empty
        assert!(validate_id("").is_err());

        // Invalid cases: path traversal and directory separators
        assert!(validate_id("../123").is_err());
        assert!(validate_id("..").is_err());
        assert!(validate_id(".").is_err());
        assert!(validate_id("abc/def").is_err());
        assert!(validate_id("abc\\def").is_err());

        // Invalid cases: other symbols
        assert!(validate_id("abc.def").is_err());
        assert!(validate_id("abc-def").is_err());
        assert!(validate_id("abc_def").is_err());
        assert!(validate_id("abc def").is_err());
        assert!(validate_id("abc\ndef").is_err());

        // Invalid cases: non-ASCII characters
        assert!(validate_id("あ").is_err());
        assert!(validate_id("abcあ").is_err());
        assert!(validate_id("😊").is_err());
    }

    #[test]
    fn put_creates_file_and_metadata() {
        let dir = tempdir().unwrap();
        let store = Store { objects: dir.path().to_path_buf() };

        let original = b"hello world";
        let command = vec!["echo".to_string(), "hello world".to_string()];
        let meta = PutMeta {
            command: &command,
            cwd: "/tmp",
            exit_code: 0,
            filter: "dummy",
            orig_lines: 1,
            compact_bytes: 11,
            compact_lines: 1,
        };

        let id = store.put(original, meta).unwrap();

        // Original file
        let content = fs::read(dir.path().join(&id)).unwrap();
        assert_eq!(content, original);

        // Metadata file
        let json = fs::read(dir.path().join(format!("{}.json", id))).unwrap();
        let parsed: Meta = serde_json::from_slice(&json).unwrap();
        assert_eq!(parsed.id, id);
        assert_eq!(parsed.filter, "dummy");
        assert_eq!(parsed.command, command);
    }

    #[test]
    fn put_skips_existing() {
        let dir = tempdir().unwrap();
        let store = Store { objects: dir.path().to_path_buf() };

        let original = b"duplicate content";
        let meta = PutMeta {
            command: &["echo".to_string()],
            cwd: "/tmp",
            exit_code: 0,
            filter: "dummy",
            orig_lines: 1,
            compact_bytes: 17,
            compact_lines: 1,
        };

        let id1 = store.put(original, meta).unwrap();

        // Try putting again with the same content
        let meta2 = PutMeta {
            command: &["echo".to_string()],
            cwd: "/tmp",
            exit_code: 0,
            filter: "dummy",
            orig_lines: 1,
            compact_bytes: 17,
            compact_lines: 1,
        };
        let id2 = store.put(original, meta2).unwrap();

        assert_eq!(id1, id2);
    }
}
