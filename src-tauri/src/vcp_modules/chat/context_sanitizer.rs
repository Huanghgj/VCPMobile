//! vcp_modules/context_sanitizer.rs
//! 上下文 HTML 标签转 MD 净化器模块（Rust 重构版）
//! 处理 HTML 净化、多媒体保留、VCP 特殊块提取以及元思考链清理

use ego_tree::NodeRef;
use fancy_regex::Regex;
use lazy_static::lazy_static;
use lru::LruCache;
use scraper::{Html, Node};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

lazy_static! {
    /// 清理 VCP 元思考链的正则表达式
    static ref THOUGHT_CHAIN_REGEX: Regex = Regex::new(r#"(?ims)[ \t]*\[--- VCP元思考链(?::\s*[^\]]*?)?\s*---\].*?[ \t]*\[--- 元思考链结束 ---\][ \t]*\r?\n?"#).unwrap();
    /// 清理未闭合 VCP 元思考链的正则表达式
    static ref INCOMPLETE_THOUGHT_CHAIN_REGEX: Regex = Regex::new(r#"(?ims)[ \t]*\[--- VCP元思考链(?::\s*[^\]]*?)?\s*---\].*\z"#).unwrap();
    /// 清理常规 <think>/<thinking> 标签的正则表达式
    static ref CONVENTIONAL_THOUGHT_REGEX: Regex = Regex::new(r"(?is)<think(?:ing)?>.*?</think(?:ing)?>").unwrap();
    /// 清理未闭合 <think>/<thinking> 标签的正则表达式
    static ref INCOMPLETE_CONVENTIONAL_THOUGHT_REGEX: Regex = Regex::new(r"(?is)<think(?:ing)?>.*\z").unwrap();
    /// 仅匹配实际会参与富文本渲染的 HTML 标签，避免把 `a < b > c` 误判为 HTML。
    static ref HTML_CHECK_REGEX: Regex = Regex::new(r"(?is)</?(?:html|head|body|title|meta|link|style|script|div|span|p|br|strong|b|em|i|u|s|h[1-6]|ul|ol|li|a|img|audio|video|source|pre|code|blockquote|table|thead|tbody|tfoot|tr|th|td|details|summary|section|article|header|footer|main|nav|aside|form|input|button|label|select|option|textarea|svg|canvas|iframe|template|noscript|hr)\b[^>]*>").unwrap();
    /// 清理多余空行（保留最多2个连续空行）的正则表达式
    static ref MULTI_NEWLINE_REGEX: regex::Regex = regex::Regex::new(r"\n{3,}").unwrap();
}

/// 缓存项结构，支持过期时间
pub struct CacheItem {
    pub value: String,
    pub timestamp: SystemTime,
}

/// 上下文净化器结构体，管理 LRU 缓存与 TTL
#[allow(dead_code)]
pub struct ContextSanitizer {
    /// 线程安全的 LRU 缓存：内容哈希 -> 净化后的内容
    pub cache: Mutex<LruCache<String, CacheItem>>,
    /// 缓存有效期 (Time To Live)
    pub ttl: Duration,
}

impl ContextSanitizer {
    /// 创建新的净化器实例
    /// @param capacity 缓存最大容量
    /// @param ttl_secs 过期时间（秒）
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        log::info!(
            "[ContextSanitizer] Initializing Rust ContextSanitizer with capacity {} and TTL {}s",
            capacity,
            ttl_secs
        );
        Self {
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(capacity).unwrap())),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// 从缓存中获取已净化的内容
    /// @param key 缓存键
    #[allow(dead_code)]
    pub fn get_cached(&self, key: &str) -> Option<String> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(item) = cache.get(key) {
            // 检查是否过期
            if let Ok(elapsed) = item.timestamp.elapsed() {
                if elapsed < self.ttl {
                    log::debug!("[ContextSanitizer] Cache hit for content");
                    return Some(item.value.clone());
                }
            }
            // 已过期，删除
            cache.pop(key);
        }
        None
    }

    /// 将净化后的内容存入缓存
    /// @param key 缓存键
    /// @param value 净化后的内容
    #[allow(dead_code)]
    pub fn set_cached(&self, key: String, value: String) {
        let mut cache = self.cache.lock().unwrap();
        cache.put(
            key,
            CacheItem {
                value,
                timestamp: SystemTime::now(),
            },
        );
        log::debug!("[ContextSanitizer] Sanitized content, cached result");
    }

    /// 净化单条消息内容：HTML -> Markdown (带缓存逻辑)
    /// @param content 原始内容
    /// @param keep_thoughts 是否保留思考链
    #[allow(dead_code)]
    pub fn sanitize_content(&self, content: &str, keep_thoughts: bool) -> String {
        let normalized = if keep_thoughts {
            content.to_string()
        } else {
            strip_thought_chains(content)
        };
        if normalized.trim().is_empty() {
            return normalized;
        }

        // 如果不包含 HTML，直接返回
        if !contains_html(&normalized) {
            return normalized;
        }

        // 尝试从缓存获取
        let cache_key = generate_cache_key(content, keep_thoughts);
        if let Some(cached) = self.get_cached(&cache_key) {
            return cached;
        }

        // 核心执行：HTML 转换为 Markdown
        let result = html_to_vcp_markdown(&normalized, keep_thoughts);

        // 存入缓存
        self.set_cached(cache_key, result.clone());
        result
    }
}

/// 默认配置：最大 100 条缓存，1 小时过期
impl Default for ContextSanitizer {
    fn default() -> Self {
        Self::new(100, 3600)
    }
}

/// 清理元思考链（明文形式）
/// @param content 原始内容
/// @returns 清理后的内容
#[allow(dead_code)]
pub fn strip_thought_chains(content: &str) -> String {
    let s = THOUGHT_CHAIN_REGEX.replace_all(content, "").to_string();
    let s = INCOMPLETE_THOUGHT_CHAIN_REGEX
        .replace_all(&s, "")
        .to_string();
    let s = CONVENTIONAL_THOUGHT_REGEX.replace_all(&s, "").to_string();
    INCOMPLETE_CONVENTIONAL_THOUGHT_REGEX
        .replace_all(&s, "")
        .to_string()
}

pub fn assistant_context_contains_html(content: &str) -> bool {
    contains_html(&strip_thought_chains(content))
}

pub fn sanitize_assistant_context_content(content: &str, preserve_render: bool) -> String {
    let without_thoughts = strip_thought_chains(content);
    if preserve_render || !contains_html(&without_thoughts) {
        without_thoughts
    } else {
        html_to_vcp_markdown(&without_thoughts, false)
    }
}

/// 核心算法：将 HTML 树转换为 VCP 风格的 Markdown
/// @param html 输入的 HTML 字符串
/// @param keep_thoughts 是否保留思考链
pub fn html_to_vcp_markdown(html: &str, keep_thoughts: bool) -> String {
    let fragment = Html::parse_fragment(html);
    let mut result = String::new();

    // 遍历 HTML 树的根节点子代
    for node in fragment.tree.root().children() {
        process_node(node, &mut result, keep_thoughts);
    }

    // 清理多余空行，对齐 JS 逻辑
    MULTI_NEWLINE_REGEX
        .replace_all(result.trim(), "\n\n")
        .to_string()
}

/// 递归处理 HTML 节点
fn process_node(node: NodeRef<Node>, out: &mut String, keep_thoughts: bool) {
    match node.value() {
        // 处理文本节点
        Node::Text(text) => {
            out.push_str(&text.text);
        }
        // 处理元素节点
        Node::Element(el) => {
            let tag = el.name();

            // 算法 A：VCP 协议块的“零损提取”
            if let Some(raw) = el.attr("data-raw-content") {
                if raw.contains("<<<[TOOL_REQUEST]>>>")
                    || raw.contains("<<<DailyNoteStart>>>")
                    || raw.contains("[[VCP调用结果信息汇总:")
                {
                    out.push_str(raw);
                    return;
                }
            }

            // 算法 B：多媒体与特殊结构处理
            match tag {
                // 保留图片标签
                "img" => {
                    let alt = el.attr("alt").unwrap_or("").trim();
                    if alt.is_empty() {
                        out.push_str("[图片]");
                    } else {
                        out.push_str(&format!("[图片: {alt}]"));
                    }
                }
                // 媒体的物理载荷由附件通道负责，历史上下文只保留语义占位。
                "audio" | "video" => {
                    if tag == "audio" {
                        out.push_str("[音频]");
                    } else {
                        out.push_str("[视频]");
                    }
                }
                // 纯渲染、资源或执行节点不进入历史上下文。
                "style" | "script" | "svg" | "canvas" | "iframe" | "template" | "noscript"
                | "link" | "meta" | "source" => {}
                "think" | "thinking" => {
                    if keep_thoughts {
                        for child in node.children() {
                            process_node(child, out, keep_thoughts);
                        }
                    }
                }
                // 处理代码块与 VCP 特殊原始块
                "pre" => {
                    let mut text_content = String::new();
                    collect_text(node, &mut text_content);

                    // 检查是否包含 VCP 专用原始标记
                    if text_content.contains("<<<[TOOL_REQUEST]>>>")
                        || text_content.contains("<<<DailyNoteStart>>>")
                    {
                        out.push_str(&text_content);
                    } else {
                        // 普通 pre 标签转为 Markdown 代码块
                        out.push_str("\n```\n");
                        out.push_str(&text_content);
                        out.push_str("\n```\n");
                    }
                }
                // 算法 C：元思考链的结构化处理
                "div"
                    if el.has_class(
                        "vcp-thought-chain-bubble",
                        scraper::CaseSensitivity::AsciiCaseInsensitive,
                    ) =>
                {
                    if keep_thoughts {
                        let title = el.attr("data-thought-title").unwrap_or("");
                        let title_part = if !title.is_empty() {
                            format!(r#": "{}""#, title)
                        } else {
                            String::new()
                        };
                        out.push_str(&format!("\n\n[--- VCP元思考链{} ---]\n", title_part));
                        for child in node.children() {
                            process_node(child, out, keep_thoughts);
                        }
                        out.push_str("\n[--- 元思考链结束 ---]\n\n");
                    }
                }
                // --- 基础 HTML 标签转 Markdown ---
                "p" => {
                    out.push('\n');
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('\n');
                }
                "br" => out.push('\n'),
                "strong" | "b" => {
                    out.push_str("**");
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push_str("**");
                }
                "em" | "i" => {
                    out.push('*');
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('*');
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = tag.chars().last().unwrap().to_digit(10).unwrap_or(1);
                    out.push('\n');
                    for _ in 0..level {
                        out.push('#');
                    }
                    out.push(' ');
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('\n');
                }
                "title" => {
                    out.push_str("\n# ");
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('\n');
                }
                "code" => {
                    out.push('`');
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('`');
                }
                "ul" => {
                    out.push('\n');
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('\n');
                }
                "li" => {
                    out.push_str("- ");
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('\n');
                }
                "blockquote" => {
                    let mut text = String::new();
                    for child in node.children() {
                        process_node(child, &mut text, keep_thoughts);
                    }
                    for line in text.lines() {
                        out.push_str("> ");
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                "table" => {
                    out.push('\n');
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push('\n');
                }
                "tr" => {
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push_str("|\n");
                }
                "th" | "td" => {
                    out.push_str("| ");
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                    out.push(' ');
                }
                "hr" => out.push_str("\n---\n"),
                "a" => {
                    let href = el.attr("href").unwrap_or("");
                    let unsafe_or_oversized = href.starts_with("data:")
                        || href.starts_with("javascript:")
                        || href.len() > 2048;
                    if href.is_empty() || unsafe_or_oversized {
                        for child in node.children() {
                            process_node(child, out, keep_thoughts);
                        }
                    } else {
                        out.push('[');
                        for child in node.children() {
                            process_node(child, out, keep_thoughts);
                        }
                        out.push_str(&format!("]({})", href));
                    }
                }
                _ => {
                    // 默认透传：递归处理子节点
                    for child in node.children() {
                        process_node(child, out, keep_thoughts);
                    }
                }
            }
        }
        _ => {}
    }
}

/// 辅助函数：收集节点下所有的纯文本内容
fn collect_text(node: NodeRef<Node>, out: &mut String) {
    for child in node.children() {
        match child.value() {
            Node::Text(text) => out.push_str(&text.text),
            Node::Element(_) => collect_text(child, out),
            _ => {}
        }
    }
}

/// 检查内容是否包含 HTML 标签
#[allow(dead_code)]
pub fn contains_html(content: &str) -> bool {
    HTML_CHECK_REGEX.is_match(content).unwrap_or(false)
}

/// 生成缓存键（使用哈希与长度组合）
#[allow(dead_code)]
pub fn generate_cache_key(content: &str, keep_thoughts: bool) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    keep_thoughts.hash(&mut hasher);
    let hash = hasher.finish();
    format!("sanitized_{}_{}", hash, content.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_thoughts() {
        let input = "Hello [--- VCP元思考链: \"test\" ---] secret [--- 元思考链结束 ---] World <think>internal</think>";
        let stripped = strip_thought_chains(input);
        assert!(stripped.contains("Hello"));
        assert!(stripped.contains("World"));
        assert!(!stripped.contains("secret"));
        assert!(!stripped.contains("internal"));
        assert!(!stripped.contains("VCP元思考链"));
        assert!(!stripped.contains("<think>"));
    }

    #[test]
    fn test_strip_thought_variants() {
        let input = "A\n[--- VCP元思考链: test ---]\nsecret\n[--- 元思考链结束 ---]\nB <thinking>hidden</thinking> C";
        let stripped = strip_thought_chains(input);
        assert!(stripped.contains("A"));
        assert!(stripped.contains("B"));
        assert!(stripped.contains("C"));
        assert!(!stripped.contains("secret"));
        assert!(!stripped.contains("hidden"));
        assert!(!stripped.contains("<thinking>"));
    }

    #[test]
    fn test_strip_incomplete_thought_to_end() {
        let input = "Visible\n<think>unfinished internal text";
        assert_eq!(strip_thought_chains(input), "Visible\n");
    }

    #[test]
    fn test_html_to_md_img() {
        let html = r#"<p>Hello <img src="test.png" alt="alt text"> World</p>"#;
        let md = html_to_vcp_markdown(html, false);
        assert!(md.contains("[图片: alt text]"));
        assert!(!md.contains("test.png"));
    }

    #[test]
    fn test_raw_content() {
        let html = "<pre data-raw-content=\"<<<[TOOL_REQUEST]>>>\ncall()\"></pre>";
        let md = html_to_vcp_markdown(html, false);
        assert_eq!(md, "<<<[TOOL_REQUEST]>>>\ncall()");
    }

    #[test]
    fn assistant_context_compacts_old_render_markup() {
        let html = r#"
            <style>.card { color: red; }</style>
            <script>window.secret = true;</script>
            <div class="card"><h2>Report</h2><p>Hello <strong>world</strong>.</p></div>
            <img src="data:image/png;base64,AAAA" alt="chart">
            <iframe src="https://example.com/embed"></iframe>
        "#;

        let compacted = sanitize_assistant_context_content(html, false);

        assert!(compacted.contains("## Report"));
        assert!(compacted.contains("Hello **world**."));
        assert!(compacted.contains("[图片: chart]"));
        assert!(!compacted.contains("color: red"));
        assert!(!compacted.contains("window.secret"));
        assert!(!compacted.contains("base64"));
        assert!(!compacted.contains("example.com/embed"));
        assert!(compacted.len() < html.len());
    }

    #[test]
    fn assistant_context_preserves_latest_render_but_removes_thoughts() {
        let html =
            "<style>.card{color:red}</style><div class=\"card\">Visible</div><think>hidden</think>";

        let preserved = sanitize_assistant_context_content(html, true);

        assert!(preserved.contains("<style>"));
        assert!(preserved.contains("class=\"card\""));
        assert!(preserved.contains("Visible"));
        assert!(!preserved.contains("hidden"));
        assert!(!preserved.contains("<think>"));
    }

    #[test]
    fn comparison_text_is_not_treated_as_html() {
        let content = "Keep the relation a < b > c unchanged.";

        assert!(!assistant_context_contains_html(content));
        assert_eq!(sanitize_assistant_context_content(content, false), content);
    }
}
