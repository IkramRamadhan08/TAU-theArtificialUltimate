use acp_thread::{MentionUri, UserMessageId};
use gpui::SharedString;
use language_model::{LanguageModelImage, LanguageModelRequestMessage, Role};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::Arc;
use util::debug_panic;
use util::markdown::MarkdownCodeBlock;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMessage {
    pub id: UserMessageId,
    pub content: Arc<[UserMessageContent]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserMessageContent {
    Text(String),
    Mention {
        uri: MentionUri,
        content: SharedString,
    },
    Image(LanguageModelImage),
}

impl UserMessage {
    pub fn to_markdown(&self) -> String {
        let mut markdown = String::new();

        for content in &*self.content {
            match content {
                UserMessageContent::Text(text) => {
                    markdown.push_str(text);
                    markdown.push('\n');
                }
                UserMessageContent::Image(_) => {
                    markdown.push_str("<image />\n");
                }
                UserMessageContent::Mention { uri, content } => {
                    if !content.is_empty() {
                        let _ = writeln!(&mut markdown, "{}\n\n{}", uri.as_link(), content);
                    } else {
                        let _ = writeln!(&mut markdown, "{}", uri.as_link());
                    }
                }
            }
        }

        markdown
    }

    pub(crate) fn to_request(&self) -> LanguageModelRequestMessage {
        let mut message = LanguageModelRequestMessage {
            role: Role::User,
            content: Vec::with_capacity(self.content.len()),
            cache: false,
            reasoning_details: None,
        };

        const OPEN_CONTEXT: &str = "<context>\n\
            The following items were attached by the user. \
            They are up-to-date and don't need to be re-read.\n\n";

        const OPEN_FILES_TAG: &str = "<files>";
        const OPEN_DIRECTORIES_TAG: &str = "<directories>";
        const OPEN_SYMBOLS_TAG: &str = "<symbols>";
        const OPEN_SELECTIONS_TAG: &str = "<selections>";
        const OPEN_THREADS_TAG: &str = "<threads>";
        const OPEN_FETCH_TAG: &str = "<fetched_urls>";
        const OPEN_RULES_TAG: &str =
            "<rules>\nThe user has specified the following rules that should be applied:\n";
        const OPEN_DIAGNOSTICS_TAG: &str = "<diagnostics>";
        const OPEN_DIFFS_TAG: &str = "<diffs>";
        const MERGE_CONFLICT_TAG: &str = "<merge_conflicts>";
        const OPEN_SKILLS_TAG: &str =
            "<skills>\nThe user has attached the following agent skills:\n";

        let mut file_context = OPEN_FILES_TAG.to_string();
        let mut directory_context = OPEN_DIRECTORIES_TAG.to_string();
        let mut symbol_context = OPEN_SYMBOLS_TAG.to_string();
        let mut selection_context = OPEN_SELECTIONS_TAG.to_string();
        let mut thread_context = OPEN_THREADS_TAG.to_string();
        let mut fetch_context = OPEN_FETCH_TAG.to_string();
        let mut rules_context = OPEN_RULES_TAG.to_string();
        let mut diagnostics_context = OPEN_DIAGNOSTICS_TAG.to_string();
        let mut diffs_context = OPEN_DIFFS_TAG.to_string();
        let mut merge_conflict_context = MERGE_CONFLICT_TAG.to_string();
        let mut skills_context = OPEN_SKILLS_TAG.to_string();

        for chunk in &*self.content {
            let chunk = match chunk {
                UserMessageContent::Text(text) => {
                    language_model::MessageContent::Text(text.clone())
                }
                UserMessageContent::Image(value) => {
                    language_model::MessageContent::Image(value.clone())
                }
                UserMessageContent::Mention { uri, content } => {
                    match uri {
                        MentionUri::File { abs_path } => {
                            write!(
                                &mut file_context,
                                "\n{}",
                                MarkdownCodeBlock {
                                    tag: &codeblock_tag(abs_path, None),
                                    text: content,
                                }
                            )
                            .ok();
                        }
                        MentionUri::PastedImage { .. } => {
                            debug_panic!("pasted image URI should not be used in mention content")
                        }
                        MentionUri::Directory { .. } => {
                            write!(&mut directory_context, "\n{}\n", content).ok();
                        }
                        MentionUri::Symbol {
                            abs_path: path,
                            line_range,
                            ..
                        } => {
                            write!(
                                &mut symbol_context,
                                "\n{}",
                                MarkdownCodeBlock {
                                    tag: &codeblock_tag(path, Some(line_range)),
                                    text: content
                                }
                            )
                            .ok();
                        }
                        MentionUri::Selection {
                            abs_path: path,
                            line_range,
                            ..
                        } => {
                            write!(
                                &mut selection_context,
                                "\n{}",
                                MarkdownCodeBlock {
                                    tag: &codeblock_tag(
                                        path.as_deref().unwrap_or("Untitled".as_ref()),
                                        Some(line_range)
                                    ),
                                    text: content
                                }
                            )
                            .ok();
                        }
                        MentionUri::Thread { .. } => {
                            write!(&mut thread_context, "\n{}\n", content).ok();
                        }
                        MentionUri::Rule { .. } => {
                            // Deprecated: keeps legacy rule mentions as context.
                            write!(
                                &mut rules_context,
                                "\n{}",
                                MarkdownCodeBlock {
                                    tag: "",
                                    text: content
                                }
                            )
                            .ok();
                        }
                        MentionUri::Fetch { url } => {
                            write!(&mut fetch_context, "\nFetch: {}\n\n{}", url, content).ok();
                        }
                        MentionUri::Diagnostics { .. } => {
                            write!(&mut diagnostics_context, "\n{}\n", content).ok();
                        }
                        MentionUri::TerminalSelection { .. } => {
                            write!(
                                &mut selection_context,
                                "\n{}",
                                MarkdownCodeBlock {
                                    tag: "console",
                                    text: content
                                }
                            )
                            .ok();
                        }
                        MentionUri::GitDiff { base_ref } => {
                            write!(
                                &mut diffs_context,
                                "\nBranch diff against {}:\n{}",
                                base_ref,
                                MarkdownCodeBlock {
                                    tag: "diff",
                                    text: content
                                }
                            )
                            .ok();
                        }
                        MentionUri::MergeConflict { file_path } => {
                            write!(
                                &mut merge_conflict_context,
                                "\nMerge conflict in {}:\n{}",
                                file_path,
                                MarkdownCodeBlock {
                                    tag: "diff",
                                    text: content
                                }
                            )
                            .ok();
                        }
                        MentionUri::Skill { name, source, .. } => {
                            let label = format!("{} ({})", name, source);
                            write!(&mut skills_context, "\nSkill: {}\n{}\n", label, content).ok();
                        }
                    }

                    language_model::MessageContent::Text(uri.as_link().to_string())
                }
            };

            message.content.push(chunk);
        }

        let len_before_context = message.content.len();

        if file_context.len() > OPEN_FILES_TAG.len() {
            file_context.push_str("</files>\n");
            message
                .content
                .push(language_model::MessageContent::Text(file_context));
        }

        if directory_context.len() > OPEN_DIRECTORIES_TAG.len() {
            directory_context.push_str("</directories>\n");
            message
                .content
                .push(language_model::MessageContent::Text(directory_context));
        }

        if symbol_context.len() > OPEN_SYMBOLS_TAG.len() {
            symbol_context.push_str("</symbols>\n");
            message
                .content
                .push(language_model::MessageContent::Text(symbol_context));
        }

        if selection_context.len() > OPEN_SELECTIONS_TAG.len() {
            selection_context.push_str("</selections>\n");
            message
                .content
                .push(language_model::MessageContent::Text(selection_context));
        }

        if diffs_context.len() > OPEN_DIFFS_TAG.len() {
            diffs_context.push_str("</diffs>\n");
            message
                .content
                .push(language_model::MessageContent::Text(diffs_context));
        }

        if thread_context.len() > OPEN_THREADS_TAG.len() {
            thread_context.push_str("</threads>\n");
            message
                .content
                .push(language_model::MessageContent::Text(thread_context));
        }

        if fetch_context.len() > OPEN_FETCH_TAG.len() {
            fetch_context.push_str("</fetched_urls>\n");
            message
                .content
                .push(language_model::MessageContent::Text(fetch_context));
        }

        if rules_context.len() > OPEN_RULES_TAG.len() {
            rules_context.push_str("</user_rules>\n");
            message
                .content
                .push(language_model::MessageContent::Text(rules_context));
        }

        if diagnostics_context.len() > OPEN_DIAGNOSTICS_TAG.len() {
            diagnostics_context.push_str("</diagnostics>\n");
            message
                .content
                .push(language_model::MessageContent::Text(diagnostics_context));
        }

        if skills_context.len() > OPEN_SKILLS_TAG.len() {
            skills_context.push_str("</skills>\n");
            message
                .content
                .push(language_model::MessageContent::Text(skills_context));
        }

        if merge_conflict_context.len() > MERGE_CONFLICT_TAG.len() {
            merge_conflict_context.push_str("</merge_conflicts>\n");
            message
                .content
                .push(language_model::MessageContent::Text(merge_conflict_context));
        }

        if message.content.len() > len_before_context {
            message.content.insert(
                len_before_context,
                language_model::MessageContent::Text(OPEN_CONTEXT.into()),
            );
            message
                .content
                .push(language_model::MessageContent::Text("</context>".into()));
        }

        message
    }
}

fn codeblock_tag(full_path: &Path, line_range: Option<&RangeInclusive<u32>>) -> String {
    let mut result = String::new();

    if let Some(extension) = full_path.extension().and_then(|ext| ext.to_str()) {
        let _ = write!(result, "{} ", extension);
    }

    let _ = write!(result, "{}", full_path.display());

    if let Some(range) = line_range {
        if range.start() == range.end() {
            let _ = write!(result, ":{}", range.start() + 1);
        } else {
            let _ = write!(result, ":{}-{}", range.start() + 1, range.end() + 1);
        }
    }

    result
}

