use crate::vcp_modules::pre_renderer::code_highlighter::highlight_code_block;
use crate::vcp_modules::pre_renderer::markdown_ast::{InlineNode, MarkdownNode};
use lazy_static::lazy_static;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use regex::Regex;
use std::borrow::Cow;

lazy_static! {
    static ref FENCE_RE: Regex =
        Regex::new(r"(?m)^[ \t]*```[a-zA-Z0-9-]*[ \t]*\r?$").unwrap();

    static ref HTML_AWARE_FENCE_RE: Regex =
        Regex::new(r"(?im)(?:^[ \t]*|</(?:div|section|article|header|footer|main|aside|figure|figcaption)>[ \t]*)(?P<fence>```[a-zA-Z0-9-]*[ \t]*\r?$)").unwrap();

    // 合并 LaTeX 匹配：[ ... ] 和 ( ... )
    static ref MATH_RE: Regex = Regex::new(r"(?s)\\\[(?P<display>.*?)\\\]|\\\((?P<inline>.*?)\\\)").unwrap();

    static ref MAGIC_RE: Regex =
        Regex::new(r##"(?s)(["“”](?:[^"“”\r\n]|\\.)+?["“”])|(@![^\s@!]+)|(@[^\s@]+)"##).unwrap();

    static ref HTML_CONTAINER_PLACEHOLDER_RE: Regex =
        Regex::new(r"<!--VCP_HTML_CONTAINER:(\d+)-->").unwrap();

    static ref TAG_SCANNER: Regex = Regex::new(r"(?i)(</?)([a-z0-9\-]+)(\s[^>]*)?>").unwrap();

    static ref INLINE_CODE_RE: Regex = Regex::new(r"(?m)`+[^`\n\r]+`+").unwrap();

    static ref COMMENT_RE: Regex = Regex::new(r"(?s)<!--[\s\S]*?(?:-->|$)").unwrap();

    static ref HTML_CONTAINER_CLOSE_BEFORE_FENCE_RE: Regex =
        Regex::new(r"(?im)</(?P<tag>div|section|article|header|footer|main|aside|figure|figcaption)>[ \t]*(?:\r?\n[ \t]*)?(?P<fence>```[a-zA-Z0-9-]*[ \t]*\r?$)").unwrap();

    // 仅在 字母/数字 + ** + 标点 的模式下注入零宽空格，修复 CommonMark left-flanking 判定失效。
    // 前驱限定为 [\p{L}\p{N}] 确保不会误触发闭合符号（闭合 ** 前驱通常是标点或 \u{200B}）。
    static ref FLANKING_FIX_LEFT: Regex =
        Regex::new(r"([\p{L}\p{N}])(\*\*|\*)([[\p{P}]&&[^*_]])").unwrap();

    // 匹配行首 ≥4 空格/Tab 缩进后紧跟 $$ 的模式（块级公式被误判为缩进代码块的根因）
    static ref INDENTED_DOLLAR_RE: Regex =
        Regex::new(r"(?m)^[ \t]{4,}(\$\$)").unwrap();
}

fn fix_flanking_delimiters(text: &str) -> String {
    if !text.contains('*') {
        return text.to_string();
    }
    FLANKING_FIX_LEFT
        .replace_all(text, "${1}${2}\u{200B}${3}")
        .into_owned()
}

/// 修复模型常见的混合渲染断口：
///
/// ```text
/// </div>```yaml
/// ...
/// ```
///
/// 这类输出通常是为了从 HTML 容器临时切回 Markdown 围栏。这里不能全局删除或改写闭合标签：
/// 对正常 HTML 片段来说它可能是真实边界。实际修复发生在已经确认的容器内部，
/// 由 `strip_stuck_container_close_before_fence` 只剥掉那一个假闭合。
fn normalize_container_breaking_code_fences(text: &str) -> Cow<'_, str> {
    if !text.contains("```") || !text.contains("</") {
        return Cow::Borrowed(text);
    }

    HTML_CONTAINER_CLOSE_BEFORE_FENCE_RE.replace_all(text, "</${tag}>\n${fence}")
}

pub(crate) fn strip_stuck_container_close_before_fence(text: &str) -> Cow<'_, str> {
    if !text.contains("```") || !text.contains("</") {
        return Cow::Borrowed(text);
    }

    HTML_CONTAINER_CLOSE_BEFORE_FENCE_RE.replace_all(text, "${fence}")
}

/// 剥除块级 $$ 公式行的多余前导缩进，防止 pulldown-cmark 将其误判为缩进代码块。
/// CommonMark 规则：≥4 空格缩进 = 缩进代码块，优先级高于 math 扩展的行内识别。
/// 此函数在代码围栏内部保持原样，只处理围栏外的文本。
fn strip_display_math_indent(text: &str) -> Cow<'_, str> {
    if !text.contains("$$") {
        return Cow::Borrowed(text);
    }
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;
    let mut in_fence = false;

    for m in FENCE_RE.find_iter(text) {
        let segment = &text[last_end..m.start()];
        if !in_fence {
            result.push_str(INDENTED_DOLLAR_RE.replace_all(segment, "$1").as_ref());
        } else {
            result.push_str(segment);
        }
        result.push_str(m.as_str());
        last_end = m.end();
        in_fence = !in_fence;
    }

    let tail = &text[last_end..];
    if !in_fence {
        result.push_str(INDENTED_DOLLAR_RE.replace_all(tail, "$1").as_ref());
    } else {
        result.push_str(tail);
    }

    Cow::Owned(result)
}

fn preprocess_latex_math(text: &str) -> Cow<'_, str> {
    if !text.contains("\\[") && !text.contains("\\(") {
        return Cow::Borrowed(text);
    }

    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;
    let mut in_fence = false;

    // 1. 扫描代码围栏
    for m in FENCE_RE.find_iter(text) {
        let segment = &text[last_end..m.start()];
        if !in_fence {
            // 在围栏外：执行极速公式替换
            push_math_replaced(&mut result, segment);
        } else {
            // 在围栏内：直接追加
            result.push_str(segment);
        }
        result.push_str(m.as_str());
        last_end = m.end();
        in_fence = !in_fence;
    }

    // 2. 处理尾部
    let tail = &text[last_end..];
    if !in_fence {
        push_math_replaced(&mut result, tail);
    } else {
        result.push_str(tail);
    }

    Cow::Owned(result)
}

/// 辅助函数：将含有 LaTeX 的片段高效推送到结果缓冲区，不产生中间 String
fn push_math_replaced(dest: &mut String, segment: &str) {
    let mut last_match_end = 0;
    for caps in MATH_RE.captures_iter(segment) {
        let full_match = caps.get(0).unwrap();
        // 推送匹配项之前的普通文本
        dest.push_str(&segment[last_match_end..full_match.start()]);

        // 识别是哪种模式并直接推送
        if let Some(display) = caps.name("display") {
            dest.push_str("$$");
            dest.push_str(display.as_str());
            dest.push_str("$$");
        } else if let Some(inline) = caps.name("inline") {
            dest.push('$');
            dest.push_str(inline.as_str());
            dest.push('$');
        }

        last_match_end = full_match.end();
    }
    // 推送剩余文本
    dest.push_str(&segment[last_match_end..]);
}

/// 提取 HTML 容器块，将其替换为占位符，并递归解析内部 Markdown
#[allow(clippy::type_complexity)]
fn extract_html_containers(text: &str) -> (Cow<'_, str>, Vec<(String, Vec<MarkdownNode>, String)>) {
    if !text.contains('<') {
        return (Cow::Borrowed(text), Vec::new());
    }

    let mut result = String::with_capacity(text.len());
    let mut containers: Vec<(String, Vec<MarkdownNode>, String)> = Vec::new();
    let mut last_pos = 0;

    // 预先收集所有代码围栏的位置以供快速查询 (标准 regex find_iter)
    let fences: Vec<regex::Match> = FENCE_RE.find_iter(text).collect();
    let mut fence_cursor = 0;
    let mut in_fence = false;

    // 预先收集所有内联反引号的范围以跳过误提取
    let inline_codes: Vec<(usize, usize)> = INLINE_CODE_RE
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect();

    for cap in crate::vcp_modules::content_parser::HTML_CONTAINER_OPEN_RE.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let tag = cap.get(1).unwrap().as_str().to_lowercase();

        if m.start() < last_pos {
            continue;
        }

        // 跳过被行内反引号包裹的标签（例如 `<div>`）
        let is_in_inline = inline_codes
            .iter()
            .any(|&(start, end)| m.start() >= start && m.end() <= end);
        if is_in_inline {
            continue;
        }

        // 高效同步围栏状态：跳过当前匹配位置之前的围栏切换
        while fence_cursor < fences.len() && fences[fence_cursor].start() <= m.start() {
            in_fence = !in_fence;
            fence_cursor += 1;
        }

        if in_fence {
            continue;
        }

        // 找到匹配的闭标签（考虑嵌套）
        if let Some((close_start, close_end)) = find_matching_close_tag(text, m.end(), &tag) {
            let open_tag = text[m.start()..m.end()].to_string();
            let inner_text = text[m.end()..close_start].to_string();
            let close_tag = text[close_start..close_end].to_string();

            // 将之前的内容加入结果
            result.push_str(&text[last_pos..m.start()]);

            // 创建占位符
            let placeholder = format!("<!--VCP_HTML_CONTAINER:{}-->", containers.len());
            result.push_str(&placeholder);

            // 递归解析内部内容
            let deindented_inner = trim_common_leading_indent(&inner_text);
            let markdown_inner = strip_stuck_container_close_before_fence(&deindented_inner);
            let inner_nodes = parse_markdown_to_ast(markdown_inner.as_ref());
            containers.push((open_tag, inner_nodes, close_tag));

            last_pos = close_end;

            // 由于 last_pos 跳跃了，同步围栏游标状态
            while fence_cursor < fences.len() && fences[fence_cursor].start() < last_pos {
                in_fence = !in_fence;
                fence_cursor += 1;
            }
        }
    }

    result.push_str(&text[last_pos..]);
    (Cow::Owned(result), containers)
}

/// 去除文本中所有非空行的公共前导缩进（空格/制表符）。
/// 用于 HTML 容器内部文本：去除嵌套带来的绝对缩进，保留相对结构，
/// 防止 pulldown-cmark 将缩进内容误识别为 Indented Code Block。
pub(crate) fn trim_common_leading_indent(text: &str) -> String {
    let mut min_indent = usize::MAX;

    // 第一遍：纯计算最小公共前导缩进（利用 split 惰性迭代，零堆分配）
    for line in text.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() && trimmed != "<br>" && trimmed != "<br/>" {
            let mut indent = 0;
            for c in line.chars() {
                if c == ' ' {
                    indent += 1;
                } else if c == '\t' {
                    indent += 4;
                } else {
                    break;
                }
            }
            if indent < min_indent {
                min_indent = indent;
            }
        }
    }

    if min_indent == usize::MAX || min_indent == 0 {
        return text.to_string();
    }

    // 第二遍：直接 split('\n') 惰性迭代追加到预分容量的 result 中，彻底消除 Vec 缓存
    let mut result = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            result.push('\n');
        }
        if line.chars().all(|c| c.is_whitespace()) {
            // 保留空行，清除空格噪音
        } else {
            let mut skipped = 0;
            let mut char_indices = line.char_indices();
            let mut skip_bytes = 0;

            while skipped < min_indent {
                if let Some((idx, c)) = char_indices.next() {
                    if c == ' ' {
                        skipped += 1;
                        skip_bytes = idx + 1;
                    } else if c == '\t' {
                        skipped += 4;
                        skip_bytes = idx + 1;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            result.push_str(&line[skip_bytes..]);
        }
    }

    result
}

fn is_void_html_tag(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn collect_code_fence_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let markers: Vec<std::ops::Range<usize>> = HTML_AWARE_FENCE_RE
        .captures_iter(text)
        .filter_map(|cap| cap.name("fence").map(|m| m.start()..m.end()))
        .collect();

    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < markers.len() {
        let start = markers[cursor].start;
        let end = markers
            .get(cursor + 1)
            .map(|marker| marker.end)
            .unwrap_or(text.len());
        ranges.push(start..end);
        cursor += 2;
    }
    ranges
}

/// 从字符串末尾向前查找匹配的 HTML 闭标签，返回 (close_start, close_end)
pub(crate) fn find_matching_close_tag(
    text: &str,
    start_pos: usize,
    tag: &str,
) -> Option<(usize, usize)> {
    let mut depth = 1;
    let search_area = &text[start_pos..];

    // 预先收集 search_area 中所有代码围栏的物理范围（支持流式未闭合边界）。
    // 模型有时会输出 `</div>```yaml`，这里把后半段 ```yaml 也视作围栏起点，
    // 防止后续真正的 HTML 闭合标签被误判在未闭合代码块内部。
    let fence_ranges = collect_code_fence_ranges(search_area);

    // 预先收集 search_area 中所有 HTML 注释的物理范围（支持流式未闭合注释边界）
    let mut comment_ranges = Vec::new();
    for m in COMMENT_RE.find_iter(search_area) {
        comment_ranges.push(m.start()..m.end());
    }

    for cap in TAG_SCANNER.captures_iter(search_area) {
        let full_match = cap.get(0).unwrap();
        let cap_start = full_match.start();

        // 健壮性防御：如果当前扫描到的 HTML 标签处于代码块围栏内部，直接跳过
        if fence_ranges.iter().any(|range| range.contains(&cap_start)) {
            continue;
        }

        // 健壮性防御：如果当前扫描到的 HTML 标签处于 HTML 注释内部，直接跳过
        if comment_ranges
            .iter()
            .any(|range| range.contains(&cap_start))
        {
            continue;
        }

        let is_close_tag = cap.get(1).unwrap().as_str() == "</";
        let tag_name = cap.get(2).unwrap().as_str();
        let tag_text = full_match.as_str();
        let is_self_closing = tag_text.trim_end().ends_with("/>");

        if is_void_html_tag(tag_name) || is_self_closing {
            continue;
        }

        if tag_name.eq_ignore_ascii_case(tag) {
            if is_close_tag {
                if depth == 1
                    && has_later_matching_close_after_stuck_fence(
                        search_area,
                        full_match.end(),
                        tag,
                    )
                {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    let full_match = cap.get(0).unwrap();
                    return Some((start_pos + full_match.start(), start_pos + full_match.end()));
                }
            } else {
                depth += 1;
            }
        }
    }
    None
}

fn stuck_code_fence_after_close(
    search_area: &str,
    relative_end: usize,
) -> Option<std::ops::Range<usize>> {
    let Some(rest) = search_area.get(relative_end..) else {
        return None;
    };

    let mut offset = 0;
    let mut after_horizontal_ws = rest;
    while after_horizontal_ws.starts_with([' ', '\t']) {
        let ch = after_horizontal_ws.chars().next().unwrap();
        offset += ch.len_utf8();
        after_horizontal_ws = &after_horizontal_ws[ch.len_utf8()..];
    }

    if after_horizontal_ws.starts_with("\r\n") {
        offset += 2;
        after_horizontal_ws = &after_horizontal_ws[2..];
    } else if after_horizontal_ws.starts_with('\n') {
        offset += 1;
        after_horizontal_ws = &after_horizontal_ws[1..];
    }

    while after_horizontal_ws.starts_with([' ', '\t']) {
        let ch = after_horizontal_ws.chars().next().unwrap();
        offset += ch.len_utf8();
        after_horizontal_ws = &after_horizontal_ws[ch.len_utf8()..];
    }

    let line_tail = after_horizontal_ws
        .split_once('\n')
        .map_or(after_horizontal_ws, |(line, _)| line);
    if FENCE_RE.is_match(line_tail) {
        let candidate_start = relative_end + offset;
        Some(candidate_start..candidate_start + line_tail.len())
    } else {
        None
    }
}

fn has_later_matching_close_after_stuck_fence(
    search_area: &str,
    close_end: usize,
    tag: &str,
) -> bool {
    let Some(fence_marker) = stuck_code_fence_after_close(search_area, close_end) else {
        return false;
    };

    let fence_ranges = collect_code_fence_ranges(search_area);
    let after_fence = fence_ranges
        .iter()
        .find(|range| range.start == fence_marker.start)
        .map(|range| range.end)
        .unwrap_or(fence_marker.end)
        .min(search_area.len());

    let after = &search_area[after_fence..];
    let later_fence_ranges = collect_code_fence_ranges(after);
    let comment_ranges: Vec<std::ops::Range<usize>> = COMMENT_RE
        .find_iter(after)
        .map(|m| m.start()..m.end())
        .collect();

    TAG_SCANNER.captures_iter(after).any(|cap| {
        let full_match = cap.get(0).unwrap();
        let cap_start = full_match.start();
        if later_fence_ranges
            .iter()
            .any(|range| range.contains(&cap_start))
            || comment_ranges
                .iter()
                .any(|range| range.contains(&cap_start))
        {
            return false;
        }

        let is_close_tag = cap.get(1).unwrap().as_str() == "</";
        let tag_name = cap.get(2).unwrap().as_str();
        is_close_tag && tag_name.eq_ignore_ascii_case(tag)
    })
}

/// 后处理：将 AST 中的占位符替换为开标签 + 子节点 + 闭标签
fn replace_container_placeholders(
    nodes: &mut Vec<MarkdownNode>,
    containers: &[(String, Vec<MarkdownNode>, String)],
) {
    let mut i = 0;
    while i < nodes.len() {
        if let MarkdownNode::RawHtml { content, .. } = &nodes[i] {
            if let Some(caps) = HTML_CONTAINER_PLACEHOLDER_RE.captures(content) {
                if let Some(idx_match) = caps.get(1) {
                    if let Ok(idx) = idx_match.as_str().parse::<usize>() {
                        if idx < containers.len() {
                            let (open_tag, children, close_tag) = &containers[idx];
                            let mut replacement = Vec::new();
                            replacement.push(MarkdownNode::raw_html(open_tag.clone()));
                            replacement.extend(children.clone());
                            replacement.push(MarkdownNode::raw_html(close_tag.clone()));
                            nodes.splice(i..=i, replacement);
                            i += children.len() + 2;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }
}

pub fn parse_markdown_to_ast(text: &str) -> Vec<MarkdownNode> {
    parse_markdown_to_ast_opt(text, false)
}

fn parse_markdown_to_ast_opt(text: &str, is_streaming: bool) -> Vec<MarkdownNode> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parse_markdown_to_ast_impl(text, is_streaming)
    }));
    match result {
        Ok(nodes) => nodes,
        Err(e) => {
            log::error!("[PreRender] parse_markdown_to_ast panicked: {:?}", e);
            let mut fallback_node =
                MarkdownNode::paragraph(vec![InlineNode::text(text.to_string())]);
            fallback_node.compute_hashes_recursively();
            vec![fallback_node]
        }
    }
}

fn parse_markdown_to_ast_impl(text: &str, is_streaming: bool) -> Vec<MarkdownNode> {
    let text = normalize_container_breaking_code_fences(text);
    let text_fixed = fix_flanking_delimiters(text.as_ref());
    let text = preprocess_latex_math(&text_fixed);
    let text = strip_display_math_indent(text.as_ref());
    let (text, containers) = extract_html_containers(text.as_ref());

    let mut nodes = Vec::new();
    let parser = Parser::new_ext(
        text.as_ref(),
        Options::ENABLE_MATH | Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    );

    let mut stack: Vec<PartialNode> = Vec::new();
    let mut accumulated_text = String::new();

    let flush_accumulated_text =
        |accumulated: &mut String, stack: &mut Vec<PartialNode>, nodes: &mut Vec<MarkdownNode>| {
            if !accumulated.is_empty() {
                let inline_nodes = if matches!(stack.last(), Some(PartialNode::CodeBlock { .. })) {
                    vec![InlineNode::text(accumulated.clone())]
                } else {
                    process_text_magic(accumulated)
                };
                if let Some(top) = stack.last_mut() {
                    top.push_inlines(inline_nodes);
                } else {
                    nodes.push(MarkdownNode::paragraph(inline_nodes));
                }
                accumulated.clear();
            }
        };

    for event in parser {
        if let Event::Text(text) = event {
            accumulated_text.push_str(&text);
            continue;
        }

        flush_accumulated_text(&mut accumulated_text, &mut stack, &mut nodes);

        match event {
            Event::Start(tag) => {
                stack.push(PartialNode::from_tag(tag));
            }
            Event::Code(code) => {
                if let Some(top) = stack.last_mut() {
                    top.push_inline(InlineNode::code(code.to_string()));
                }
            }
            Event::InlineMath(math) => {
                if let Some(top) = stack.last_mut() {
                    top.push_inline(InlineNode::inline_math(math.to_string(), false));
                } else {
                    nodes.push(MarkdownNode::paragraph(vec![InlineNode::inline_math(
                        math.to_string(),
                        false,
                    )]));
                }
            }
            Event::DisplayMath(math) => {
                let inline_node = InlineNode::inline_math(math.to_string(), true);
                if let Some(parent) = stack.last_mut() {
                    parent.push_inline(inline_node);
                } else {
                    nodes.push(MarkdownNode::paragraph(vec![inline_node]));
                }
            }
            Event::End(tag_end) => {
                if let Some(node) = stack.pop() {
                    match node {
                        PartialNode::Item { children } => {
                            if let Some(parent) = stack.last_mut() {
                                parent.push_list_item(children);
                            }
                        }
                        PartialNode::TableCell { children } => {
                            if let Some(parent) = stack.last_mut() {
                                parent.push_table_cell(children);
                            }
                        }
                        PartialNode::TableHead { cells } => {
                            if let Some(parent) = stack.last_mut() {
                                parent.set_table_header(cells);
                            }
                        }
                        PartialNode::TableRow { cells } => {
                            if let Some(parent) = stack.last_mut() {
                                parent.push_table_row(cells);
                            }
                        }
                        PartialNode::Strong { children } => {
                            let inline = InlineNode::strong(children);
                            if let Some(parent) = stack.last_mut() {
                                parent.push_inline(inline);
                            } else {
                                nodes.push(MarkdownNode::paragraph(vec![inline]));
                            }
                        }
                        PartialNode::Emphasis { children } => {
                            let inline = InlineNode::emphasis(children);
                            if let Some(parent) = stack.last_mut() {
                                parent.push_inline(inline);
                            } else {
                                nodes.push(MarkdownNode::paragraph(vec![inline]));
                            }
                        }
                        PartialNode::Strikethrough { children } => {
                            let inline = InlineNode::strikethrough(children);
                            if let Some(parent) = stack.last_mut() {
                                parent.push_inline(inline);
                            } else {
                                nodes.push(MarkdownNode::paragraph(vec![inline]));
                            }
                        }
                        PartialNode::Link {
                            href,
                            title,
                            children,
                        } => {
                            let needs_asset_conversion =
                                href.starts_with("vcp-asset:") || href.starts_with("/");
                            let mut inline = InlineNode::link(href, title, children);
                            if let InlineNode::Link {
                                needs_asset_conversion: nac,
                                ..
                            } = &mut inline
                            {
                                *nac = needs_asset_conversion;
                            }

                            if let Some(parent) = stack.last_mut() {
                                parent.push_inline(inline);
                            } else {
                                nodes.push(MarkdownNode::paragraph(vec![inline]));
                            }
                        }
                        PartialNode::Image { src, alt, title } => {
                            let needs_asset_conversion =
                                src.starts_with("vcp-asset:") || src.starts_with("/");
                            let mut inline = InlineNode::image(src, alt, title);
                            if let InlineNode::Image {
                                needs_asset_conversion: nac,
                                ..
                            } = &mut inline
                            {
                                *nac = needs_asset_conversion;
                            }

                            if let Some(parent) = stack.last_mut() {
                                parent.push_inline(inline);
                            } else {
                                nodes.push(MarkdownNode::paragraph(vec![inline]));
                            }
                        }
                        _ => {
                            let completed = node.finalize(tag_end, is_streaming);
                            if let Some(parent) = stack.last_mut() {
                                parent.push_child(completed);
                            } else {
                                nodes.push(completed);
                            }
                        }
                    }
                }
            }
            Event::Html(html) => {
                nodes.push(MarkdownNode::raw_html(html.to_string()));
            }
            Event::InlineHtml(html) => {
                if let Some(top) = stack.last_mut() {
                    top.push_inline(InlineNode::raw_html_inline(html.to_string()));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(top) = stack.last_mut() {
                    top.push_inline(InlineNode::r#break());
                }
            }
            Event::Rule => {
                nodes.push(MarkdownNode::thematic_break());
            }
            _ => {}
        }
    }

    flush_accumulated_text(&mut accumulated_text, &mut stack, &mut nodes);

    // 后处理：将 HTML 容器占位符替换为实际的开标签 + 解析后的子节点 + 闭标签
    replace_container_placeholders(&mut nodes, &containers);

    // 计算全量 AST 节点的稳定哈希指纹
    for node in &mut nodes {
        node.compute_hashes_recursively();
    }

    nodes
}

enum PartialNode {
    Paragraph {
        children: Vec<InlineNode>,
    },
    Heading {
        level: u8,
        children: Vec<InlineNode>,
    },
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    Blockquote {
        children: Vec<MarkdownNode>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<MarkdownNode>>,
    },
    Item {
        children: Vec<MarkdownNode>,
    },
    Table {
        header: Vec<Vec<InlineNode>>,
        rows: Vec<Vec<Vec<InlineNode>>>,
    },
    TableHead {
        cells: Vec<Vec<InlineNode>>,
    },
    TableRow {
        cells: Vec<Vec<InlineNode>>,
    },
    TableCell {
        children: Vec<InlineNode>,
    },
    Link {
        href: String,
        title: Option<String>,
        children: Vec<InlineNode>,
    },
    Image {
        src: String,
        alt: String,
        title: Option<String>,
    },
    Strong {
        children: Vec<InlineNode>,
    },
    Emphasis {
        children: Vec<InlineNode>,
    },
    Strikethrough {
        children: Vec<InlineNode>,
    },
}

impl PartialNode {
    fn from_tag(tag: Tag) -> Self {
        match tag {
            Tag::Paragraph => PartialNode::Paragraph {
                children: Vec::new(),
            },
            Tag::Heading { level, .. } => PartialNode::Heading {
                level: level as u8,
                children: Vec::new(),
            },
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => Some(l.to_string()),
                    CodeBlockKind::Indented => None,
                };
                PartialNode::CodeBlock {
                    lang,
                    code: String::new(),
                }
            }
            Tag::BlockQuote(_) => PartialNode::Blockquote {
                children: Vec::new(),
            },
            Tag::List(start) => PartialNode::List {
                ordered: start.is_some(),
                items: Vec::new(),
            },
            Tag::Item => PartialNode::Item {
                children: Vec::new(),
            },
            Tag::Table(_) => PartialNode::Table {
                header: Vec::new(),
                rows: Vec::new(),
            },
            Tag::TableHead => PartialNode::TableHead { cells: Vec::new() },
            Tag::TableRow => PartialNode::TableRow { cells: Vec::new() },
            Tag::TableCell => PartialNode::TableCell {
                children: Vec::new(),
            },
            Tag::Link {
                dest_url, title, ..
            } => PartialNode::Link {
                href: dest_url.to_string(),
                title: if title.is_empty() {
                    None
                } else {
                    Some(title.to_string())
                },
                children: Vec::new(),
            },
            Tag::Image {
                dest_url, title, ..
            } => PartialNode::Image {
                src: dest_url.to_string(),
                alt: String::new(),
                title: if title.is_empty() {
                    None
                } else {
                    Some(title.to_string())
                },
            },
            Tag::Strong => PartialNode::Strong {
                children: Vec::new(),
            },
            Tag::Emphasis => PartialNode::Emphasis {
                children: Vec::new(),
            },
            Tag::Strikethrough => PartialNode::Strikethrough {
                children: Vec::new(),
            },
            _ => PartialNode::Paragraph {
                children: Vec::new(),
            },
        }
    }

    fn push_inline(&mut self, node: InlineNode) {
        match self {
            PartialNode::Paragraph { children } => children.push(node),
            PartialNode::Heading { children, .. } => children.push(node),
            PartialNode::CodeBlock { code, .. } => {
                if let InlineNode::Text { value } = node {
                    code.push_str(&value);
                }
            }
            PartialNode::Link { children, .. } => children.push(node),
            PartialNode::Image { alt, .. } => {
                if let InlineNode::Text { value } = node {
                    alt.push_str(&value);
                }
            }
            PartialNode::Strong { children } => children.push(node),
            PartialNode::Emphasis { children } => children.push(node),
            PartialNode::Strikethrough { children } => children.push(node),
            PartialNode::TableCell { children } => children.push(node),
            PartialNode::Item { children } | PartialNode::Blockquote { children } => {
                if let Some(MarkdownNode::Paragraph {
                    children: para_children,
                    ..
                }) = children.last_mut()
                {
                    para_children.push(node);
                } else {
                    children.push(MarkdownNode::paragraph(vec![node]));
                }
            }
            _ => {}
        }
    }

    fn push_inlines(&mut self, mut nodes: Vec<InlineNode>) {
        match self {
            PartialNode::Paragraph { children } => children.append(&mut nodes),
            PartialNode::Heading { children, .. } => children.append(&mut nodes),
            PartialNode::CodeBlock { code, .. } => {
                for node in nodes {
                    if let InlineNode::Text { value } = node {
                        code.push_str(&value);
                    }
                }
            }
            PartialNode::Link { children, .. } => children.append(&mut nodes),
            PartialNode::Image { alt, .. } => {
                for node in nodes {
                    if let InlineNode::Text { value } = node {
                        alt.push_str(&value);
                    }
                }
            }
            PartialNode::Strong { children } => children.append(&mut nodes),
            PartialNode::Emphasis { children } => children.append(&mut nodes),
            PartialNode::Strikethrough { children } => children.append(&mut nodes),
            PartialNode::TableCell { children } => children.append(&mut nodes),
            PartialNode::Item { children } | PartialNode::Blockquote { children } => {
                if let Some(MarkdownNode::Paragraph {
                    children: para_children,
                    ..
                }) = children.last_mut()
                {
                    para_children.append(&mut nodes);
                } else {
                    children.push(MarkdownNode::paragraph(nodes));
                }
            }
            _ => {}
        }
    }

    fn push_child(&mut self, node: MarkdownNode) {
        match self {
            PartialNode::Blockquote { children } => children.push(node),
            PartialNode::Item { children } => children.push(node),
            _ => {}
        }
    }

    fn push_list_item(&mut self, item: Vec<MarkdownNode>) {
        if let PartialNode::List { items, .. } = self {
            items.push(item);
        }
    }

    fn push_table_cell(&mut self, cell: Vec<InlineNode>) {
        match self {
            PartialNode::TableHead { cells } => cells.push(cell),
            PartialNode::TableRow { cells } => cells.push(cell),
            _ => {}
        }
    }

    fn set_table_header(&mut self, header: Vec<Vec<InlineNode>>) {
        if let PartialNode::Table { header: h, .. } = self {
            *h = header;
        }
    }

    fn push_table_row(&mut self, row: Vec<Vec<InlineNode>>) {
        if let PartialNode::Table { rows, .. } = self {
            rows.push(row);
        }
    }

    fn finalize(self, _tag_end: TagEnd, is_streaming: bool) -> MarkdownNode {
        match self {
            PartialNode::Paragraph { children } => MarkdownNode::paragraph(children),
            PartialNode::Heading { level, children } => MarkdownNode::heading(level, children),
            PartialNode::CodeBlock { lang, code } => {
                let lang_str = lang.as_deref().unwrap_or("plaintext");
                let highlighted = if lang_str == "mermaid" || (is_streaming && code.len() > 4096) {
                    None
                } else {
                    highlight_code_block(&code, lang_str)
                };
                let mut node = MarkdownNode::code_block(lang, code);
                if let MarkdownNode::CodeBlock {
                    highlighted_html, ..
                } = &mut node
                {
                    *highlighted_html = highlighted;
                }
                node
            }
            PartialNode::Blockquote { children } => MarkdownNode::blockquote(children),
            PartialNode::List { ordered, items } => MarkdownNode::list(ordered, items),
            PartialNode::Table { header, rows } => {
                let mut node = MarkdownNode::table(header, rows);
                if let MarkdownNode::Table { wrapper_class, .. } = &mut node {
                    *wrapper_class = Some("vcp-scrollable no-swipe".to_string());
                }
                node
            }
            PartialNode::Link {
                href,
                title,
                children,
            } => {
                let needs_asset_conversion =
                    href.starts_with("vcp-asset:") || href.starts_with("/");
                let mut node = InlineNode::link(href, title, children);
                if let InlineNode::Link {
                    needs_asset_conversion: nac,
                    ..
                } = &mut node
                {
                    *nac = needs_asset_conversion;
                }
                MarkdownNode::paragraph(vec![node])
            }
            PartialNode::Image { src, alt, title } => {
                let needs_asset_conversion = src.starts_with("vcp-asset:") || src.starts_with("/");
                let mut node = InlineNode::image(src, alt, title);
                if let InlineNode::Image {
                    needs_asset_conversion: nac,
                    ..
                } = &mut node
                {
                    *nac = needs_asset_conversion;
                }
                MarkdownNode::paragraph(vec![node])
            }
            PartialNode::Strong { children } => {
                MarkdownNode::paragraph(vec![InlineNode::strong(children)])
            }
            PartialNode::Emphasis { children } => {
                MarkdownNode::paragraph(vec![InlineNode::emphasis(children)])
            }
            PartialNode::Strikethrough { children } => {
                MarkdownNode::paragraph(vec![InlineNode::strikethrough(children)])
            }
            _ => MarkdownNode::paragraph(Vec::new()),
        }
    }
}

fn process_text_magic(text: &str) -> Vec<InlineNode> {
    if !text.contains('@') && !text.contains('"') && !text.contains('“') && !text.contains('”')
    {
        return vec![InlineNode::text(text.to_string())];
    }

    let mut nodes = Vec::new();
    let mut last_end = 0;

    for cap in MAGIC_RE.captures_iter(text) {
        let m = cap.get(0).unwrap();
        if m.start() > last_end {
            nodes.push(InlineNode::text(text[last_end..m.start()].to_string()));
        }

        let node = if let Some(quote) = cap.get(1) {
            let quote_text = quote.as_str();
            let children = vec![InlineNode::text(quote_text.to_string())];
            InlineNode::vcp_custom("quote".to_string(), None, Some(children))
        } else if let Some(alert) = cap.get(2) {
            InlineNode::vcp_custom("alert".to_string(), Some(alert.as_str().to_string()), None)
        } else if let Some(tag) = cap.get(3) {
            InlineNode::vcp_custom(
                "highlight".to_string(),
                Some(tag.as_str().to_string()),
                None,
            )
        } else {
            unreachable!()
        };

        nodes.push(node);
        last_end = m.end();
    }

    if last_end < text.len() {
        nodes.push(InlineNode::text(text[last_end..].to_string()));
    }

    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_yaml_code_block(nodes: &[MarkdownNode]) -> bool {
        nodes.iter().any(|node| match node {
            MarkdownNode::CodeBlock { lang, code, .. } => {
                lang.as_deref() == Some("yaml") && code.contains("- name: 家宽分组")
            }
            MarkdownNode::Blockquote { children, .. } => contains_yaml_code_block(children),
            MarkdownNode::List { items, .. } => items
                .iter()
                .any(|item_nodes| contains_yaml_code_block(item_nodes)),
            _ => false,
        })
    }

    fn collect_yaml_code_blocks(nodes: &[MarkdownNode], codes: &mut Vec<String>) {
        for node in nodes {
            match node {
                MarkdownNode::CodeBlock { lang, code, .. } if lang.as_deref() == Some("yaml") => {
                    codes.push(code.clone());
                }
                MarkdownNode::Blockquote { children, .. } => {
                    collect_yaml_code_blocks(children, codes)
                }
                MarkdownNode::List { items, .. } => {
                    for item_nodes in items {
                        collect_yaml_code_blocks(item_nodes, codes);
                    }
                }
                _ => {}
            }
        }
    }

    fn contains_raw_html(nodes: &[MarkdownNode], needle: &str) -> bool {
        nodes.iter().any(|node| match node {
            MarkdownNode::RawHtml { content, .. } => content.contains(needle),
            MarkdownNode::Paragraph { children, .. } | MarkdownNode::Heading { children, .. } => {
                contains_inline_raw_html(children, needle)
            }
            MarkdownNode::Blockquote { children, .. } => contains_raw_html(children, needle),
            MarkdownNode::List { items, .. } => items
                .iter()
                .any(|item_nodes| contains_raw_html(item_nodes, needle)),
            MarkdownNode::Table { header, rows, .. } => {
                header
                    .iter()
                    .any(|cell| contains_inline_raw_html(cell, needle))
                    || rows.iter().any(|row| {
                        row.iter()
                            .any(|cell| contains_inline_raw_html(cell, needle))
                    })
            }
            _ => false,
        })
    }

    fn contains_inline_raw_html(nodes: &[InlineNode], needle: &str) -> bool {
        nodes.iter().any(|node| match node {
            InlineNode::RawHtmlInline { content, .. } => content.contains(needle),
            InlineNode::Strong { children, .. }
            | InlineNode::Emphasis { children, .. }
            | InlineNode::Strikethrough { children, .. }
            | InlineNode::Link { children, .. } => contains_inline_raw_html(children, needle),
            InlineNode::VcpCustom {
                children: Some(children),
                ..
            } => contains_inline_raw_html(children, needle),
            _ => false,
        })
    }

    #[test]
    fn parses_html_container_with_close_tag_stuck_to_code_fence() {
        let input = r#"<div id="vcp-root" style="padding:20px;">
<p>改法有俩：</p>
<p>要是想留着这个功能，就在proxy-groups里补一个：</p>

</div>```yaml
  - name: 家宽分组
    type: select
    use:
      - proxy4
```

<p>放在手动选择那个组前面就行。</p>
</div>"#;

        let nodes = parse_markdown_to_ast(input);

        assert!(matches!(
            nodes.first(),
            Some(MarkdownNode::RawHtml { content, .. }) if content.contains("id=\"vcp-root\"")
        ));
        assert!(
            contains_yaml_code_block(&nodes),
            "expected stuck fence after </div> to render as yaml code block, got {nodes:#?}"
        );
        assert!(
            contains_raw_html(&nodes, "放在手动选择那个组前面就行"),
            "expected content after code fence to remain inside rendered AST, got {nodes:#?}"
        );
        assert!(
            contains_raw_html(&nodes, "</div>"),
            "expected outer HTML container to retain a closing tag, got {nodes:#?}"
        );
    }

    #[test]
    fn parses_inline_html_container_stuck_to_code_fence_without_swallowing_tail() {
        let input = r#"AI render probe:

The next boundary is intentionally stuck together:
<div class="vcp-debug-probe"><strong>HTML container</strong></div>```yaml
name: render-probe
copy_button: should_not_send
```

<button data-vcp-ui-control="true" data-vcp-copy-code="render-probe">Copy code</button>

[[点击按钮:继续]]

- The YAML block above should render as a code block.
- The copy button should not be sent as an AI button click."#;

        let nodes = parse_markdown_to_ast(input);
        let mut yaml_codes = Vec::new();
        collect_yaml_code_blocks(&nodes, &mut yaml_codes);

        assert_eq!(
            yaml_codes.len(),
            1,
            "expected exactly one yaml code block, got {nodes:#?}"
        );
        assert!(yaml_codes[0].contains("name: render-probe"));
        assert!(yaml_codes[0].contains("copy_button: should_not_send"));
        assert!(
            !yaml_codes[0].contains("<button"),
            "copy button HTML was swallowed by yaml code block: {nodes:#?}"
        );
        assert!(
            !yaml_codes[0].contains("[[点击按钮:继续]]"),
            "AI button marker was swallowed by yaml code block: {nodes:#?}"
        );
        assert!(
            contains_raw_html(&nodes, "vcp-debug-probe"),
            "expected inline HTML container to render as raw HTML, got {nodes:#?}"
        );
        assert!(
            contains_raw_html(&nodes, "data-vcp-copy-code"),
            "expected copy button HTML to remain renderable after the yaml block, got {nodes:#?}"
        );
    }

    #[test]
    fn preserves_inline_css_token_boundaries_in_raw_html_nodes() {
        let inline_style = concat!(
            "background:linear-gradient(180deg,#fdf6e9 0%,#fcebd4 40%,",
            "#f9e0c0 100%);padding:20px 16px 24px;opacity:1"
        );
        let input = format!(r#"<div id="vcp-root" style="{inline_style}"><p>visible</p></div>"#);

        let nodes = parse_markdown_to_ast(&input);

        assert!(
            contains_raw_html(&nodes, inline_style),
            "inline CSS changed while building raw HTML AST nodes: {nodes:#?}"
        );
        let serialized = serde_json::to_string(&nodes).expect("serialize markdown AST");
        assert!(
            serialized.contains(inline_style),
            "inline CSS changed while serializing markdown AST: {serialized}"
        );
    }
}
