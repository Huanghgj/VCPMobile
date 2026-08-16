use regex::Regex;
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtectedBlock {
    Tool,
    Thought,
    Think,
    ToolResult,
    Diary,
    HtmlFence,
    CodeFence,
    TildeHtmlFence,
    TildeCodeFence,
    RoleDivider,
}

lazy_static::lazy_static! {
    static ref TILDE_HTML_FENCE_START: Regex =
        Regex::new(r"(?im)^[ \t]*~~~html[ \t]*\r?$").unwrap();
    static ref TILDE_CODE_FENCE_START: Regex =
        Regex::new(r"(?im)^[ \t]*~~~[a-zA-Z0-9-]*[ \t]*\r?$").unwrap();
    static ref TILDE_CODE_FENCE_END: Regex =
        Regex::new(r"(?im)^[ \t]*~~~[ \t]*\r?$").unwrap();
}

const TOOL_REQUEST_MARKER: &str = "<<<[TOOL_REQUEST]>>>";

/// Repairs renderable AI output before it is compiled and persisted.
///
/// The important behavior is boundary-aware repair: if a naked HTML render block
/// is left open before a VCP control block, the HTML is closed before the marker
/// so the parser can still recognize the control block.
pub fn repair_message_content_before_persist(content: &str) -> String {
    if !content.contains('<') && !content.contains("```") && !content.contains("~~~") {
        return content.to_string();
    }

    let mut repaired = String::with_capacity(content.len() + 64);
    let mut cursor = 0;

    while cursor < content.len() {
        let remaining = &content[cursor..];
        let Some((start, end, block_type)) = find_earliest_protected_block(remaining) else {
            repaired.push_str(&repair_html_fragment(remaining));
            break;
        };

        if start > 0 {
            repaired.push_str(&repair_html_fragment(&remaining[..start]));
        }

        let consumed = match block_type {
            ProtectedBlock::HtmlFence => append_repaired_fence_block(
                &mut repaired,
                remaining,
                start,
                end,
                true,
                &crate::vcp_modules::content_parser::GENERIC_CODE_FENCE_END,
            ),
            ProtectedBlock::CodeFence => append_repaired_fence_block(
                &mut repaired,
                remaining,
                start,
                end,
                false,
                &crate::vcp_modules::content_parser::GENERIC_CODE_FENCE_END,
            ),
            ProtectedBlock::TildeHtmlFence => append_repaired_fence_block(
                &mut repaired,
                remaining,
                start,
                end,
                true,
                &TILDE_CODE_FENCE_END,
            ),
            ProtectedBlock::TildeCodeFence => append_repaired_fence_block(
                &mut repaired,
                remaining,
                start,
                end,
                false,
                &TILDE_CODE_FENCE_END,
            ),
            ProtectedBlock::RoleDivider => {
                repaired.push_str(&remaining[start..end]);
                end
            }
            ProtectedBlock::Tool => append_protected_until_end(
                &mut repaired,
                remaining,
                start,
                end,
                &crate::vcp_modules::content_parser::TOOL_END,
            ),
            ProtectedBlock::Thought => append_protected_until_end(
                &mut repaired,
                remaining,
                start,
                end,
                &crate::vcp_modules::content_parser::THOUGHT_END,
            ),
            ProtectedBlock::Think => append_protected_until_end(
                &mut repaired,
                remaining,
                start,
                end,
                &crate::vcp_modules::content_parser::THINK_END,
            ),
            ProtectedBlock::ToolResult => append_protected_until_end(
                &mut repaired,
                remaining,
                start,
                end,
                &crate::vcp_modules::content_parser::TOOL_RESULT_END,
            ),
            ProtectedBlock::Diary => append_protected_until_end(
                &mut repaired,
                remaining,
                start,
                end,
                &crate::vcp_modules::content_parser::DIARY_END,
            ),
        };

        cursor += consumed;
    }

    repaired
}

/// Repairs a standalone HTML fragment by inserting missing close tags.
pub fn repair_html_fragment(fragment: &str) -> String {
    if !fragment.contains('<') {
        return fragment.to_string();
    }

    let normalized_fragment = repair_premature_vcp_root_closes(fragment);
    let fragment = normalized_fragment.as_ref();

    let mut output = String::with_capacity(fragment.len() + 32);
    let mut stack: Vec<String> = Vec::new();
    let mut cursor = 0;

    while cursor < fragment.len() {
        let Some(relative_lt) = fragment[cursor..].find('<') else {
            output.push_str(&fragment[cursor..]);
            break;
        };
        let lt = cursor + relative_lt;
        output.push_str(&fragment[cursor..lt]);

        if fragment[lt..].starts_with("<!--") {
            if let Some(relative_end) = fragment[lt + 4..].find("-->") {
                let end = lt + 4 + relative_end + 3;
                output.push_str(&fragment[lt..end]);
                cursor = end;
                continue;
            }
            output.push_str(&fragment[lt..]);
            break;
        }

        let Some(gt) = find_tag_end(fragment, lt) else {
            output.push_str(&fragment[lt..]);
            break;
        };
        let tag_text = &fragment[lt..=gt];

        let Some(tag) = parse_html_tag(tag_text) else {
            output.push_str(tag_text);
            cursor = gt + 1;
            continue;
        };

        if tag.closing {
            if let Some(pos) = stack.iter().rposition(|open| open == &tag.name) {
                while stack.len() > pos + 1 {
                    if let Some(open) = stack.pop() {
                        output.push_str("</");
                        output.push_str(&open);
                        output.push('>');
                    }
                }
                stack.pop();
            }
            output.push_str(tag_text);
        } else {
            if should_auto_close_before(&tag.name, stack.last().map(String::as_str)) {
                if let Some(open) = stack.pop() {
                    output.push_str("</");
                    output.push_str(&open);
                    output.push('>');
                }
            }

            output.push_str(tag_text);
            if !tag.self_closing && is_raw_text_tag(&tag.name) {
                if let Some(raw_end) = find_raw_text_element_end(fragment, gt + 1, &tag.name) {
                    output.push_str(&fragment[gt + 1..raw_end]);
                    cursor = raw_end;
                    continue;
                }
            }
            if !tag.self_closing && !is_void_tag(&tag.name) {
                stack.push(tag.name);
            }
        }

        cursor = gt + 1;
    }

    for tag in stack.iter().rev() {
        output.push_str("</");
        output.push_str(tag);
        output.push('>');
    }

    output
}

pub(crate) fn is_vcp_root_open_tag(tag_text: &str) -> bool {
    lazy_static::lazy_static! {
        static ref VCP_ROOT_ID: Regex = Regex::new(
            r#"(?i)\bid\s*=\s*(?:\"vcp-root\"|'vcp-root'|vcp-root(?:[\s/>]|$))"#,
        )
        .unwrap();
    }

    parse_html_tag(tag_text)
        .is_some_and(|tag| !tag.closing && tag.name == "div" && VCP_ROOT_ID.is_match(tag_text))
}

/// Keeps the designated rich-message root open across later rich blocks when
/// an early `</div>` would otherwise split the rendered response.
pub(crate) fn repair_premature_vcp_root_closes(fragment: &str) -> Cow<'_, str> {
    if !fragment.contains("vcp-root") {
        return Cow::Borrowed(fragment);
    }

    let mut output = String::with_capacity(fragment.len());
    let mut cursor = 0;
    let mut active_root: Option<(String, usize)> = None;
    let mut changed = false;

    while cursor < fragment.len() {
        let Some(relative_lt) = fragment[cursor..].find('<') else {
            output.push_str(&fragment[cursor..]);
            break;
        };
        let lt = cursor + relative_lt;
        output.push_str(&fragment[cursor..lt]);

        if fragment[lt..].starts_with("<!--") {
            if let Some(relative_end) = fragment[lt + 4..].find("-->") {
                let end = lt + 4 + relative_end + 3;
                output.push_str(&fragment[lt..end]);
                cursor = end;
                continue;
            }
            output.push_str(&fragment[lt..]);
            break;
        }

        let Some(gt) = find_tag_end(fragment, lt) else {
            output.push_str(&fragment[lt..]);
            break;
        };
        let tag_text = &fragment[lt..=gt];
        let Some(tag) = parse_html_tag(tag_text) else {
            output.push_str(tag_text);
            cursor = gt + 1;
            continue;
        };

        if active_root.is_none() && is_vcp_root_open_tag(tag_text) {
            active_root = Some((tag.name.clone(), 1));
            output.push_str(tag_text);
            cursor = gt + 1;
            continue;
        }

        if let Some((root_tag, depth)) = active_root.as_mut() {
            if tag.name == *root_tag && !tag.self_closing && !is_void_tag(&tag.name) {
                if tag.closing {
                    if *depth == 1
                        && (has_later_orphan_close(fragment, gt + 1, root_tag.as_str())
                            || has_renderable_vcp_root_continuation(fragment, gt + 1))
                    {
                        changed = true;
                        cursor = gt + 1;
                        continue;
                    }

                    *depth = depth.saturating_sub(1);
                    if *depth == 0 {
                        active_root = None;
                    }
                } else {
                    *depth += 1;
                }
            }
        }

        if !tag.closing && !tag.self_closing && is_raw_text_tag(&tag.name) {
            if let Some(raw_end) = find_raw_text_element_end(fragment, gt + 1, &tag.name) {
                output.push_str(&fragment[lt..raw_end]);
                cursor = raw_end;
                continue;
            }
        }

        output.push_str(tag_text);
        cursor = gt + 1;
    }

    if changed {
        Cow::Owned(output)
    } else {
        Cow::Borrowed(fragment)
    }
}

fn has_renderable_vcp_root_continuation(fragment: &str, start: usize) -> bool {
    lazy_static::lazy_static! {
        static ref RICH_HTML_CONTINUATION: Regex = Regex::new(
            r"(?is)^<(?:div|section|article|header|footer|main|aside|figure|figcaption|img|p|span|table|ul|ol)\b",
        )
        .unwrap();
    }

    fragment.get(start..).is_some_and(|rest| {
        let rest = rest.trim_start();
        if let Some(gt) = find_tag_end(rest, 0) {
            if is_vcp_root_open_tag(&rest[..=gt]) {
                return false;
            }
        }
        RICH_HTML_CONTINUATION.is_match(rest)
    })
}

fn has_later_orphan_close(fragment: &str, start: usize, tag_name: &str) -> bool {
    let mut depth = 0usize;
    let mut cursor = start;

    while cursor < fragment.len() {
        let Some(relative_lt) = fragment[cursor..].find('<') else {
            break;
        };
        let lt = cursor + relative_lt;

        if fragment[lt..].starts_with("<!--") {
            if let Some(relative_end) = fragment[lt + 4..].find("-->") {
                cursor = lt + 4 + relative_end + 3;
                continue;
            }
            break;
        }

        let Some(gt) = find_tag_end(fragment, lt) else {
            break;
        };
        let tag_text = &fragment[lt..=gt];
        cursor = gt + 1;

        let Some(tag) = parse_html_tag(tag_text) else {
            continue;
        };
        if !tag.closing && !tag.self_closing && is_raw_text_tag(&tag.name) {
            if let Some(raw_end) = find_raw_text_element_end(fragment, gt + 1, &tag.name) {
                cursor = raw_end;
                continue;
            }
        }
        if tag.name != tag_name || tag.self_closing || is_void_tag(&tag.name) {
            continue;
        }

        if tag.closing {
            if depth == 0 {
                return true;
            }
            depth -= 1;
        } else {
            depth += 1;
        }
    }

    false
}

/// Finds a tool request that is stuck directly to a rendered HTML block. The
/// regular protocol regex remains line-anchored so prose and code examples do
/// not become control blocks; this narrow path only accepts a real closing HTML
/// tag outside literal HTML elements such as script/style/pre/code.
pub(crate) fn find_stuck_tool_request_start(text: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;

    while let Some(relative_start) = text[search_from..].find(TOOL_REQUEST_MARKER) {
        let start = search_from + relative_start;
        let end = start + TOOL_REQUEST_MARKER.len();
        let prefix = text[..start].trim_end();
        let Some(tag_start) = prefix.rfind('<') else {
            search_from = end;
            continue;
        };
        let Some(tag) = parse_html_tag(&prefix[tag_start..]) else {
            search_from = end;
            continue;
        };
        let is_block_boundary = tag.closing
            && matches!(
                tag.name.as_str(),
                "div"
                    | "section"
                    | "article"
                    | "header"
                    | "footer"
                    | "main"
                    | "aside"
                    | "figure"
                    | "figcaption"
                    | "details"
                    | "p"
                    | "w2g"
                    | "catsay"
                    | "bginfor"
            );

        if is_block_boundary && !is_inside_html_raw_text_or_tag(text, start) {
            return Some((start, end));
        }
        search_from = end;
    }

    None
}

/// Finds an unambiguous tool boundary while an outer generated HTML container
/// is still open. A line-anchored protocol marker is valid even when the model
/// forgot to close the surrounding HTML first, but markers inside literal HTML
/// elements or tag attributes must remain document text.
pub(crate) fn find_tool_request_boundary_in_html(text: &str) -> Option<(usize, usize)> {
    let anchored = crate::vcp_modules::content_parser::TOOL_START
        .find_iter(text)
        .find(|marker| !is_inside_html_raw_text_or_tag(text, marker.start()))
        .map(|marker| (marker.start(), marker.end()));
    let stuck = find_stuck_tool_request_start(text);

    match (anchored, stuck) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

fn is_inside_html_raw_text_or_tag(text: &str, end: usize) -> bool {
    let mut cursor = 0;
    let mut raw_text_tag: Option<String> = None;

    while cursor < end {
        let Some(relative_lt) = text[cursor..end].find('<') else {
            break;
        };
        let lt = cursor + relative_lt;

        if text[lt..].starts_with("<!--") {
            let Some(relative_comment_end) = text[lt + 4..].find("-->") else {
                return true;
            };
            let comment_end = lt + 4 + relative_comment_end + 3;
            if comment_end > end {
                return true;
            }
            cursor = comment_end;
            continue;
        }

        let Some(gt) = find_tag_end(text, lt) else {
            return true;
        };
        if gt >= end {
            return true;
        }
        let tag_text = &text[lt..=gt];
        if let Some(tag) = parse_html_tag(tag_text) {
            if let Some(active_raw_tag) = raw_text_tag.as_deref() {
                if tag.closing && tag.name == active_raw_tag {
                    raw_text_tag = None;
                }
            } else if !tag.closing
                && !tag.self_closing
                && matches!(
                    tag.name.as_str(),
                    "script" | "style" | "textarea" | "title" | "pre" | "code"
                )
            {
                raw_text_tag = Some(tag.name);
            }
        }
        cursor = gt + 1;
    }

    raw_text_tag.is_some()
}

fn find_earliest_protected_block(text: &str) -> Option<(usize, usize, ProtectedBlock)> {
    let checks: [(&Regex, ProtectedBlock); 9] = [
        (
            &crate::vcp_modules::content_parser::THOUGHT_START,
            ProtectedBlock::Thought,
        ),
        (
            &crate::vcp_modules::content_parser::THINK_START,
            ProtectedBlock::Think,
        ),
        (
            &crate::vcp_modules::content_parser::TOOL_RESULT_START,
            ProtectedBlock::ToolResult,
        ),
        (
            &crate::vcp_modules::content_parser::DIARY_START,
            ProtectedBlock::Diary,
        ),
        (
            &crate::vcp_modules::content_parser::HTML_FENCE_START,
            ProtectedBlock::HtmlFence,
        ),
        (&TILDE_HTML_FENCE_START, ProtectedBlock::TildeHtmlFence),
        (
            &crate::vcp_modules::content_parser::ROLE_DIVIDER,
            ProtectedBlock::RoleDivider,
        ),
        (
            &crate::vcp_modules::content_parser::GENERIC_CODE_FENCE_START,
            ProtectedBlock::CodeFence,
        ),
        (&TILDE_CODE_FENCE_START, ProtectedBlock::TildeCodeFence),
    ];

    let mut earliest: Option<(usize, usize, ProtectedBlock)> = None;
    for (regex, block_type) in checks {
        if let Some(m) = regex.find(text) {
            if earliest
                .as_ref()
                .is_none_or(|(start, _, _)| m.start() < *start)
            {
                earliest = Some((m.start(), m.end(), block_type));
            }
        }
    }
    if let Some((start, end)) = find_tool_request_boundary_in_html(text) {
        if earliest
            .as_ref()
            .is_none_or(|(current_start, _, _)| start < *current_start)
        {
            earliest = Some((start, end, ProtectedBlock::Tool));
        }
    }
    earliest
}

fn append_protected_until_end(
    output: &mut String,
    text: &str,
    start: usize,
    marker_end: usize,
    end_regex: &Regex,
) -> usize {
    let search_area = &text[marker_end..];
    if let Some(end_marker) = end_regex.find(search_area) {
        let end = marker_end + end_marker.end();
        output.push_str(&text[start..end]);
        end
    } else {
        output.push_str(&text[start..]);
        text.len()
    }
}

fn append_repaired_fence_block(
    output: &mut String,
    text: &str,
    start: usize,
    marker_end: usize,
    repair_inner_html: bool,
    close_regex: &Regex,
) -> usize {
    let opening = &text[start..marker_end];
    let search_area = &text[marker_end..];
    output.push_str(opening);

    if let Some(close_marker) = close_regex.find(search_area) {
        let body = &search_area[..close_marker.start()];
        if repair_inner_html {
            output.push_str(&repair_html_fragment(body));
        } else {
            output.push_str(body);
        }
        output.push_str(&search_area[close_marker.start()..close_marker.end()]);
        marker_end + close_marker.end()
    } else {
        if repair_inner_html {
            output.push_str(&repair_html_fragment(search_area));
        } else {
            output.push_str(search_area);
        }
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&closing_fence_marker(opening));
        output.push('\n');
        text.len()
    }
}

fn closing_fence_marker(opening: &str) -> String {
    let trimmed = opening.trim_start();
    let fence_char = trimmed.chars().next().unwrap_or('`');
    if fence_char != '`' && fence_char != '~' {
        return "```".to_string();
    }
    let count = trimmed.chars().take_while(|ch| *ch == fence_char).count();
    std::iter::repeat_n(fence_char, count.max(3)).collect()
}

fn find_tag_end(text: &str, lt: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    let scan_start = lt.checked_add(1)?;
    let suffix = text.get(scan_start..)?;
    for (offset, ch) in suffix.char_indices() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '>' => return Some(scan_start + offset),
            None => {}
        }
    }
    None
}

struct HtmlTag {
    name: String,
    closing: bool,
    self_closing: bool,
}

fn parse_html_tag(tag_text: &str) -> Option<HtmlTag> {
    if tag_text.len() < 3 || !tag_text.starts_with('<') || !tag_text.ends_with('>') {
        return None;
    }

    let mut inner = tag_text[1..tag_text.len() - 1].trim_start();
    if inner.starts_with('!') || inner.starts_with('?') {
        return None;
    }

    let closing = inner.starts_with('/');
    if closing {
        inner = inner[1..].trim_start();
    }

    let mut chars = inner.char_indices();
    let first = chars.next()?.1;
    if !first.is_ascii_alphabetic() {
        return None;
    }

    let mut name_end = first.len_utf8();
    for (idx, ch) in chars {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ':' {
            name_end = idx + ch.len_utf8();
        } else {
            break;
        }
    }

    let name = inner[..name_end].to_ascii_lowercase();
    let self_closing = tag_text.trim_end_matches('>').trim_end().ends_with('/');

    Some(HtmlTag {
        name,
        closing,
        self_closing,
    })
}

fn is_raw_text_tag(name: &str) -> bool {
    matches!(name, "script" | "style" | "textarea" | "title")
}

fn find_raw_text_element_end(text: &str, body_start: usize, tag_name: &str) -> Option<usize> {
    let suffix = text.get(body_start..)?;
    let lowercase = suffix.to_ascii_lowercase();
    let needle = format!("</{tag_name}");
    let mut search_from = 0;

    while let Some(relative_close) = lowercase[search_from..].find(&needle) {
        let close_start = body_start + search_from + relative_close;
        let name_end = close_start + needle.len();
        let boundary = text.get(name_end..)?.chars().next()?;
        if boundary.is_ascii_whitespace() || boundary == '>' {
            let close_end = find_tag_end(text, close_start)? + 1;
            return Some(close_end);
        }
        search_from += relative_close + needle.len();
    }

    None
}

fn is_void_tag(name: &str) -> bool {
    matches!(
        name,
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

fn should_auto_close_before(next: &str, current: Option<&str>) -> bool {
    matches!(
        (current, next),
        (Some("p"), "p")
            | (Some("li"), "li")
            | (Some("dt"), "dt" | "dd")
            | (Some("dd"), "dt" | "dd")
            | (Some("tr"), "tr")
            | (Some("th"), "th" | "td")
            | (Some("td"), "th" | "td")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_code_block(nodes: &[crate::vcp_modules::pre_renderer::MarkdownNode]) -> bool {
        nodes.iter().any(|node| match node {
            crate::vcp_modules::pre_renderer::MarkdownNode::CodeBlock { .. } => true,
            crate::vcp_modules::pre_renderer::MarkdownNode::Blockquote { children, .. } => {
                contains_code_block(children)
            }
            crate::vcp_modules::pre_renderer::MarkdownNode::List { items, .. } => items
                .iter()
                .any(|item_nodes| contains_code_block(item_nodes)),
            _ => false,
        })
    }

    #[test]
    fn closes_raw_html_before_tool_block() {
        let input = concat!(
            "<div class=\"card\"><span>hello\n",
            "<<<[TOOL_REQUEST]>>>\n",
            "<tool_name>ComfyUIGen</tool_name>\n",
            "<<<[END_TOOL_REQUEST]>>>"
        );
        let repaired = repair_message_content_before_persist(input);

        assert!(repaired.contains("<span>hello\n</span></div><<<[TOOL_REQUEST]>>>"));
        assert!(repaired.contains("<tool_name>ComfyUIGen</tool_name>"));
        assert!(!repaired.contains("</tool_name></tool_name>"));

        let blocks = crate::vcp_modules::content_parser::parse_content(&repaired);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            blocks[0],
            crate::vcp_modules::content_parser::ContentBlock::Markdown { .. }
        ));
        assert!(matches!(
            blocks[1],
            crate::vcp_modules::content_parser::ContentBlock::ToolUse { .. }
        ));
    }

    #[test]
    fn closes_generated_root_around_stuck_tool_request() {
        let input = concat!(
            "<div id=\"vcp-root\"><div data-probe=\"body\">visible</div>",
            "<<<[TOOL_REQUEST]>>>\n",
            "maid:「始」Nova「末」,\n",
            "tool_name:「始」DailyNote「末」,\n",
            "command:「始」create「末」,\n",
            "Date:「始」2026-08-02「末」,\n",
            "Content:「始ESCAPE」diary body「末ESCAPE」\n",
            "<<<[END_TOOL_REQUEST]>>>\n",
            "<w2g><catsay>tail card</catsay></w2g></div>"
        );

        let repaired = repair_message_content_before_persist(input);

        assert!(
            repaired.contains("<div data-probe=\"body\">visible</div></div><<<[TOOL_REQUEST]>>>"),
            "outer generated HTML must close before the tool marker: {repaired}"
        );
        assert!(repaired.contains("<w2g><catsay>tail card</catsay></w2g>"));

        let line_separated = input.replace(
            "<div data-probe=\"body\">visible</div><<<[TOOL_REQUEST]>>>",
            "<div data-probe=\"body\">visible</div>\n<<<[TOOL_REQUEST]>>>",
        );
        assert!(find_stuck_tool_request_start(&line_separated).is_some());
    }

    #[test]
    fn does_not_extract_tool_marker_from_script_text() {
        let input = concat!(
            "<div><script>",
            "const example = '</div><<<[TOOL_REQUEST]>>>';",
            "</script></div>"
        );

        assert!(find_stuck_tool_request_start(input).is_none());
        assert_eq!(repair_message_content_before_persist(input), input);
    }

    #[test]
    fn closes_unclosed_html_before_line_anchored_tool_request() {
        let input = concat!(
            "<div id=\"vcp-root\"><p>visible</p><w2g><catsay>tail text\n",
            "<<<[TOOL_REQUEST]>>>\n",
            "tool_name:「始」ComfyUIGen「末」"
        );

        let repaired = repair_message_content_before_persist(input);

        assert!(
            repaired.contains("<w2g><catsay>tail text\n</catsay></w2g></div><<<[TOOL_REQUEST]>>>"),
            "open HTML must close before the protocol boundary: {repaired}"
        );
    }

    #[test]
    fn does_not_extract_line_anchored_tool_marker_from_preformatted_html() {
        let input = concat!(
            "<div id=\"vcp-root\"><pre><code>example\n",
            "<<<[TOOL_REQUEST]>>>\n",
            "tool_name: demo\n",
            "</code></pre></div>"
        );

        assert!(find_tool_request_boundary_in_html(input).is_none());
        assert_eq!(repair_message_content_before_persist(input), input);
    }

    #[test]
    fn closes_html_inside_unclosed_html_fence() {
        let input = "```html\n<div><span>hello";
        let repaired = repair_message_content_before_persist(input);

        assert_eq!(repaired, "```html\n<div><span>hello</span></div>\n```\n");
    }

    #[test]
    fn leaves_tool_payload_unscanned() {
        let input = concat!(
            "<<<[TOOL_REQUEST]>>>\n",
            "prompt:「始」<lora:test:0.8>, <not_a_tag「末」\n",
            "<<<[END_TOOL_REQUEST]>>>"
        );
        let repaired = repair_message_content_before_persist(input);

        assert_eq!(repaired, input);
    }

    #[test]
    fn leaves_non_html_tilde_fence_unscanned() {
        let input = "~~~text\n<div><span>example\n~~~";
        let repaired = repair_message_content_before_persist(input);

        assert_eq!(repaired, input);
    }

    #[test]
    fn dialogue_html_text_does_not_become_code_block() {
        let input = concat!(
            "<div style=\"box-sizing:border-box; max-width:88%; background:#fff0f7;\">\n",
            "      『今日确是凡间的“儿童节”不假……可、可妾身都天庭八岁了，又不是凡间那种什么都不懂的三岁稚童！兄长莫要老是拿妾身当小娃娃打趣……』\n",
            "      <br><br>\n",
            "      『不过……既然是过节，凡间的娃娃们都有礼物拿，那妾身……妾身是不是也可以要点好吃的仙果糖酥？』\n",
            "    </div>"
        );
        let blocks = crate::vcp_modules::content_parser::parse_content(input);

        let has_code = blocks.iter().any(|block| {
            matches!(
                block,
                crate::vcp_modules::content_parser::ContentBlock::Markdown {
                    nodes: Some(nodes),
                    ..
                } if contains_code_block(nodes)
            )
        });
        assert!(!has_code, "{blocks:#?}");
    }

    #[test]
    fn keeps_vcp_root_open_until_later_orphan_close() {
        let input = concat!(
            "<div id=\"vcp-root\"><style>@keyframes fade { from { opacity: 0 } to { opacity: 1 } }</style>",
            "<div data-probe=\"first\">first</div></div>",
            "<div data-probe=\"second\">second</div>",
            "</div>"
        );

        let repaired = repair_html_fragment(input);

        assert_eq!(
            repaired,
            concat!(
                "<div id=\"vcp-root\"><style>@keyframes fade { from { opacity: 0 } to { opacity: 1 } }</style>",
                "<div data-probe=\"first\">first</div>",
                "<div data-probe=\"second\">second</div>",
                "</div>"
            )
        );
    }

    #[test]
    fn keeps_streaming_vcp_root_open_across_later_rich_blocks() {
        let partial = concat!(
            "<div id=\"vcp-root\"><div data-probe=\"first\">first</div></div>",
            "<div data-probe=\"second\">second</div>"
        );

        let repaired = repair_html_fragment(partial);

        assert_eq!(
            repaired,
            concat!(
                "<div id=\"vcp-root\"><div data-probe=\"first\">first</div>",
                "<div data-probe=\"second\">second</div></div>"
            )
        );
    }

    #[test]
    fn keeps_sequential_vcp_roots_as_separate_documents() {
        let input = concat!(
            "<div id=\"vcp-root\" style=\"display:none\"></div>",
            "<div id=\"vcp-root\"><p data-probe=\"final\">visible</p></div>"
        );

        let repaired = repair_html_fragment(input);

        assert_eq!(repaired, input);
        assert!(!repaired.contains("display:none\"><div id=\"vcp-root\""));
    }
}
