#![allow(dead_code, unused_imports)]

//! Tail 处理性能基准（criterion harness）。
//!
//! 目的：为 `MAX_SPECULATIVE_TAIL_AST_BYTES`（当前 8192）和流式代码块高亮阈值
//! （当前 4096）的取值提供实测数据，并为自适应降帧梯度提供单帧开销曲线。
//!
//! 运行方式（用 perf profile 逼近发布版热路径性能）：
//!   cargo bench --profile perf
//!
//! 注意：dev profile 下的绝对耗时无参考意义；务必看 perf profile 的输出，
//! 并参考文末的"发布版换算说明"。
//!
//! 历史：从 `src/vcp_modules/chat/ast_bench.rs` 迁出，由 `#[test]` 改为 criterion
//! benchmark，避免污染 `cargo test` 反馈环。fixture 改用 `include_str!` 编译期内嵌，
//! 杜绝运行时绝对路径依赖。

#[path = "../src/distributed/mod.rs"]
mod distributed;
#[path = "../src/vcp_modules/mod.rs"]
mod vcp_modules;

use crate::vcp_modules::aurora_pipeline::{AuroraBuffer, AuroraUpdate};
use crate::vcp_modules::pre_renderer::code_highlighter::highlight_code_block;
use crate::vcp_modules::pre_renderer::parse_markdown_to_ast;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

/// 读取 40k 测试 HTML 源（827 行）。编译期内嵌，杜绝运行时路径依赖。
fn load_genesis_html() -> &'static str {
    include_str!("fixtures/v1.1.0-aurora-genesis.html")
}

/// 按字节上限安全截断到 char 边界。
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

const TIERS: [usize; 8] = [2048, 4096, 8192, 16384, 24576, 32768, 40000, 40960];

/// 将一段内容包装为未闭合的流式 html 代码围栏（模拟 AI 正在吐出 ```html 块）。
fn as_open_code_fence(content: &str) -> String {
    format!("```html\n{}", content)
}

/// 基准 1：最终块 parse + serialize 开销。
///
/// 活动 tail 已不再逐帧生成 AST；这个基准衡量块闭合时的一次性成本。
fn bench_single_frame_pipeline(c: &mut Criterion) {
    let html = load_genesis_html();
    let mut group = c.benchmark_group("tail_single_frame_pipeline");

    for &tier in TIERS.iter() {
        group.bench_with_input(BenchmarkId::from_parameter(tier), &tier, |b, &tier| {
            let content = truncate_on_char_boundary(html, tier);
            let fenced = as_open_code_fence(content);

            b.iter(|| {
                let parsed = parse_markdown_to_ast(&fenced);
                let serialized = serde_json::to_string(&parsed).unwrap();
                black_box(serialized);
            });
        });
    }
    group.finish();
}

/// 基准 2：syntect 高亮开销（决定 4096 流式高亮阈值是否合理）。
fn bench_syntect_highlight(c: &mut Criterion) {
    let html = load_genesis_html();
    let mut group = c.benchmark_group("tail_syntect_highlight");

    for &tier in TIERS.iter() {
        group.bench_with_input(BenchmarkId::from_parameter(tier), &tier, |b, &tier| {
            let content = truncate_on_char_boundary(html, tier).to_string();
            b.iter(|| {
                let out = highlight_code_block(black_box(&content), "html");
                black_box(out);
            });
        });
    }
    group.finish();
}

/// 基准 3：累计流式开销 —— 一个代码块从 0 增长到目标尺寸，逐帧 re-parse 的总和。
///
/// 模拟真实 SSE：固定 chunk 字节（约模拟一次 SSE delta），每追加一块就跑一次
/// parse + serialize。它给出旧式逐帧 AST 路径的累计成本上界。
fn bench_cumulative_stream(c: &mut Criterion) {
    let html = load_genesis_html();
    const CHUNK_BYTES: usize = 48;
    let mut group = c.benchmark_group("tail_cumulative_stream");

    for &tier in TIERS.iter() {
        group.bench_with_input(BenchmarkId::from_parameter(tier), &tier, |b, &tier| {
            let full = truncate_on_char_boundary(html, tier);

            // 构造增长边界（char 安全）
            let mut bounds: Vec<usize> = Vec::new();
            let mut bnd = CHUNK_BYTES;
            while bnd < full.len() {
                let mut e = bnd;
                while e < full.len() && !full.is_char_boundary(e) {
                    e += 1;
                }
                bounds.push(e);
                bnd += CHUNK_BYTES;
            }
            bounds.push(full.len());

            b.iter(|| {
                for &end in &bounds {
                    let content = &full[..end];
                    let fenced = as_open_code_fence(content);
                    let ast = parse_markdown_to_ast(&fenced);
                    let _ = serde_json::to_string(&ast).unwrap();
                }
                black_box(());
            });
        });
    }
    group.finish();
}

/// 基准 4：端到端 AuroraBuffer —— 用真实管道喂入增长的代码块，验证整链路（含
/// compact `AuroraUpdate` 序列化）的真实开销，而非孤立函数。
fn bench_end_to_end_aurora(c: &mut Criterion) {
    let html = load_genesis_html();
    const CHUNK_BYTES: usize = 48;
    let mut group = c.benchmark_group("tail_end_to_end_aurora");

    for &tier in TIERS.iter() {
        group.bench_with_input(BenchmarkId::from_parameter(tier), &tier, |b, &tier| {
            let full = truncate_on_char_boundary(html, tier);
            let fenced_full = as_open_code_fence(full);
            let chars_total = fenced_full.len();

            b.iter(|| {
                let mut buffer = AuroraBuffer::new();
                let mut sent = 0usize;
                while sent < chars_total {
                    let mut end = (sent + CHUNK_BYTES).min(chars_total);
                    while end < chars_total && !fenced_full.is_char_boundary(end) {
                        end += 1;
                    }
                    let chunk = &fenced_full[sent..end];
                    sent = end;
                    buffer.append_chunk(chunk);
                    let (stable_changed, tail_changed) = buffer.process_queue();
                    let update = AuroraUpdate {
                        sequence: 1,
                        stable_blocks_delta: None,
                        stable_changed,
                        tail_block: buffer.tail_block.clone(),
                        tail_changed,
                        content_delta: Some(chunk.to_string()),
                        content: None,
                    };
                    let _ = serde_json::to_string(&update).unwrap();
                }
                black_box(());
            });
        });
    }
    group.finish();
}

criterion_group!(
    name = ast_tail_benches;
    config = Criterion::default();
    targets = bench_single_frame_pipeline,
    bench_syntect_highlight,
    bench_cumulative_stream,
    bench_end_to_end_aurora,
);
criterion_main!(ast_tail_benches);
