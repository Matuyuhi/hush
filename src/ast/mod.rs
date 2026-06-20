//! tree-sitter による AST 解析（feature = "ast"）。
//!
//! 最初は Rust 1 言語のみ。他言語は `rust.rs` と同型のモジュールを足し、
//! 拡張子で振り分ければ増やせる。

pub mod rust;
