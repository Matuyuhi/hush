//! 実コマンドの実行とパイプライン。
//!
//! runner が子プロセスを起動して出力を取得し、pipeline が
//! 「実行 → 非送信ゲート → フィルタ → 出力」の順序を強制する。

pub mod pipeline;
pub mod runner;
