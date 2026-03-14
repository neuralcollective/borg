use std::collections::HashMap;

use anyhow::Result;

use crate::traits::{DocumentParser, DocumentSection, ParsedDocument};

/// Routes documents to the correct parser based on mime type or extension.
pub struct DocumentParserRouter {
    parsers: Vec<Box<dyn DocumentParser>>,
}

impl DocumentParserRouter {
    pub fn new() -> Self {
        Self {
            parsers: vec![
                Box::new(MarkdownParser),
                Box::new(HtmlParser),
                Box::new(PlainTextParser),
            ],
        }
    }

    pub fn with_parser(mut self, parser: Box<dyn DocumentParser>) -> Self {
        self.parsers.push(parser);
        self
    }

    pub fn parse(&self, data: &[u8], filename: &str, mime_type: &str) -> Result<ParsedDocument> {
        let effective_mime = if mime_type.is_empty() {
            mime_from_extension(filename)
        } else {
            mime_type.to_string()
        };

        for parser in &self.parsers {
            if parser.supported_types().iter().any(|t| t == &effective_mime) {
                return parser.parse(data, filename, &effective_mime);
            }
        }

        // Fallback to plain text
        PlainTextParser.parse(data, filename, &effective_mime)
    }
}

impl Default for DocumentParserRouter {
    fn default() -> Self {
        Self::new()
    }
}

fn mime_from_extension(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "md" | "markdown" => "text/markdown".into(),
        "html" | "htm" => "text/html".into(),
        "txt" | "text" => "text/plain".into(),
        "pdf" => "application/pdf".into(),
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
        "doc" => "application/msword".into(),
        "json" => "application/json".into(),
        "csv" => "text/csv".into(),
        "xml" => "application/xml".into(),
        _ => "text/plain".into(),
    }
}

// ── Markdown Parser ──────────────────────────────────────────────────────

pub struct MarkdownParser;

impl DocumentParser for MarkdownParser {
    fn parse(&self, data: &[u8], filename: &str, _mime_type: &str) -> Result<ParsedDocument> {
        let text = String::from_utf8_lossy(data).to_string();
        let mut sections = Vec::new();
        let mut current_heading = String::new();
        let mut current_content = Vec::new();
        let mut current_level: u8 = 0;

        for line in text.lines() {
            if let Some(heading) = parse_markdown_heading(line) {
                if !current_heading.is_empty() || !current_content.is_empty() {
                    sections.push(DocumentSection {
                        heading: current_heading.clone(),
                        content: current_content.join("\n").trim().to_string(),
                        level: current_level,
                    });
                }
                current_heading = heading.1.to_string();
                current_level = heading.0;
                current_content.clear();
            } else {
                current_content.push(line.to_string());
            }
        }

        if !current_heading.is_empty() || !current_content.is_empty() {
            sections.push(DocumentSection {
                heading: current_heading,
                content: current_content.join("\n").trim().to_string(),
                level: current_level,
            });
        }

        let mut metadata = HashMap::new();
        metadata.insert("filename".into(), filename.to_string());
        metadata.insert("format".into(), "markdown".into());

        Ok(ParsedDocument {
            text,
            metadata,
            sections,
            page_count: None,
        })
    }

    fn supported_types(&self) -> Vec<String> {
        vec!["text/markdown".into()]
    }
}

fn parse_markdown_heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level > 6 || level == 0 {
        return None;
    }
    let rest = trimmed[level..].trim();
    if rest.is_empty() {
        return None;
    }
    Some((level as u8, rest))
}

// ── HTML Parser ──────────────────────────────────────────────────────────

pub struct HtmlParser;

impl DocumentParser for HtmlParser {
    fn parse(&self, data: &[u8], filename: &str, _mime_type: &str) -> Result<ParsedDocument> {
        let html = String::from_utf8_lossy(data).to_string();
        let text = strip_html_tags(&html);

        let mut metadata = HashMap::new();
        metadata.insert("filename".into(), filename.to_string());
        metadata.insert("format".into(), "html".into());

        // Extract title if present
        if let Some(title) = extract_html_tag_content(&html, "title") {
            metadata.insert("title".into(), title);
        }

        Ok(ParsedDocument {
            text,
            metadata,
            sections: Vec::new(),
            page_count: None,
        })
    }

    fn supported_types(&self) -> Vec<String> {
        vec!["text/html".into()]
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;

    let lower = html.to_lowercase();
    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag && chars[i] == '<' {
            in_tag = true;
            let rest: String = lower_chars[i..].iter().take(10).collect();
            if rest.starts_with("<script") {
                in_script = true;
            } else if rest.starts_with("<style") {
                in_style = true;
            } else if rest.starts_with("</script") {
                in_script = false;
            } else if rest.starts_with("</style") {
                in_style = false;
            }
        } else if in_tag && chars[i] == '>' {
            in_tag = false;
        } else if !in_tag && !in_script && !in_style {
            result.push(chars[i]);
        }
        i += 1;
    }

    // Decode common entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
}

fn extract_html_tag_content(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let start_idx = lower.find(&open)?;
    let after_open = lower[start_idx..].find('>')?;
    let content_start = start_idx + after_open + 1;
    let end_idx = lower[content_start..].find(&close)?;
    Some(html[content_start..content_start + end_idx].trim().to_string())
}

// ── Plain Text Parser ────────────────────────────────────────────────────

pub struct PlainTextParser;

impl DocumentParser for PlainTextParser {
    fn parse(&self, data: &[u8], filename: &str, _mime_type: &str) -> Result<ParsedDocument> {
        let text = String::from_utf8_lossy(data).to_string();
        let mut metadata = HashMap::new();
        metadata.insert("filename".into(), filename.to_string());
        metadata.insert("format".into(), "plain".into());

        Ok(ParsedDocument {
            text,
            metadata,
            sections: Vec::new(),
            page_count: None,
        })
    }

    fn supported_types(&self) -> Vec<String> {
        vec![
            "text/plain".into(),
            "application/json".into(),
            "text/csv".into(),
            "application/xml".into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_parser_extracts_sections() {
        let md = b"# Title\nIntro text\n## Section 1\nContent 1\n## Section 2\nContent 2";
        let doc = MarkdownParser.parse(md, "test.md", "text/markdown").unwrap();
        assert_eq!(doc.sections.len(), 3);
        assert_eq!(doc.sections[0].heading, "Title");
        assert_eq!(doc.sections[0].level, 1);
        assert_eq!(doc.sections[1].heading, "Section 1");
        assert_eq!(doc.sections[1].level, 2);
        assert!(doc.sections[1].content.contains("Content 1"));
    }

    #[test]
    fn html_parser_strips_tags() {
        let html = b"<html><head><title>Test</title></head><body><p>Hello <b>world</b></p></body></html>";
        let doc = HtmlParser.parse(html, "test.html", "text/html").unwrap();
        assert!(doc.text.contains("Hello world"));
        assert_eq!(doc.metadata.get("title").unwrap(), "Test");
    }

    #[test]
    fn html_parser_strips_scripts() {
        let html = b"<p>Before</p><script>alert('xss')</script><p>After</p>";
        let doc = HtmlParser.parse(html, "test.html", "text/html").unwrap();
        assert!(doc.text.contains("Before"));
        assert!(doc.text.contains("After"));
        assert!(!doc.text.contains("alert"));
    }

    #[test]
    fn plain_text_parser_preserves_content() {
        let text = b"Just some plain text\nWith multiple lines";
        let doc = PlainTextParser.parse(text, "test.txt", "text/plain").unwrap();
        assert_eq!(doc.text, "Just some plain text\nWith multiple lines");
    }

    #[test]
    fn router_selects_correct_parser() {
        let router = DocumentParserRouter::new();
        let md = b"# Heading\nContent";
        let doc = router.parse(md, "test.md", "").unwrap();
        assert_eq!(doc.metadata.get("format").unwrap(), "markdown");
    }

    #[test]
    fn router_falls_back_to_plain_text() {
        let router = DocumentParserRouter::new();
        let data = b"some data";
        let doc = router.parse(data, "file.xyz", "application/octet-stream").unwrap();
        assert_eq!(doc.metadata.get("format").unwrap(), "plain");
    }

    #[test]
    fn mime_from_extension_works() {
        assert_eq!(mime_from_extension("test.md"), "text/markdown");
        assert_eq!(mime_from_extension("test.html"), "text/html");
        assert_eq!(mime_from_extension("test.txt"), "text/plain");
        assert_eq!(mime_from_extension("test.pdf"), "application/pdf");
    }
}
