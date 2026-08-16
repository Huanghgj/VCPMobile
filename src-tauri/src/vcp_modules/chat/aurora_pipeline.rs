use serde::{Deserialize, Serialize};

use crate::vcp_modules::stream_block_parser::{StreamBlock, StreamBlockParser};

/// Aurora 语义沉淀更新，由 Rust 流式管道推送到前端
/// 采用稀疏序列化：只在字段有变化时才包含在 JSON 中，减少 IPC payload
#[derive(Debug, Serialize, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuroraUpdate {
    /// 每条消息内单调递增，用于前端丢弃重复或乱序 IPC 帧。
    pub sequence: u64,
    /// 本帧新闭合的语义块；已沉淀的历史块不再重复传输。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable_blocks_delta: Option<Vec<StreamBlock>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub stable_changed: bool,
    /// 唯一会变化的尾块，只携带原文和语义类型，不携带逐帧 AST。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_block: Option<StreamBlock>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tail_changed: bool,
    /// 全量内容的流式增量（正常流式中发送，避免依赖原始 data 事件重复渲染）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_delta: Option<String>,
    /// 全量内容（仅终结事件时发送，正常流式中省略）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Aurora 语义沉淀缓冲区
/// 职责：用轻量块解析器识别已闭合/未闭合块，前端增量接收
pub struct AuroraBuffer {
    pub full_text: String,
    pub stable_blocks: Vec<StreamBlock>,
    pub tail_content: String,
    pub tail_block: Option<StreamBlock>,
    parser: StreamBlockParser,
    is_finishing: bool,
}

impl AuroraBuffer {
    pub fn new() -> Self {
        Self {
            full_text: String::new(),
            stable_blocks: Vec::new(),
            tail_content: String::new(),
            tail_block: None,
            parser: StreamBlockParser::new(),
            is_finishing: false,
        }
    }

    /// 将新的文本块追加到全文
    pub fn append_chunk(&mut self, chunk: &str) {
        self.full_text.push_str(chunk);
    }

    /// 运行块解析器，识别已闭合块和未闭合尾部
    /// 返回 (stable_changed, tail_changed)
    pub fn process_queue(&mut self) -> (bool, bool) {
        if self.is_finishing {
            return (false, false);
        }

        let prev_stable_count = self.stable_blocks.len();

        // 1. 增量解析全文，产出本次新增的已闭合块 + 尾部纯文本
        let (new_blocks, new_tail) = self.parser.process(&self.full_text);
        let tail_changed = self.tail_content != new_tail;

        if !new_blocks.is_empty() {
            self.stable_blocks.extend(new_blocks);
        }

        self.tail_content = new_tail;

        // 活动 tail 只携带原文和语义类型。Markdown AST 在块闭合后生成，
        // 避免逐帧 parse + hash + diff + JSON AST 放大 IPC 和 WebView 工作量。
        if !self.tail_content.is_empty() {
            let render_tail =
                crate::vcp_modules::stream_block_parser::strip_incomplete_tool_request_suffix(
                    &self.tail_content,
                );
            if crate::vcp_modules::stream_block_parser::is_incomplete_tool_request_marker(
                &self.tail_content,
            ) || render_tail.trim().is_empty()
            {
                self.tail_block = None;
            } else {
                let semantic_tail_block =
                    StreamBlockParser::build_incomplete_semantic_tail_block_lightweight(
                        render_tail,
                    );
                self.tail_block = semantic_tail_block.or_else(|| {
                    let content =
                        if crate::vcp_modules::content_parser::is_html_tag_block(render_tail) {
                            crate::vcp_modules::render_repair::repair_html_fragment(render_tail)
                        } else {
                            render_tail.to_string()
                        };
                    let hash = crate::vcp_modules::sync_hash::HashAggregator::compute_content_hash(
                        &content,
                    );
                    Some(StreamBlock::markdown(content, None, hash))
                });
            }
        } else {
            self.tail_block = None;
        }

        let stable_changed = self.stable_blocks.len() != prev_stable_count;

        (stable_changed, tail_changed)
    }

    /// 结束流：强制完成剩余内容
    pub fn finalize(&mut self) {
        if self.is_finishing {
            return;
        }
        self.is_finishing = true;
        let final_new_blocks = self.parser.finalize(&self.full_text);

        self.stable_blocks.extend(final_new_blocks);
        self.tail_content.clear();
        self.tail_block = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_does_not_invent_think_closing_tags() {
        let mut buffer = AuroraBuffer::new();
        buffer.append_chunk("普通文本里提到 <think 不是思考块");
        buffer.finalize();

        assert_eq!(buffer.full_text, "普通文本里提到 <think 不是思考块");
        assert_eq!(buffer.stable_blocks.len(), 1);

        match &buffer.stable_blocks[0] {
            StreamBlock::Markdown { content, .. } => {
                assert_eq!(content, "普通文本里提到 <think 不是思考块");
            }
            other => panic!("expected markdown block, got {:?}", other),
        }
    }

    #[test]
    fn streaming_tail_is_compact_for_small_and_large_markdown() {
        let mut buffer = AuroraBuffer::new();
        buffer.append_chunk("正常一段流式文本，尚未闭合");
        buffer.process_queue();

        match buffer.tail_block.as_ref().expect("tail block") {
            StreamBlock::Markdown { content, nodes, .. } => {
                assert_eq!(content, "正常一段流式文本，尚未闭合");
                assert!(nodes.is_none(), "活动 tail 不应携带 AST");
            }
            other => panic!("expected markdown tail block, got {:?}", other),
        }

        let large = "X".repeat(80_000);
        buffer.append_chunk(&large);
        buffer.process_queue();
        match buffer.tail_block.as_ref().expect("large tail block") {
            StreamBlock::Markdown { content, nodes, .. } => {
                assert!(content.ends_with(&large));
                assert!(nodes.is_none(), "大 tail 同样不应携带 AST");
            }
            other => panic!("expected markdown tail block, got {:?}", other),
        }
    }

    #[test]
    fn streaming_vcp_root_source_is_repaired_without_ast() {
        let mut buffer = AuroraBuffer::new();
        buffer.append_chunk(concat!(
            "<div id=\"vcp-root\"><div data-probe=\"first\">first</div></div>",
            "<div data-probe=\"second\">second</div>"
        ));
        buffer.process_queue();

        let rendered_tail = match buffer.tail_block.as_ref() {
            Some(StreamBlock::Markdown { content, nodes, .. }) => {
                assert!(nodes.is_none());
                content
            }
            other => panic!("expected compact rich HTML tail, got {other:?}"),
        };

        assert!(rendered_tail.contains("data-probe=\"second\""));
        assert!(!rendered_tail.contains("</div></div><div data-probe=\"second\""));
        assert!(rendered_tail.ends_with("</div>"), "{rendered_tail}");
    }

    #[test]
    fn streaming_vcp_root_followed_by_daily_note_becomes_stable() {
        let mut buffer = AuroraBuffer::new();
        buffer.append_chunk(concat!(
            "<div id=\"vcp-root\"><p>visible reply</p></div>",
            "<<<[TOOL_REQUEST]>>>\n",
            "maid:「始」Mama「末」,\n",
            "tool_name:「始」DailyNote「末」,\n",
            "command:「始」create「末」,\n",
            "Date:「始」2026-07-26「末」,\n",
            "Content:「始ESCAPE」stream diary body「末ESCAPE」,\n",
            "archery:「始」no_reply「末」\n",
            "<<<[END_TOOL_REQUEST]>>>"
        ));

        let (stable_changed, tail_changed) = buffer.process_queue();

        assert!(stable_changed);
        assert!(!tail_changed);
        assert!(buffer.tail_content.is_empty());
        assert!(buffer.tail_block.is_none());
        assert!(buffer.stable_blocks.iter().any(|block| matches!(
            block,
            StreamBlock::Diary { maid, date, content, .. }
                if maid == "Mama" && date == "2026-07-26" && content == "stream diary body"
        )));
    }

    #[test]
    fn chunked_tool_request_after_html_never_becomes_markdown_tail() {
        let html = "<div id=\"vcp-root\"><p data-probe=\"story\">visible story</p></div>";
        let request = concat!(
            "<<<[TOOL_REQUEST]>>>\n",
            "maid:「始」Nova「末」,\n",
            "tool_name:「始」ComfyUIGen「末」,\n",
            "mode:「始」anima「末」,\n",
            "prompt:「始」portrait prompt still streaming「末」"
        );
        let mut buffer = AuroraBuffer::new();
        buffer.append_chunk(html);
        buffer.process_queue();

        for character in request.chars() {
            let mut encoded = [0; 4];
            buffer.append_chunk(character.encode_utf8(&mut encoded));
            buffer.process_queue();

            if let Some(StreamBlock::Markdown { content, .. }) = buffer.tail_block.as_ref() {
                assert!(
                    !content.contains("<<<")
                        && !content.contains("maid:「始」Nova「末」")
                        && !content.contains("tool_name:「始」ComfyUIGen「末」"),
                    "protocol text leaked through a markdown tail: {content}"
                );
            }
        }

        assert!(buffer.stable_blocks.iter().any(|block| matches!(
            block,
            StreamBlock::Markdown { content, .. } if content.contains("data-probe=\"story\"")
        )));
        assert!(matches!(
            buffer.tail_block.as_ref(),
            Some(StreamBlock::Tool {
                tool_name,
                content,
                is_complete: false,
                ..
            }) if tool_name == "ComfyUIGen"
                && content.contains("prompt:「始」portrait prompt still streaming「末」")
        ));

        buffer.append_chunk("\n<<<[END_TOOL_REQUEST]>>>");
        buffer.process_queue();
        assert!(buffer.tail_block.is_none());
        assert!(buffer.stable_blocks.iter().any(|block| matches!(
            block,
            StreamBlock::Tool {
                tool_name,
                is_complete: true,
                ..
            } if tool_name == "ComfyUIGen"
        )));
    }

    #[test]
    fn chunked_tool_request_inside_unclosed_html_never_becomes_visible_html() {
        let html = concat!(
            "<div id=\"vcp-root\"><p data-probe=\"story\">visible story</p>",
            "<w2g><catsay>咪咪点评"
        );
        let request = concat!(
            "\n<<<[TOOL_REQUEST]>>>\n",
            "maid:「始」Nova「末」,\n",
            "tool_name:「始」ComfyUIGen「末」,\n",
            "mode:「始」anima「末」,\n",
            "prompt:「始」screenshot prompt still streaming「末」"
        );
        let mut buffer = AuroraBuffer::new();
        buffer.append_chunk(html);
        buffer.process_queue();

        for character in request.chars() {
            let mut encoded = [0; 4];
            buffer.append_chunk(character.encode_utf8(&mut encoded));
            buffer.process_queue();

            for block in buffer.stable_blocks.iter().chain(buffer.tail_block.iter()) {
                if let StreamBlock::Markdown { content, .. }
                | StreamBlock::HtmlPreview { content, .. } = block
                {
                    assert!(
                        !content.contains("<<<[TOOL_REQUEST]>>>")
                            && !content.contains("tool_name:「始」ComfyUIGen「末」"),
                        "protocol text leaked through rendered HTML/Markdown: {content}"
                    );
                }
            }
        }

        assert!(buffer.stable_blocks.iter().any(|block| matches!(
            block,
            StreamBlock::Markdown { content, .. }
                if content.contains("data-probe=\"story\"")
                    && content.ends_with("</catsay></w2g></div>")
        )));
        assert!(matches!(
            buffer.tail_block.as_ref(),
            Some(StreamBlock::Tool {
                tool_name,
                content,
                is_complete: false,
                ..
            }) if tool_name == "ComfyUIGen"
                && content.contains("prompt:「始」screenshot prompt still streaming「末」")
        ));
    }

    #[test]
    fn streaming_chunks_and_ipc_serialization_preserve_css_spaces() {
        let chunks = [
            "<div id=\"vcp-root\" style=\"background:linear-gradient(180deg,#fdf6e9",
            " 0%,#fcebd4 40%,#f9e0c0 100%);padding:20px",
            " 16px 24px;opacity:1\"><p>visible</p></div>",
        ];
        let expected = chunks.concat();
        let mut buffer = AuroraBuffer::new();

        for chunk in chunks {
            buffer.append_chunk(chunk);
            buffer.process_queue();
        }
        assert_eq!(buffer.full_text, expected);

        buffer.finalize();
        let update = AuroraUpdate {
            sequence: 1,
            stable_blocks_delta: Some(buffer.stable_blocks.clone()),
            stable_changed: true,
            tail_block: None,
            tail_changed: true,
            content_delta: None,
            content: Some(buffer.full_text.clone()),
        };
        let serialized = serde_json::to_value(update).expect("serialize Aurora update");

        assert_eq!(serialized["content"].as_str(), Some(expected.as_str()));
        let serialized_blocks = serialized["stableBlocksDelta"].to_string();
        assert!(
            serialized_blocks.contains("#fdf6e9 0%"),
            "{serialized_blocks}"
        );
        assert!(
            serialized_blocks.contains("padding:20px 16px 24px"),
            "{serialized_blocks}"
        );
    }

    #[test]
    fn final_blocks_still_include_render_ast() {
        let mut buffer = AuroraBuffer::new();
        buffer.append_chunk("**最终内容**");
        buffer.process_queue();
        buffer.finalize();

        match buffer.stable_blocks.last().expect("final block") {
            StreamBlock::Markdown { nodes, .. } => {
                assert!(nodes.is_some(), "闭合块仍应携带最终 AST");
            }
            other => panic!("expected final markdown block, got {other:?}"),
        }
    }

    #[test]
    fn aurora_update_serializes_only_compact_stream_fields() {
        let update = AuroraUpdate {
            sequence: 7,
            stable_blocks_delta: None,
            stable_changed: false,
            tail_block: Some(StreamBlock::markdown(
                "hello".to_string(),
                None,
                "hash".to_string(),
            )),
            tail_changed: true,
            content_delta: Some("hello".to_string()),
            content: None,
        };

        let value = serde_json::to_value(update).expect("serialize Aurora update");
        assert_eq!(value["sequence"], 7);
        assert_eq!(value["tailBlock"]["content"], "hello");
        assert!(value["tailBlock"].get("nodes").is_none());
        assert!(value.get("tail").is_none());
        assert!(value.get("tailFrame").is_none());
        assert!(value.get("tailSnapshot").is_none());
    }
}
