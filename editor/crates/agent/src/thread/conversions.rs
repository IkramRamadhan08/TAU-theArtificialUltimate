use super::user_message::UserMessageContent;
use acp_thread::MentionUri;
use agent_client_protocol::schema as acp;
use gpui::SharedString;
use language_model::LanguageModelImage;
use util::markdown::MarkdownCodeBlock;
use util::paths::PathStyle;
impl From<&str> for UserMessageContent {
    fn from(text: &str) -> Self {
        Self::Text(text.into())
    }
}

impl From<String> for UserMessageContent {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl UserMessageContent {
    pub fn from_content_block(value: acp::ContentBlock, path_style: PathStyle) -> Self {
        match value {
            acp::ContentBlock::Text(text_content) => Self::Text(text_content.text),
            acp::ContentBlock::Image(image_content) => Self::Image(convert_image(image_content)),
            acp::ContentBlock::Audio(_) => {
                // TODO
                Self::Text("[audio]".to_string())
            }
            acp::ContentBlock::ResourceLink(resource_link) => {
                match MentionUri::parse(&resource_link.uri, path_style) {
                    Ok(uri) => Self::Mention {
                        uri,
                        content: SharedString::default(),
                    },
                    Err(err) => {
                        log::error!("Failed to parse mention link: {}", err);
                        Self::Text(format!("[{}]({})", resource_link.name, resource_link.uri))
                    }
                }
            }
            acp::ContentBlock::Resource(resource) => match resource.resource {
                acp::EmbeddedResourceResource::TextResourceContents(resource) => {
                    match MentionUri::parse(&resource.uri, path_style) {
                        Ok(uri) => Self::Mention {
                            uri,
                            content: resource.text.into(),
                        },
                        Err(err) => {
                            log::error!("Failed to parse mention link: {}", err);
                            Self::Text(
                                MarkdownCodeBlock {
                                    tag: &resource.uri,
                                    text: &resource.text,
                                }
                                .to_string(),
                            )
                        }
                    }
                }
                acp::EmbeddedResourceResource::BlobResourceContents(_) => {
                    // TODO
                    Self::Text("[blob]".to_string())
                }
                other => {
                    log::warn!("Unexpected content type: {:?}", other);
                    Self::Text("[unknown]".to_string())
                }
            },
            other => {
                log::warn!("Unexpected content type: {:?}", other);
                Self::Text("[unknown]".to_string())
            }
        }
    }
}

impl From<UserMessageContent> for acp::ContentBlock {
    fn from(content: UserMessageContent) -> Self {
        match content {
            UserMessageContent::Text(text) => text.into(),
            UserMessageContent::Image(image) => {
                acp::ContentBlock::Image(acp::ImageContent::new(image.source, "image/png"))
            }
            UserMessageContent::Mention { uri, content } => acp::ContentBlock::Resource(
                acp::EmbeddedResource::new(acp::EmbeddedResourceResource::TextResourceContents(
                    acp::TextResourceContents::new(content, uri.to_uri().to_string()),
                )),
            ),
        }
    }
}

pub(crate) fn convert_image(image_content: acp::ImageContent) -> LanguageModelImage {
    LanguageModelImage {
        source: image_content.data.into(),
    }
}
