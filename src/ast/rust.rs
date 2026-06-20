//! tree-sitter-rust によるシグネチャ抽出。
//!
//! 関数本体や struct フィールド等の中身は捨てず、本体は expand へ回し、
//! シグネチャ（fn/struct/enum/trait/impl/mod/const/...）だけを表示する。

use tree_sitter::Node;

use crate::error::{Error, Result};
use crate::filters::FilterOutput;

/// Rust ソースからシグネチャ一覧を抽出する。
pub fn signatures(src: &[u8]) -> Result<FilterOutput> {
    let mut parser = tree_sitter::Parser::new();
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|e| Error::Filter(format!("tree-sitter 言語設定に失敗: {e}")))?;
    let tree = parser
        .parse(src, None)
        .ok_or_else(|| Error::Filter("Rust ソースのパースに失敗しました".into()))?;

    let mut out: Vec<String> = Vec::new();
    walk(tree.root_node(), src, 0, &mut out);

    let orig_lines = String::from_utf8_lossy(src).lines().count();
    let shown_lines = out.len();
    let compact = if out.is_empty() {
        "(シグネチャが見つかりませんでした)".to_string()
    } else {
        out.join("\n")
    };

    Ok(FilterOutput {
        filter_name: "read-sig",
        compact,
        // シグネチャは原文を大きく削るので、全文は常に expand に保存する。
        original: Some(src.to_vec()),
        orig_lines,
        shown_lines,
    })
}

fn walk(node: Node, src: &[u8], depth: usize, out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_item" => out.push(line(depth, with_body_or_whole(child, src, true))),
            "struct_item" | "enum_item" | "union_item" => {
                out.push(line(depth, with_body_or_whole(child, src, true)))
            }
            "const_item" | "static_item" | "type_item" => {
                out.push(line(depth, oneline(node_text(child, src))))
            }
            "macro_definition" => out.push(line(
                depth,
                format!("macro_rules! {}", field(child, src, "name")),
            )),
            "trait_item" | "impl_item" | "mod_item" => {
                out.push(line(depth, with_body_or_whole(child, src, false)));
                if let Some(body) = child.child_by_field_name("body") {
                    walk(body, src, depth + 1, out);
                }
            }
            _ => {}
        }
    }
}

/// body フィールドがあれば本体直前までをシグネチャとして取り、
/// brace=true なら ` { … }` を付ける。body が無ければノード全体。
fn with_body_or_whole(node: Node, src: &[u8], brace: bool) -> String {
    if let Some(body) = node.child_by_field_name("body") {
        let head = oneline(slice(src, node.start_byte(), body.start_byte()));
        if brace {
            format!("{head} {{ … }}")
        } else {
            head
        }
    } else {
        oneline(node_text(node, src))
    }
}

fn line(depth: usize, s: String) -> String {
    format!("{}{}", "  ".repeat(depth), s)
}

fn oneline(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn slice(src: &[u8], start: usize, end: usize) -> &str {
    std::str::from_utf8(&src[start..end]).unwrap_or("")
}

fn node_text<'a>(node: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[node.byte_range()]).unwrap_or("")
}

fn field(node: Node, src: &[u8], name: &str) -> String {
    node.child_by_field_name(name)
        .map(|n| node_text(n, src).to_string())
        .unwrap_or_default()
}
