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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_id_returns_12_chars() {
        let id = content_id(b"hello world");
        assert_eq!(id.len(), 12);
    }

    #[test]
    fn content_id_is_deterministic() {
        let id1 = content_id(b"deterministic");
        let id2 = content_id(b"deterministic");
        assert_eq!(id1, id2);
    }

    #[test]
    fn content_id_differs_for_different_inputs() {
        let id1 = content_id(b"input 1");
        let id2 = content_id(b"input 2");
        assert_ne!(id1, id2);
    }

    #[test]
    fn content_id_is_lowercase_hex() {
        let id = content_id(b"hex test");
        assert!(id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn content_id_empty_input() {
        let id = content_id(b"");
        assert_eq!(id.len(), 12);
        assert!(id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }
}
