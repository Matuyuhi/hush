//! content-addressed な安定 ID。
//!
//! 原文の blake3 ハッシュ先頭 12 hex 文字（48bit）を ID とする。
//! 同一出力は同一 ID になるため自然に dedup される。個人利用の規模では
//! 48bit で衝突はほぼ起きない。

/// 原文バイト列から安定 ID を生成する。
pub fn content_id(bytes: &[u8]) -> String {
    let hash = blake3::hash(bytes);
    hash.to_hex().as_str()[..12].to_string()
}
