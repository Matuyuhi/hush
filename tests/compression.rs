//! 圧縮率の integration テスト兼ベンチ。
//!
//! `tests/fixtures/` の固定サンプルを各フィルタ（`hush::filters::run`）に通し、
//!
//! 1. `meets_minimum_compression` — コマンド別に「最低圧縮率（floor）」を下回ったら
//!    fail させる回帰ガード。新フィルタ追加や既存フィルタ改修で圧縮が劣化したら CI で気づける。
//! 2. `writes_report` — 同じ計測でコマンド別の圧縮率表（markdown）を
//!    `target/compression-report.md` に書き出す。CI が main push では job summary に、
//!    PR ではラベル時に PR コメントとして掲示する。
//!
//! 圧縮率は `stats` と同じく「削減バイト / 原文バイト」。原文 = 生の stdout+stderr、
//! 圧縮後 = フィルタ本文（expand フッタは含めない）。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use hush::filters::{self, FilterInput};
use hush::ui::{self, Row, commas, human_bytes, human_count};

struct Case {
    /// 表示するコマンド名。
    cmd: &'static str,
    /// フィルタ選択に使う argv。
    argv: &'static [&'static str],
    /// stdout に流す fixture（`tests/fixtures/` 相対）。
    stdout: Option<&'static str>,
    /// stderr に流す fixture。
    stderr: Option<&'static str>,
    /// 最低圧縮率（これを下回ると回帰とみなして fail）。実測から約 10pt 下げた緩めの値。
    min_ratio: f64,
}

/// 計測対象。代表的な fixture を 1 コマンド 1 件（cargo build のみ 2 形態）。
/// floor は実測から約 10-15pt 下げた緩めの値。通常変動では落ちず、明確な退行で落ちる。
const CASES: &[Case] = &[
    Case {
        cmd: "git status",
        argv: &["git", "status"],
        stdout: Some("git-status/dirty.stdout"),
        stderr: None,
        min_ratio: 0.20,
    },
    Case {
        cmd: "git diff",
        argv: &["git", "diff"],
        stdout: Some("git-diff/changes.stdout"),
        stderr: None,
        min_ratio: 0.80,
    },
    Case {
        cmd: "git log",
        argv: &["git", "log"],
        stdout: Some("git-log/recent.stdout"),
        stderr: None,
        min_ratio: 0.80,
    },
    Case {
        cmd: "cargo build",
        argv: &["cargo", "build"],
        stdout: None,
        stderr: Some("cargo-build/diagnostics.stderr"),
        min_ratio: 0.40,
    },
    // cargo 自身のエラー（スニペット無し）は原因を保持するのが正しい挙動なので低圧縮。
    // 退行ガードではなく「原因を残す」ことの記録用なので floor は 0。
    Case {
        cmd: "cargo build (cargo err)",
        argv: &["cargo", "build"],
        stdout: None,
        stderr: Some("cargo-build/cargo-own-error.stderr"),
        min_ratio: 0.0,
    },
    Case {
        cmd: "cargo test",
        argv: &["cargo", "test"],
        stdout: Some("cargo-test/pass.stdout"),
        stderr: None,
        min_ratio: 0.70,
    },
    Case {
        cmd: "go test",
        argv: &["go", "test"],
        stdout: Some("test/go-test.stdout"),
        stderr: None,
        min_ratio: 0.75,
    },
    Case {
        cmd: "pytest",
        argv: &["pytest"],
        stdout: Some("test/pytest.stdout"),
        stderr: None,
        min_ratio: 0.30,
    },
    Case {
        cmd: "vitest",
        argv: &["vitest"],
        stdout: Some("test/vitest.stdout"),
        stderr: None,
        min_ratio: 0.55,
    },
    Case {
        cmd: "docker ps",
        argv: &["docker", "ps"],
        stdout: Some("tabular/docker-ps.stdout"),
        stderr: None,
        min_ratio: 0.25,
    },
    Case {
        cmd: "json (kubectl -o json)",
        argv: &["kubectl", "get", "pods", "-o", "json"],
        stdout: Some("json/k8s-pods.json"),
        stderr: None,
        min_ratio: 0.55,
    },
    Case {
        cmd: "json (cargo messages)",
        argv: &["cargo", "build", "--message-format=json"],
        stdout: Some("json/cargo-messages.ndjson"),
        stderr: None,
        min_ratio: 0.55,
    },
    Case {
        cmd: "grep",
        argv: &["grep", "-rn", "fn ", "src"],
        stdout: Some("grep/fn.stdout"),
        stderr: None,
        min_ratio: 0.80,
    },
    Case {
        cmd: "find",
        argv: &["find", "src", "-name", "*.rs"],
        stdout: Some("find/rs.stdout"),
        stderr: None,
        min_ratio: 0.50,
    },
    Case {
        cmd: "ls",
        argv: &["ls", "-la", "/usr/bin"],
        stdout: Some("ls/usr-bin.stdout"),
        stderr: None,
        min_ratio: 0.85,
    },
    Case {
        cmd: "pip list",
        argv: &["pip", "list"],
        stdout: Some("tabular/pip-list.stdout"),
        stderr: None,
        min_ratio: 0.40,
    },
    Case {
        cmd: "git show",
        argv: &["git", "show"],
        stdout: Some("git-show/commit.stdout"),
        stderr: None,
        min_ratio: 0.70,
    },
    Case {
        cmd: "du -a",
        argv: &["du", "-a"],
        stdout: Some("du-tree/du-a.stdout"),
        stderr: None,
        min_ratio: 0.75,
    },
    Case {
        cmd: "tree",
        argv: &["tree"],
        stdout: Some("du-tree/tree.stdout"),
        stderr: None,
        min_ratio: 0.60,
    },
    Case {
        cmd: "npm install",
        argv: &["npm", "install"],
        stdout: Some("pkg-install/npm-install.stdout"),
        stderr: None,
        min_ratio: 0.55,
    },
    Case {
        cmd: "make",
        argv: &["make"],
        stdout: None,
        stderr: Some("make/build.stderr"),
        min_ratio: 0.55,
    },
    Case {
        cmd: "go build",
        argv: &["go", "build"],
        stdout: None,
        stderr: Some("go-build/diagnostics.stderr"),
        min_ratio: 0.10,
    },
    Case {
        cmd: "diff",
        argv: &["diff", "-ru", "old", "new"],
        stdout: Some("diff/changes.stdout"),
        stderr: None,
        min_ratio: 0.80,
    },
    Case {
        cmd: "python (traceback)",
        argv: &["python", "main.py"],
        stdout: Some("py-traceback/deep.stdout"),
        stderr: Some("py-traceback/deep.stderr"),
        min_ratio: 0.30,
    },
    // tsc/eslint は出力の大半が「残すべき診断」なので本質的に低圧縮（honest floor）。
    Case {
        cmd: "tsc",
        argv: &["tsc"],
        stdout: Some("node-check/tsc.stdout"),
        stderr: None,
        min_ratio: 0.03,
    },
    Case {
        cmd: "eslint",
        argv: &["eslint", "."],
        stdout: Some("node-check/eslint.stdout"),
        stderr: None,
        min_ratio: 0.12,
    },
    Case {
        cmd: "build log (passthrough)",
        argv: &["npm", "run", "build"],
        stdout: Some("passthrough/build-log.stdout"),
        stderr: None,
        min_ratio: 0.65,
    },
];

struct Measured {
    cmd: &'static str,
    filter: String,
    orig_bytes: usize,
    compact_bytes: usize,
    orig_lines: usize,
    shown_lines: usize,
    ratio: f64,
    min_ratio: f64,
}

fn read_fixture(rel: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel);
    fs::read(&p).unwrap_or_else(|e| panic!("read fixture {}: {e}", p.display()))
}

fn measure(c: &Case) -> Measured {
    let stdout = c.stdout.map(read_fixture).unwrap_or_default();
    let stderr = c.stderr.map(read_fixture).unwrap_or_default();
    let orig_bytes = stdout.len() + stderr.len();

    let input = FilterInput {
        argv: c.argv.iter().map(|s| s.to_string()).collect(),
        stdout,
        stderr,
    };
    let out = filters::run(&input).expect("filter run");

    let compact_bytes = out.compact.len();
    let ratio = if orig_bytes > 0 {
        orig_bytes.saturating_sub(compact_bytes) as f64 / orig_bytes as f64
    } else {
        0.0
    };
    Measured {
        cmd: c.cmd,
        filter: out.filter_name.to_string(),
        orig_bytes,
        compact_bytes,
        orig_lines: out.orig_lines,
        shown_lines: out.shown_lines,
        ratio,
        min_ratio: c.min_ratio,
    }
}

/// 概算トークン（`hush stats` と同じ 1 token ~= 4 bytes）。
fn approx_tokens(bytes: u64) -> u64 {
    bytes / 4
}

/// `hush stats` と同じ枠付きブロックでレポートを描画する（README/job summary 共通）。
/// 罫線・桁揃えは `ui` を再利用し、見た目を `hush stats` に合わせる。
fn report_block(rows: &[Measured], baseline: Option<&HashMap<String, (u64, u64)>>) -> String {
    let orig_b: u64 = rows.iter().map(|m| m.orig_bytes as u64).sum();
    let comp_b: u64 = rows.iter().map(|m| m.compact_bytes as u64).sum();
    let orig_l: u64 = rows.iter().map(|m| m.orig_lines as u64).sum();
    let comp_l: u64 = rows.iter().map(|m| m.shown_lines as u64).sum();
    let saved_b = orig_b.saturating_sub(comp_b);
    let ratio = if orig_b > 0 {
        100.0 * saved_b as f64 / orig_b as f64
    } else {
        0.0
    };

    // ベースライン（main の filters を同じ fixtures で計測した値）の合計圧縮率。Δ 表示用。
    // 個別 cmd が baseline に無ければ現値で埋めて合計の意味を保つ（通常は全 cmd 揃う）。
    let base_ratio = baseline.map(|b| {
        let (bo, bc) = rows.iter().fold((0u64, 0u64), |(o, c), m| {
            let (mo, mc) = b
                .get(m.cmd)
                .copied()
                .unwrap_or((m.orig_bytes as u64, m.compact_bytes as u64));
            (o + mo, c + mc)
        });
        if bo > 0 {
            100.0 * bo.saturating_sub(bc) as f64 / bo as f64
        } else {
            0.0
        }
    });
    let saved_mid = match base_ratio {
        Some(br) => format!("({ratio:.1}%, main {br:.1}% {:+.1}pt)", ratio - br),
        None => format!("({ratio:.1}%)"),
    };

    // --- totals block: (label, bytes, middle, tokens) ---
    let totals = [
        (
            "original",
            human_bytes(orig_b),
            format!("{} lines", commas(orig_l)),
            human_count(approx_tokens(orig_b)),
        ),
        (
            "compressed",
            human_bytes(comp_b),
            format!("{} lines", commas(comp_l)),
            human_count(approx_tokens(comp_b)),
        ),
        (
            "saved",
            human_bytes(saved_b),
            saved_mid,
            human_count(approx_tokens(saved_b)),
        ),
    ];
    let tw_label = totals.iter().map(|t| t.0.len()).max().unwrap_or(0);
    let tw_bytes = totals.iter().map(|t| t.1.len()).max().unwrap_or(0);
    let tw_mid = totals.iter().map(|t| t.2.len()).max().unwrap_or(0);
    let tw_tok = totals.iter().map(|t| t.3.len()).max().unwrap_or(0);
    let total_lines: Vec<String> = totals
        .iter()
        .map(|(l, b, m, t)| {
            format!("  {l:<tw_label$}   {b:>tw_bytes$}   {m:>tw_mid$}   ~{t:>tw_tok$} tok")
        })
        .collect();

    // --- by-command block: (name, original, compressed, percent[, vs main]) ---
    // 削減バイトの大きい順（hush stats の by filter と同じ並び）。
    let mut sorted: Vec<&Measured> = rows.iter().collect();
    sorted.sort_by_key(|m| std::cmp::Reverse(m.orig_bytes.saturating_sub(m.compact_bytes)));
    // 各 cmd の main 比。baseline 無し(None)なら全件を Δ 無しで出す。
    // baseline あり(PR コメント)なら「main と差がある cmd だけ」出す（Same は隠す）。
    enum Vs {
        Diff { bp: String, delta: String }, // main と差あり -> 表示
        Same,                               // main と一致   -> 隠す
        New,                                // main に対応なし -> 表示
    }
    struct CRow {
        name: String,
        ob: String,
        cb: String,
        p: String,
        vs: Option<Vs>,
    }
    let crows: Vec<CRow> = sorted
        .iter()
        .map(|m| {
            let cp = m.ratio * 100.0;
            let vs = baseline.map(|b| match b.get(m.cmd) {
                Some(&(bo, bc)) => {
                    let bp = if bo > 0 {
                        100.0 * bo.saturating_sub(bc) as f64 / bo as f64
                    } else {
                        0.0
                    };
                    let d = (cp - bp).round() as i64;
                    if d == 0 {
                        Vs::Same
                    } else {
                        let delta = if d > 0 {
                            format!("+{d}pt")
                        } else {
                            format!("{d}pt")
                        };
                        Vs::Diff {
                            bp: format!("{bp:.0}"),
                            delta,
                        }
                    }
                }
                None => Vs::New,
            });
            CRow {
                name: m.cmd.to_string(),
                ob: human_bytes(m.orig_bytes as u64),
                cb: human_bytes(m.compact_bytes as u64),
                p: format!("{cp:.0}%"),
                vs,
            }
        })
        .collect();
    // PR（baseline あり）では差分のある行だけ。baseline 無しは全件。
    let shown: Vec<&CRow> = crows
        .iter()
        .filter(|r| !matches!(r.vs, Some(Vs::Same)))
        .collect();
    let cw_name = shown.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let cw_ob = shown.iter().map(|r| r.ob.len()).max().unwrap_or(0);
    let cw_cb = shown.iter().map(|r| r.cb.len()).max().unwrap_or(0);
    let cw_pct = shown.iter().map(|r| r.p.len()).max().unwrap_or(0);
    // vs-main 注釈の桁揃え（baseline% と Δ の最大幅）。
    let cw_bp = shown
        .iter()
        .filter_map(|r| match &r.vs {
            Some(Vs::Diff { bp, .. }) => Some(bp.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let cw_d = shown
        .iter()
        .filter_map(|r| match &r.vs {
            Some(Vs::Diff { delta, .. }) => Some(delta.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let cmd_lines: Vec<String> = shown
        .iter()
        .map(|r| {
            let base = format!(
                "  {n:<cw_name$}   {ob:>cw_ob$} -> {cb:>cw_cb$}   {p:>cw_pct$}",
                n = r.name,
                ob = r.ob,
                cb = r.cb,
                p = r.p
            );
            match &r.vs {
                Some(Vs::Diff { bp, delta }) => {
                    format!("{base}   (main {bp:>cw_bp$}%, {delta:>cw_d$})")
                }
                Some(Vs::New) => format!("{base}   (new vs main)"),
                Some(Vs::Same) => unreachable!("Same は shown から除外済み"),
                None => base,
            }
        })
        .collect();

    let mut block = vec![
        Row::Center("hush compression report".to_string()),
        Row::Rule,
        Row::Center(format!("{} sample commands", rows.len())),
        Row::Rule,
    ];
    block.extend(total_lines.into_iter().map(Row::Line));
    block.push(Row::Rule);
    let by_header = if baseline.is_some() {
        "  by command (vs main)"
    } else {
        "  by command"
    };
    block.push(Row::Line(by_header.to_string()));
    if cmd_lines.is_empty() {
        // baseline ありで全 cmd が main と一致した時だけここに来る。
        block.push(Row::Line("  no per-command change vs main".to_string()));
    } else {
        block.extend(cmd_lines.into_iter().map(Row::Line));
    }
    block.push(Row::Rule);
    block.push(Row::Center(
        "~tok = bytes/4, from fixed sample inputs".to_string(),
    ));

    ui::render_to_string(&block)
}

fn build_report(rows: &[Measured], baseline: Option<&HashMap<String, (u64, u64)>>) -> String {
    // README と同じフレーム表示を code fence で包んで md 化（PR コメント/job summary 用）。
    format!(
        "## hush compression report\n\n```\n{}\n```\n",
        report_block(rows, baseline)
    )
}

/// 計測値を TSV（`cmd<TAB>orig<TAB>compact<TAB>orig_lines<TAB>shown_lines`）で書き出す。
/// CI は main 側（main の filters x この PR の fixtures）でも同じテストを走らせ、
/// この TSV を `HUSH_BASELINE` 経由で PR 側に渡して main 比 Δ を描く。
fn write_data_tsv(rows: &[Measured]) {
    let mut s = String::from("# cmd\torig_bytes\tcompact_bytes\torig_lines\tshown_lines\n");
    for m in rows {
        s.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            m.cmd, m.orig_bytes, m.compact_bytes, m.orig_lines, m.shown_lines
        ));
    }
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/compression-data.tsv");
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&out, s);
}

/// `HUSH_BASELINE` が指すベースライン TSV を cmd -> (orig, compact) で読む。
/// 未設定・空・読めない・壊れている場合は None（Δ 無しの従来表示にフォールバック）。
fn read_baseline() -> Option<HashMap<String, (u64, u64)>> {
    let path = std::env::var_os("HUSH_BASELINE")?;
    if path.is_empty() {
        return None;
    }
    let text = fs::read_to_string(&path).ok()?;
    let mut map = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut it = line.split('\t');
        let cmd = it.next()?.to_string();
        let orig: u64 = it.next()?.trim().parse().ok()?;
        let comp: u64 = it.next()?.trim().parse().ok()?;
        map.insert(cmd, (orig, comp));
    }
    if map.is_empty() { None } else { Some(map) }
}

#[test]
fn meets_minimum_compression() {
    let mut failures = Vec::new();
    for c in CASES {
        let m = measure(c);
        if m.ratio < m.min_ratio {
            failures.push(format!(
                "  {:<24} {:>5.1}%  < floor {:>5.1}%  (filter {})",
                m.cmd,
                m.ratio * 100.0,
                m.min_ratio * 100.0,
                m.filter
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "compression dropped below floor:\n{}",
        failures.join("\n")
    );
}

#[test]
fn writes_report() {
    let rows: Vec<Measured> = CASES.iter().map(measure).collect();
    // 計測値を TSV でも残す（CI が main 側の同テスト出力をベースラインに使う）。
    write_data_tsv(&rows);
    // HUSH_BASELINE があれば main 比 Δ 付き、無ければ従来どおりの絶対表示。
    let baseline = read_baseline();
    let md = build_report(&rows, baseline.as_ref());

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/compression-report.md");
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&out, &md).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));

    // `cargo test -- --nocapture` でも確認できるよう標準出力にも出す。
    println!("\n{md}");
}

const README_START: &str = "<!-- compression-report:start -->";
const README_END: &str = "<!-- compression-report:end -->";

/// README のマーカー間に最新の圧縮率表を流し込む。
///
/// 副作用（README 書き換え）を避けるため、`HUSH_UPDATE_README` が立っている時だけ実行する。
/// 通常の `cargo test` では no-op。CI は main への push（= ラベル付き PR のマージ後）で
/// このテストを env 付きで走らせ、差分があれば更新 PR を自動で開く。
#[test]
fn sync_readme() {
    if std::env::var_os("HUSH_UPDATE_README").is_none() {
        return;
    }
    let rows: Vec<Measured> = CASES.iter().map(measure).collect();
    // フレーム表示を code fence で包む（markdown 内で桁揃えを保つため）。
    // README は正準の絶対表示なので PR 相対の Δ は出さない（baseline は常に None）。
    let block = format!("```\n{}\n```", report_block(&rows, None));

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md");
    let readme = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read README: {e}"));

    let start = readme
        .find(README_START)
        .expect("README is missing the compression-report start marker");
    let end = readme
        .find(README_END)
        .expect("README is missing the compression-report end marker");
    assert!(start < end, "README markers are out of order");

    let updated = format!(
        "{}\n{}\n{}",
        &readme[..start + README_START.len()],
        block,
        &readme[end..]
    );
    if updated != readme {
        fs::write(&path, updated).unwrap_or_else(|e| panic!("write README: {e}"));
    }
}
