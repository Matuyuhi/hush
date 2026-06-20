//! tree-sitter による AST シグネチャ抽出（feature = "ast"）。
//!
//! 拡張子で言語を選び、共通のウォーカで「定義ノードのヘッダ行」だけを取り出す。
//! 関数本体や構造体フィールドの中身は捨て、シグネチャだけを残す（原文は expand へ）。
//! 言語追加は `spec_for` に 1 エントリ足すだけ。

use std::path::Path;

use tree_sitter::{Language, Node, Parser};

use crate::error::{Error, Result};
use crate::filters::FilterOutput;

/// 言語ごとの抽出仕様。
struct Spec {
    language: Language,
    /// 1 行シグネチャとして出すノード種別。
    defs: &'static [&'static str],
    /// ヘッダを出しつつ body フィールドへ再帰するノード種別（class / impl 等）。
    containers: &'static [&'static str],
    /// 自身は出さず子へ素通りするラッパ（export / 装飾子等）。
    transparent: &'static [&'static str],
}

fn spec_for(ext: &str) -> Option<Spec> {
    match ext {
        "rs" => Some(Spec {
            language: tree_sitter_rust::LANGUAGE.into(),
            defs: &[
                "function_item",
                "struct_item",
                "enum_item",
                "union_item",
                "const_item",
                "static_item",
                "type_item",
                "macro_definition",
            ],
            containers: &["impl_item", "trait_item", "mod_item"],
            transparent: &[],
        }),
        "py" => Some(Spec {
            language: tree_sitter_python::LANGUAGE.into(),
            defs: &["function_definition"],
            containers: &["class_definition"],
            transparent: &["decorated_definition"],
        }),
        "go" => Some(Spec {
            language: tree_sitter_go::LANGUAGE.into(),
            defs: &[
                "function_declaration",
                "method_declaration",
                "type_declaration",
                "const_declaration",
                "var_declaration",
            ],
            containers: &[],
            transparent: &[],
        }),
        "ts" | "js" | "mjs" | "cjs" | "tsx" | "jsx" => {
            let language = if ext == "tsx" || ext == "jsx" {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            };
            Some(Spec {
                language,
                defs: &[
                    "function_declaration",
                    "function_signature",
                    "method_definition",
                    "method_signature",
                    "abstract_method_signature",
                    "type_alias_declaration",
                    "enum_declaration",
                    "lexical_declaration",
                    "variable_declaration",
                ],
                containers: &[
                    "class_declaration",
                    "abstract_class_declaration",
                    "interface_declaration",
                    "internal_module",
                    "module",
                ],
                transparent: &["export_statement", "ambient_declaration"],
            })
        }
        _ => None,
    }
}

/// 拡張子に応じてシグネチャを抽出する。
pub fn signatures(path: &Path, src: &[u8]) -> Result<FilterOutput> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let spec = spec_for(&ext).ok_or_else(|| {
        Error::Filter(format!(
            "--signatures: unsupported file type {ext:?} (supported: rs, py, go, ts/tsx/js/jsx)"
        ))
    })?;

    let mut parser = Parser::new();
    parser
        .set_language(&spec.language)
        .map_err(|e| Error::Filter(format!("failed to set tree-sitter language: {e}")))?;
    let tree = parser
        .parse(src, None)
        .ok_or_else(|| Error::Filter("failed to parse source".into()))?;

    let mut out: Vec<String> = Vec::new();
    walk(tree.root_node(), src, 0, &spec, &mut out);

    let orig_lines = String::from_utf8_lossy(src).lines().count();
    let shown_lines = out.len();
    let compact = if out.is_empty() {
        "(no signatures found)".to_string()
    } else {
        out.join("\n")
    };

    Ok(FilterOutput {
        filter_name: "read-sig",
        compact,
        original: Some(src.to_vec()),
        orig_lines,
        shown_lines,
    })
}

fn walk(node: Node, src: &[u8], depth: usize, spec: &Spec, out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let kind = child.kind();
        if spec.transparent.contains(&kind) {
            walk(child, src, depth, spec, out);
        } else if spec.containers.contains(&kind) {
            out.push(line(depth, header(child, src)));
            if let Some(body) = child.child_by_field_name("body") {
                walk(body, src, depth + 1, spec, out);
            }
        } else if spec.defs.contains(&kind) {
            out.push(line(depth, header(child, src)));
        }
    }
}

/// 定義のヘッダ（body フィールドがあれば本体直前まで、無ければ先頭行）を 1 行化。
fn header(node: Node, src: &[u8]) -> String {
    let raw = if let Some(body) = node.child_by_field_name("body") {
        slice(src, node.start_byte(), body.start_byte())
    } else {
        let text = node_text(node, src);
        text.split('\n').next().unwrap_or(text)
    };
    oneline(raw)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sigs(name: &str, src: &str) -> String {
        signatures(Path::new(name), src.as_bytes()).unwrap().compact
    }

    #[test]
    fn rust_signatures() {
        let out = sigs(
            "x.rs",
            "pub fn foo(a: i32) -> i32 { a }\nstruct S { x: i32 }\nimpl S { fn m(&self) {} }\n",
        );
        assert!(out.contains("pub fn foo(a: i32) -> i32"));
        assert!(out.contains("struct S"));
        assert!(out.contains("impl S"));
        assert!(out.contains("fn m(&self)"));
        assert!(!out.contains("a }")); // 本体は出ない
    }

    #[test]
    fn python_signatures() {
        let out = sigs(
            "x.py",
            "def foo(a, b):\n    return a + b\n\nclass C:\n    def m(self):\n        pass\n",
        );
        assert!(out.contains("def foo(a, b):"));
        assert!(out.contains("class C:"));
        assert!(out.contains("def m(self):"));
        assert!(!out.contains("return a + b"));
    }

    #[test]
    fn go_signatures() {
        let out = sigs(
            "x.go",
            "package main\nfunc Foo(a int) int { return a }\ntype S struct { X int }\n",
        );
        assert!(out.contains("func Foo(a int) int"));
        assert!(out.contains("type S struct"));
        assert!(!out.contains("return a"));
    }

    #[test]
    fn typescript_signatures() {
        let out = sigs(
            "x.ts",
            "export function foo(a: number): number { return a; }\nexport class C { m(): void {} }\ninterface I { x: number; }\n",
        );
        assert!(out.contains("function foo(a: number): number"));
        assert!(out.contains("class C"));
        assert!(out.contains("interface I"));
    }

    #[test]
    fn unsupported_extension_errors() {
        let err = signatures(Path::new("x.txt"), b"hello").unwrap_err();
        assert!(format!("{err}").contains("unsupported file type"));
    }
}
