use super::*;
use gpui::{App, Context, SharedString, Task, TestAppContext};
use language_model::{
    CompletionIntent, LanguageModel, LanguageModelCompletionEvent,
    LanguageModelRequest, LanguageModelRequestMessage, LanguageModelToolResult,
    LanguageModelToolUseId, Role, TokenUsage,
};
use collections::HashMap;
use serde_json::json;
use std::sync::Arc;
use agent_settings::AutoCompactSettings;

mod tests {
    use super::*;
    use gpui::TestAppContext;
    use language_model::LanguageModelToolUseId;
    use language_model::fake_provider::FakeLanguageModel;
    use serde_json::json;
    use std::sync::Arc;

    async fn setup_thread_for_test(cx: &mut TestAppContext) -> (Entity<Thread>, ThreadEventStream) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
        });

        let fs = fs::FakeFs::new(cx.background_executor.clone());
        let templates = Templates::new();
        let project = Project::test(fs.clone(), [], cx).await;

        cx.update(|cx| {
            let project_context = cx.new(|_cx| prompt_store::ProjectContext::default());
            let context_server_store = project.read(cx).context_server_store();
            let context_server_registry =
                cx.new(|cx| ContextServerRegistry::new(context_server_store, cx));

            let thread = cx.new(|cx| {
                Thread::new(
                    project,
                    project_context,
                    context_server_registry,
                    templates,
                    None,
                    cx,
                )
            });

            let (event_tx, _event_rx) = mpsc::unbounded();
            let event_stream = ThreadEventStream(event_tx);

            (thread, event_stream)
        })
    }

    fn set_auto_compact_settings(cx: &mut App, auto_compact: agent_settings::AutoCompactSettings) {
        let mut settings = AgentSettings::get_global(cx).clone();
        settings.auto_compact = auto_compact;
        AgentSettings::override_global(settings, cx);
    }

    #[test]
    fn test_summary_compaction_renders_for_request_and_markdown() {
        let message = Message::Compaction(CompactionInfo::Summary("Older context".into()));

        assert_eq!(message.role(), Role::User);
        assert_eq!(message.to_markdown(), "--- Context Compacted ---\n");

        let request_messages = message.to_request();
        assert_eq!(request_messages.len(), 1);
        assert_eq!(request_messages[0].role, Role::User);
        assert!(!request_messages[0].cache);
        assert_eq!(request_messages[0].reasoning_details, None);
        assert_eq!(request_messages[0].content.len(), 1);
        let language_model::MessageContent::Text(text) = &request_messages[0].content[0] else {
            panic!("expected text summary context");
        };
        assert_eq!(
            text.as_str(),
            "The previous conversation was compacted. Use this summary as context:\n\nOlder context"
        );
    }

    fn user_text_message(id: UserMessageId, text: &str) -> Arc<Message> {
        Arc::new(Message::User(UserMessage {
            id,
            content: vec![UserMessageContent::Text(text.to_string())].into(),
        }))
    }

    fn agent_text_message(text: &str) -> Arc<Message> {
        Arc::new(Message::Agent(AgentMessage {
            content: vec![AgentMessageContent::Text(text.to_string())],
            ..Default::default()
        }))
    }

    fn summary_compaction(summary: &str) -> Arc<Message> {
        Arc::new(Message::Compaction(CompactionInfo::Summary(summary.into())))
    }

    fn summary_request_text(summary: &str) -> String {
        format!(
            "The previous conversation was compacted. Use this summary as context:\n\n{summary}"
        )
    }

    fn request_texts_after_system(messages: &[LanguageModelRequestMessage]) -> Vec<String> {
        messages
            .iter()
            .skip(1)
            .map(LanguageModelRequestMessage::string_contents)
            .collect()
    }

    #[gpui::test]
    async fn test_compaction_threshold_uses_percentage_setting(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        let user_message_id = UserMessageId::new();

        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model, cx);
                thread
                    .messages
                    .push(user_text_message(user_message_id.clone(), "below limit"));
                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 899_999,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), None);

                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 900_000,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), Some(1));
            });
        });
    }

    #[gpui::test]
    async fn test_compaction_threshold_respects_enabled_setting(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        let user_message_id = UserMessageId::new();

        cx.update(|cx| {
            set_auto_compact_settings(
                cx,
                agent_settings::AutoCompactSettings {
                    enabled: false,
                    threshold: AutoCompactThreshold::Percentage(0.9),
                },
            );
            thread.update(cx, |thread, cx| {
                thread.set_model(model, cx);
                thread
                    .messages
                    .push(user_text_message(user_message_id.clone(), "near limit"));
                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 960_000,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), None);
            });
        });
    }

    #[gpui::test]
    async fn test_compaction_threshold_respects_token_settings(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        let user_message_id = UserMessageId::new();

        cx.update(|cx| {
            set_auto_compact_settings(
                cx,
                agent_settings::AutoCompactSettings {
                    enabled: true,
                    threshold: AutoCompactThreshold::TokensUsed(100_000),
                },
            );
            thread.update(cx, |thread, cx| {
                thread.set_model(model, cx);
                thread.messages.push(user_text_message(
                    user_message_id.clone(),
                    "fixed token limit",
                ));
                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 99_999,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), None);

                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 100_000,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), Some(1));

                set_auto_compact_settings(
                    cx,
                    agent_settings::AutoCompactSettings {
                        enabled: true,
                        threshold: AutoCompactThreshold::TokensRemaining(20_000),
                    },
                );
                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 980_000,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), None);

                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 980_001,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), Some(1));
            });
        });
    }

    #[gpui::test]
    async fn test_compaction_unavailable_for_small_context_window(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        // A context window below the minimum disables auto-compaction.
        model.set_max_token_count(MIN_COMPACTION_CONTEXT_WINDOW - 1);
        let user_message_id = UserMessageId::new();

        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model, cx);
                thread
                    .messages
                    .push(user_text_message(user_message_id.clone(), "near limit"));
                thread.request_token_usage.insert(
                    user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: u64::MAX,
                        ..Default::default()
                    },
                );

                assert_eq!(thread.compaction_message_target_ix(cx), None);
            });
        });
    }

    #[gpui::test]
    async fn test_compaction_inserts_before_new_user_and_requests_compacted_window(
        cx: &mut TestAppContext,
    ) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        let old_user_message_id = UserMessageId::new();
        let new_user_message_id = UserMessageId::new();

        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .messages
                    .push(user_text_message(old_user_message_id.clone(), "old user"));
                thread.messages.push(agent_text_message("old assistant"));
                thread.request_token_usage.insert(
                    old_user_message_id.clone(),
                    language_model::TokenUsage {
                        input_tokens: 960_000,
                        ..Default::default()
                    },
                );
            });
        });

        let _events = cx
            .update(|cx| {
                thread.update(cx, |thread, cx| {
                    thread.send(new_user_message_id, vec!["new prompt"], cx)
                })
            })
            .unwrap();
        cx.run_until_parked();

        let compaction_request = model.pending_completions().pop().unwrap();
        assert_eq!(
            compaction_request.intent,
            Some(CompletionIntent::ThreadContextSummarization)
        );
        let compaction_texts = request_texts_after_system(&compaction_request.messages);
        assert_eq!(compaction_texts.len(), 3);
        assert_eq!(compaction_texts[0], "old user");
        assert_eq!(compaction_texts[1], "old assistant");
        assert_eq!(compaction_texts[2], COMPACTION_PROMPT);

        model.send_completion_stream_text_chunk(&compaction_request, "compacted old context");
        model.end_completion_stream(&compaction_request);
        cx.run_until_parked();

        let final_request = model.pending_completions().pop().unwrap();
        assert_eq!(final_request.intent, Some(CompletionIntent::UserPrompt));
        assert_eq!(
            request_texts_after_system(&final_request.messages),
            vec![
                "old user".to_string(),
                summary_request_text("compacted old context"),
                "new prompt".to_string(),
            ]
        );

        model.send_completion_stream_text_chunk(&final_request, "answer");
        model.end_completion_stream(&final_request);
        cx.run_until_parked();

        cx.update(|cx| {
            thread.read_with(cx, |thread, _cx| {
                assert!(matches!(&*thread.messages[0], Message::User(_)));
                assert!(matches!(&*thread.messages[1], Message::Agent(_)));
                assert!(matches!(
                    &*thread.messages[2],
                    Message::Compaction(CompactionInfo::Summary(summary)) if summary.as_ref() == "compacted old context"
                ));
                assert!(matches!(&*thread.messages[3], Message::User(_)));
            });
        });
    }

    #[gpui::test]
    async fn test_manual_compact_forces_summary(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        // A context window below the minimum and no recorded token usage would
        // both disable *automatic* compaction. Manual compaction forces it anyway.
        model.set_max_token_count(MIN_COMPACTION_CONTEXT_WINDOW - 1);
        let user_message_id = UserMessageId::new();
        let compact_message_id = UserMessageId::new();

        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .messages
                    .push(user_text_message(user_message_id.clone(), "old user"));
                thread.messages.push(agent_text_message("old assistant"));
                // Auto-compaction would be a no-op here.
                assert_eq!(thread.compaction_message_target_ix(cx), None);
            });
        });

        let _events = cx
            .update(|cx| {
                thread.update(cx, |thread, cx| {
                    thread.compact(compact_message_id.clone(), cx)
                })
            })
            .unwrap();
        cx.run_until_parked();

        let compaction_request = model.pending_completions().pop().unwrap();
        assert_eq!(
            compaction_request.intent,
            Some(CompletionIntent::ThreadContextSummarization)
        );
        let compaction_texts = request_texts_after_system(&compaction_request.messages);
        assert_eq!(compaction_texts.len(), 3);
        assert_eq!(compaction_texts[0], "old user");
        assert_eq!(compaction_texts[1], "old assistant");
        assert_eq!(compaction_texts[2], COMPACTION_PROMPT);

        model.send_completion_stream_text_chunk(&compaction_request, "summary of old context");
        model.end_completion_stream(&compaction_request);
        cx.run_until_parked();

        // The compaction summary is appended after a zero-content user message
        // marker, and no follow-up model turn is requested — `/compact` only
        // compacts.
        assert!(model.pending_completions().is_empty());
        cx.update(|cx| {
            thread.read_with(cx, |thread, _cx| {
                assert!(matches!(&*thread.messages[0], Message::User(_)));
                assert!(matches!(&*thread.messages[1], Message::Agent(_)));
                assert!(matches!(
                    &*thread.messages[2],
                    Message::User(UserMessage { id, content }) if id == &compact_message_id && content.is_empty()
                ));
                assert!(matches!(
                    &*thread.messages[3],
                    Message::Compaction(CompactionInfo::Summary(summary)) if summary.as_ref() == "summary of old context"
                ));
                // Re-running `/compact` with nothing new to summarize is a
                // no-op: the thread already ends in a compaction.
                assert_eq!(thread.forced_compaction_target_ix(), None);
            });

            thread
                .update(cx, |thread, cx| thread.truncate(compact_message_id.clone(), cx))
                .unwrap();

            thread.read_with(cx, |thread, _cx| {
                assert_eq!(thread.messages.len(), 2);
                assert!(matches!(&*thread.messages[0], Message::User(_)));
                assert!(matches!(&*thread.messages[1], Message::Agent(_)));
            });
        });
    }

    /// Cancelling an in-flight manual compaction must not leave the zero-content
    /// rewind marker (or a partial summary) dangling at the end of the thread.
    #[gpui::test]
    async fn test_manual_compact_cancelled_leaves_no_marker(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());

        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), "old user"));
                thread.messages.push(agent_text_message("old assistant"));
            });
        });

        let _events = cx
            .update(|cx| thread.update(cx, |thread, cx| thread.compact(UserMessageId::new(), cx)))
            .unwrap();
        cx.run_until_parked();
        // The compaction request is in flight but hasn't streamed a summary.
        assert_eq!(model.pending_completions().len(), 1);

        cx.update(|cx| thread.update(cx, |thread, cx| thread.cancel(cx)))
            .await;
        cx.run_until_parked();

        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.messages.len(), 2);
            assert!(matches!(&*thread.messages[0], Message::User(_)));
            assert!(matches!(&*thread.messages[1], Message::Agent(_)));
        });
    }

    /// A failed compaction (here, an empty summary) reports an error and leaves
    /// the thread untouched — no marker, no compaction.
    #[gpui::test]
    async fn test_manual_compact_empty_summary_leaves_no_marker(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());

        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), "old user"));
                thread.messages.push(agent_text_message("old assistant"));
            });
        });

        let mut events = cx
            .update(|cx| thread.update(cx, |thread, cx| thread.compact(UserMessageId::new(), cx)))
            .unwrap();
        cx.run_until_parked();

        let request = model.pending_completions().pop().unwrap();
        // End the stream without emitting any summary text.
        model.end_completion_stream(&request);
        cx.run_until_parked();

        // An error is surfaced, and the thread is left exactly as it was. The
        // compaction task drops the event stream after failing, so the channel
        // closes and this drain terminates.
        let mut saw_error = false;
        while let Some(event) = events.next().await {
            if event.is_err() {
                saw_error = true;
            }
        }
        assert!(saw_error, "expected an error event for the empty summary");
        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.messages.len(), 2);
            assert!(matches!(&*thread.messages[0], Message::User(_)));
            assert!(matches!(&*thread.messages[1], Message::Agent(_)));
        });
    }

    /// `/compact` on an empty thread (nothing to summarize) is a no-op: it
    /// issues no model request and adds no marker.
    #[gpui::test]
    async fn test_manual_compact_noop_on_empty_thread(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        cx.update(|cx| thread.update(cx, |thread, cx| thread.set_model(model.clone(), cx)));

        let _events = cx
            .update(|cx| thread.update(cx, |thread, cx| thread.compact(UserMessageId::new(), cx)))
            .unwrap();
        cx.run_until_parked();

        assert!(model.pending_completions().is_empty());
        thread.read_with(cx, |thread, _cx| {
            assert!(thread.messages.is_empty());
        });
    }

    /// The zero-content marker replays as an empty user message, which the UI
    /// drops (it renders content blocks, of which there are none), so reloading
    /// a compacted thread doesn't surface an empty `/compact` bubble.
    #[gpui::test]
    async fn test_manual_compact_marker_replays_as_empty_user_message(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let marker_id = UserMessageId::new();

        let mut replay_events = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), "before"));
                thread.messages.push(agent_text_message("answer"));
                thread.messages.push(Arc::new(Message::User(UserMessage {
                    id: marker_id.clone(),
                    content: Arc::from([]),
                })));
                thread.messages.push(summary_compaction("summary"));
                thread.replay(cx)
            })
        });

        // Skip the leading "before"/"answer" replay events.
        let _ = replay_events.next().await;
        let _ = replay_events.next().await;

        let event = replay_events.next().await;
        match event {
            Some(Ok(ThreadEvent::UserMessage(message))) => {
                assert_eq!(message.id, marker_id);
                assert!(
                    message.content.is_empty(),
                    "marker should replay with no content so the UI renders nothing"
                );
            }
            _ => panic!("expected the marker to replay as a user message, got {event:?}"),
        }

        let event = replay_events.next().await;
        assert!(
            matches!(&event, Some(Ok(ThreadEvent::ContextCompaction(_)))),
            "expected the compaction to replay after the marker, got {event:?}"
        );
    }

    #[gpui::test]
    async fn test_compaction_usage_counts_toward_cumulative_usage(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let model = Arc::new(FakeLanguageModel::default());
        let old_user_message_id = UserMessageId::new();
        let new_user_message_id = UserMessageId::new();
        let prior_usage = TokenUsage {
            input_tokens: 960_000,
            output_tokens: 25,
            ..Default::default()
        };
        let compaction_usage = TokenUsage {
            input_tokens: 40,
            output_tokens: 9,
            cache_creation_input_tokens: 2,
            cache_read_input_tokens: 3,
        };
        let final_usage = TokenUsage {
            input_tokens: 500,
            output_tokens: 50,
            ..Default::default()
        };

        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .messages
                    .push(user_text_message(old_user_message_id.clone(), "old user"));
                thread.messages.push(agent_text_message("old assistant"));
                thread
                    .request_token_usage
                    .insert(old_user_message_id.clone(), prior_usage);
                thread.cumulative_token_usage = prior_usage;
                thread.current_request_token_usage = prior_usage;
            });
        });

        let _events = cx
            .update(|cx| {
                thread.update(cx, |thread, cx| {
                    thread.send(new_user_message_id.clone(), vec!["new prompt"], cx)
                })
            })
            .unwrap();
        cx.run_until_parked();

        let compaction_request = model.pending_completions().pop().unwrap();
        assert_eq!(
            compaction_request.intent,
            Some(CompletionIntent::ThreadContextSummarization)
        );

        model.send_completion_stream_event(
            &compaction_request,
            LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                input_tokens: 40,
                output_tokens: 4,
                ..Default::default()
            }),
        );
        model.send_completion_stream_event(
            &compaction_request,
            LanguageModelCompletionEvent::UsageUpdate(compaction_usage),
        );
        model.send_completion_stream_text_chunk(&compaction_request, "compacted old context");
        model.end_completion_stream(&compaction_request);
        cx.run_until_parked();

        let expected_after_compaction = prior_usage + compaction_usage;
        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.cumulative_token_usage(), expected_after_compaction);
            assert!(
                !thread
                    .request_token_usage
                    .contains_key(&new_user_message_id)
            );
        });

        let final_request = model.pending_completions().pop().unwrap();
        assert_eq!(final_request.intent, Some(CompletionIntent::UserPrompt));

        model.send_completion_stream_event(
            &final_request,
            LanguageModelCompletionEvent::UsageUpdate(final_usage),
        );
        model.end_completion_stream(&final_request);
        cx.run_until_parked();

        thread.read_with(cx, |thread, _cx| {
            assert_eq!(
                thread.cumulative_token_usage(),
                expected_after_compaction + final_usage
            );
            assert_eq!(
                thread.request_token_usage.get(&new_user_message_id),
                Some(&final_usage)
            );
        });
    }

    #[gpui::test]
    async fn test_replay_emits_context_compaction(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let user_message_id = UserMessageId::new();

        let mut replay_events = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread
                    .messages
                    .push(user_text_message(user_message_id.clone(), "before"));
                thread.messages.push(summary_compaction("summary"));
                thread.messages.push(agent_text_message("after"));

                thread.replay(cx)
            })
        });

        let event = replay_events.next().await;
        assert!(
            matches!(
                &event,
                Some(Ok(ThreadEvent::UserMessage(UserMessage { id, .. }))) if id == &user_message_id
            ),
            "expected replayed user message, got {event:?}"
        );

        let event = replay_events.next().await;
        let compaction_id = match &event {
            Some(Ok(ThreadEvent::ContextCompaction(compaction))) => compaction.id.clone(),
            _ => panic!("expected context compaction event, got {event:?}"),
        };

        let event = replay_events.next().await;
        assert!(
            matches!(
                &event,
                Some(Ok(ThreadEvent::ContextCompactionUpdate(update)))
                    if update.id == compaction_id && update.summary_delta == "summary"
            ),
            "expected context compaction summary event, got {event:?}"
        );

        let event = replay_events.next().await;
        assert!(
            matches!(&event, Some(Ok(ThreadEvent::AgentText(text))) if text == "after"),
            "expected replayed agent text, got {event:?}"
        );
    }

    #[gpui::test]
    async fn test_native_compaction_boundary(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;

        let request_messages = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), "before native"));
                thread.messages.push(Arc::new(Message::Compaction(
                    CompactionInfo::ProviderNative {
                        provider: LanguageModelProviderId::from("openai".to_string()),
                        items: vec![json!({"type": "compaction"})],
                    },
                )));
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), "after native"));

                thread.build_request_messages(Vec::new(), cx)
            })
        });

        assert_eq!(
            request_texts_after_system(&request_messages),
            vec!["after native".to_string()]
        );
    }

    #[gpui::test]
    async fn test_retained_users_truncate_oldest(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;
        let mut long_text = "START".to_string();
        long_text.push_str(&"x".repeat(COMPACTION_RETAINED_USER_MESSAGES_BYTE_BUDGET));
        long_text.push_str("END");

        let request_messages = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.messages.push(user_text_message(
                    UserMessageId::new(),
                    "dropped older user",
                ));
                thread
                    .messages
                    .push(agent_text_message("dropped assistant"));
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), &long_text));
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), "new"));
                thread.messages.push(summary_compaction("summary context"));
                thread.messages.push(agent_text_message("after assistant"));
                thread
                    .messages
                    .push(user_text_message(UserMessageId::new(), "after user"));

                thread.build_request_messages(Vec::new(), cx)
            })
        });

        let request_texts = request_texts_after_system(&request_messages);
        assert_eq!(request_texts.len(), 5);
        assert_eq!(
            request_texts[0],
            format!(
                "START{}",
                "x".repeat(
                    COMPACTION_RETAINED_USER_MESSAGES_BYTE_BUDGET - "START".len() - "new".len()
                )
            )
        );
        assert_eq!(request_texts[1], "new");
        assert_eq!(request_texts[2], summary_request_text("summary context"));
        assert_eq!(request_texts[3], "after assistant");
        assert_eq!(request_texts[4], "after user");
        assert!(request_texts.iter().all(
            |text| !text.contains("dropped older user") && !text.contains("dropped assistant")
        ));
    }

    #[test]
    fn test_truncate_text_utf8_boundary() {
        let message = LanguageModelRequestMessage {
            role: Role::User,
            content: vec![MessageContent::Text("hello 👋 world".to_string())],
            cache: false,
            reasoning_details: None,
        };

        let truncated = truncate_user_message_to_byte_budget(message, 8).unwrap();
        assert_eq!(
            truncated.content,
            vec![MessageContent::Text("hello ".to_string())]
        );
    }

    #[test]
    fn test_truncate_keeps_fitting_images() {
        let image = LanguageModelImage {
            source: "image".into(),
        };
        let message = LanguageModelRequestMessage {
            role: Role::User,
            content: vec![
                MessageContent::Text("abc".to_string()),
                MessageContent::Image(image.clone()),
            ],
            cache: false,
            reasoning_details: None,
        };

        let truncated = truncate_user_message_to_byte_budget(message, 8).unwrap();
        assert_eq!(
            truncated.content,
            vec![
                MessageContent::Text("abc".to_string()),
                MessageContent::Image(image),
            ]
        );
    }

    fn setup_parent_with_subagents(
        cx: &mut TestAppContext,
        parent: &Entity<Thread>,
        count: usize,
    ) -> Vec<Entity<Thread>> {
        cx.update(|cx| {
            let mut subagents = Vec::new();
            for _ in 0..count {
                let subagent = cx.new(|cx| Thread::new_subagent(parent, cx));
                parent.update(cx, |thread, _cx| {
                    thread.register_running_subagent(subagent.downgrade());
                });
                subagents.push(subagent);
            }
            subagents
        })
    }

    struct ReplayImageTool;

    impl AgentTool for ReplayImageTool {
        type Input = ();
        type Output = String;

        const NAME: &'static str = "registered_image_tool";

        fn kind() -> acp::ToolKind {
            acp::ToolKind::Other
        }

        fn initial_title(
            &self,
            _input: Result<Self::Input, serde_json::Value>,
            _cx: &mut App,
        ) -> SharedString {
            "Registered Image Tool".into()
        }

        fn run(
            self: Arc<Self>,
            _input: ToolInput<Self::Input>,
            _event_stream: ToolCallEventStream,
            _cx: &mut App,
        ) -> Task<Result<Self::Output, Self::Output>> {
            Task::ready(Ok(String::new()))
        }
    }

    #[gpui::test]
    async fn test_authorize_sandbox_allow_always_records_current_grant(cx: &mut TestAppContext) {
        crate::tests::init_test(cx);

        let (event_stream, mut receiver) = ToolCallEventStream::test();
        let request = SandboxRequest {
            network: false,
            allow_fs_write_all: false,
            unsandboxed: false,
            write_paths: vec![
                PathBuf::from("/tmp/build"),
                PathBuf::from("/tmp/cache"),
                PathBuf::from("/tmp/logs"),
                PathBuf::from("/tmp/secret"),
            ],
        };

        let authorize = cx.update(|cx| {
            event_stream.authorize_sandbox("Allow write access?", None, request.clone(), cx)
        });
        let authorization = receiver.expect_authorization().await;
        let details =
            acp_thread::sandbox_authorization_details_from_meta(&authorization.tool_call.meta)
                .expect("sandbox authorization should include request details");
        assert_eq!(details.command, None);
        assert_eq!(details.network, request.network);
        assert_eq!(details.allow_fs_write_all, request.allow_fs_write_all);
        assert_eq!(details.unsandboxed, request.unsandboxed);
        assert_eq!(details.write_paths, request.write_paths);
        assert!(authorization.tool_call.fields.content.is_none());

        let acp_thread::PermissionOptions::Flat(options) = &authorization.options else {
            panic!("expected flat sandbox permission options");
        };
        let options = options
            .iter()
            .map(|option| {
                (
                    option.option_id.0.as_ref(),
                    option.name.as_ref(),
                    option.kind,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            options,
            vec![
                ("allow", "Allow once", acp::PermissionOptionKind::AllowOnce),
                (
                    "allow_thread",
                    "Allow for this thread",
                    acp::PermissionOptionKind::AllowAlways,
                ),
                (
                    "allow_always",
                    "Allow always",
                    acp::PermissionOptionKind::AllowAlways,
                ),
                ("deny", "Deny", acp::PermissionOptionKind::RejectOnce),
            ]
        );

        let send_result = authorization
            .response
            .send(acp_thread::SelectedPermissionOutcome::new(
                acp::PermissionOptionId::new("allow_always"),
                acp::PermissionOptionKind::AllowAlways,
            ));
        assert!(send_result.is_ok());
        authorize.await.unwrap();

        let effective = event_stream.effective_sandbox_request(
            &SandboxRequest::default(),
            &agent_settings::SandboxPermissions::default(),
        );
        assert_eq!(
            effective.write_paths,
            vec![
                PathBuf::from("/tmp/build"),
                PathBuf::from("/tmp/cache"),
                PathBuf::from("/tmp/logs"),
                PathBuf::from("/tmp/secret"),
            ]
        );
    }

    #[test]
    fn test_auto_resolve_permission_outcome_uses_once_only_options() {
        let options = acp_thread::PermissionOptions::Dropdown(vec![
            acp_thread::PermissionOptionChoice {
                allow: acp::PermissionOption::new(
                    acp::PermissionOptionId::new("always_allow:test_tool"),
                    "Always allow",
                    acp::PermissionOptionKind::AllowAlways,
                ),
                deny: acp::PermissionOption::new(
                    acp::PermissionOptionId::new("always_deny:test_tool"),
                    "Always deny",
                    acp::PermissionOptionKind::RejectAlways,
                ),
                sub_patterns: vec![],
            },
            acp_thread::PermissionOptionChoice {
                allow: acp::PermissionOption::new(
                    acp::PermissionOptionId::new("allow"),
                    "Allow once",
                    acp::PermissionOptionKind::AllowOnce,
                ),
                deny: acp::PermissionOption::new(
                    acp::PermissionOptionId::new("deny"),
                    "Deny once",
                    acp::PermissionOptionKind::RejectOnce,
                ),
                sub_patterns: vec![],
            },
        ]);

        let allow = auto_resolve_permission_outcome(&options, true)
            .expect("allow auto-resolve should use once-only option");
        assert_eq!(allow.option_id, acp::PermissionOptionId::new("allow"));
        assert_eq!(allow.option_kind, acp::PermissionOptionKind::AllowOnce);

        let deny = auto_resolve_permission_outcome(&options, false)
            .expect("deny auto-resolve should use once-only option");
        assert_eq!(deny.option_id, acp::PermissionOptionId::new("deny"));
        assert_eq!(deny.option_kind, acp::PermissionOptionKind::RejectOnce);
    }

    #[gpui::test]
    async fn test_replay_tool_call_replays_image_content(cx: &mut TestAppContext) {
        let (thread, _event_stream) = setup_thread_for_test(cx).await;

        let registered_tool_use_id = LanguageModelToolUseId::from("registered_tool_id");
        let missing_tool_use_id = LanguageModelToolUseId::from("missing_tool_id");
        let image_data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let image = LanguageModelImage {
            source: image_data.into(),
        };

        let mut replay_events = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.add_tool(ReplayImageTool);

                let registered_tool_use = LanguageModelToolUse {
                    id: registered_tool_use_id.clone(),
                    name: ReplayImageTool::NAME.into(),
                    raw_input: "null".to_string(),
                    input: json!(null),
                    is_input_complete: true,
                    thought_signature: None,
                };
                let missing_tool_use = LanguageModelToolUse {
                    id: missing_tool_use_id.clone(),
                    name: "missing_image_tool".into(),
                    raw_input: "{}".to_string(),
                    input: json!({}),
                    is_input_complete: true,
                    thought_signature: None,
                };

                let mut tool_results = IndexMap::default();
                tool_results.insert(
                    registered_tool_use_id.clone(),
                    LanguageModelToolResult {
                        tool_use_id: registered_tool_use_id.clone(),
                        tool_name: ReplayImageTool::NAME.into(),
                        is_error: false,
                        content: vec![
                            LanguageModelToolResultContent::Text("before".into()),
                            LanguageModelToolResultContent::Image(image.clone()),
                            LanguageModelToolResultContent::Text("after".into()),
                        ],
                        output: Some(json!("raw output")),
                    },
                );
                tool_results.insert(
                    missing_tool_use_id.clone(),
                    LanguageModelToolResult {
                        tool_use_id: missing_tool_use_id.clone(),
                        tool_name: "missing_image_tool".into(),
                        is_error: false,
                        content: vec![LanguageModelToolResultContent::Image(image.clone())],
                        output: Some(json!("raw output")),
                    },
                );

                thread.messages.push(Arc::new(Message::Agent(AgentMessage {
                    content: vec![
                        AgentMessageContent::ToolUse(registered_tool_use),
                        AgentMessageContent::ToolUse(missing_tool_use),
                    ],
                    tool_results,
                    reasoning_details: None,
                })));

                thread.replay(cx)
            })
        });

        let mut tool_use_ids_with_image_content = HashSet::default();
        while let Some(event) = replay_events.next().await {
            let event = event.unwrap();
            if let ThreadEvent::ToolCallUpdate(acp_thread::ToolCallUpdate::UpdateFields(update)) =
                event
                && let Some(content) = &update.fields.content
                && content.iter().any(|content| {
                    matches!(
                        content,
                        acp::ToolCallContent::Content(acp::Content {
                            content: acp::ContentBlock::Image(_),
                            ..
                        })
                    )
                })
            {
                tool_use_ids_with_image_content.insert(update.tool_call_id.to_string());
            }
        }

        assert!(tool_use_ids_with_image_content.contains(&registered_tool_use_id.to_string()));
        assert!(tool_use_ids_with_image_content.contains(&missing_tool_use_id.to_string()));
    }

    #[gpui::test]
    async fn test_set_model_propagates_to_subagents(cx: &mut TestAppContext) {
        let (parent, _event_stream) = setup_thread_for_test(cx).await;
        let subagents = setup_parent_with_subagents(cx, &parent, 2);

        let new_model: Arc<dyn LanguageModel> = Arc::new(FakeLanguageModel::with_id_and_thinking(
            "test-provider",
            "new-model",
            "New Model",
            false,
        ));

        cx.update(|cx| {
            parent.update(cx, |thread, cx| {
                thread.set_model(new_model, cx);
            });

            for subagent in &subagents {
                let subagent_model_id = subagent.read(cx).model().unwrap().id();
                assert_eq!(
                    subagent_model_id.0.as_ref(),
                    "new-model",
                    "Subagent model should match parent model after set_model"
                );
            }
        });
    }

    #[gpui::test]
    async fn test_set_summarization_model_propagates_to_subagents(cx: &mut TestAppContext) {
        let (parent, _event_stream) = setup_thread_for_test(cx).await;
        let subagents = setup_parent_with_subagents(cx, &parent, 2);

        let summary_model: Arc<dyn LanguageModel> =
            Arc::new(FakeLanguageModel::with_id_and_thinking(
                "test-provider",
                "summary-model",
                "Summary Model",
                false,
            ));

        cx.update(|cx| {
            parent.update(cx, |thread, cx| {
                thread.set_summarization_model(Some(summary_model), cx);
            });

            for subagent in &subagents {
                let subagent_summary_id = subagent.read(cx).summarization_model().unwrap().id();
                assert_eq!(
                    subagent_summary_id.0.as_ref(),
                    "summary-model",
                    "Subagent summarization model should match parent after set_summarization_model"
                );
            }
        });
    }

    #[gpui::test]
    async fn test_set_thinking_enabled_propagates_to_subagents(cx: &mut TestAppContext) {
        let (parent, _event_stream) = setup_thread_for_test(cx).await;
        let subagents = setup_parent_with_subagents(cx, &parent, 2);

        cx.update(|cx| {
            parent.update(cx, |thread, cx| {
                thread.set_thinking_enabled(true, cx);
            });

            for subagent in &subagents {
                assert!(
                    subagent.read(cx).thinking_enabled(),
                    "Subagent thinking should be enabled after parent enables it"
                );
            }

            parent.update(cx, |thread, cx| {
                thread.set_thinking_enabled(false, cx);
            });

            for subagent in &subagents {
                assert!(
                    !subagent.read(cx).thinking_enabled(),
                    "Subagent thinking should be disabled after parent disables it"
                );
            }
        });
    }

    #[gpui::test]
    async fn test_set_thinking_effort_propagates_to_subagents(cx: &mut TestAppContext) {
        let (parent, _event_stream) = setup_thread_for_test(cx).await;
        let subagents = setup_parent_with_subagents(cx, &parent, 2);

        cx.update(|cx| {
            parent.update(cx, |thread, cx| {
                thread.set_thinking_effort(Some("high".to_string()), cx);
            });

            for subagent in &subagents {
                assert_eq!(
                    subagent.read(cx).thinking_effort().map(|s| s.as_str()),
                    Some("high"),
                    "Subagent thinking effort should match parent"
                );
            }

            parent.update(cx, |thread, cx| {
                thread.set_thinking_effort(None, cx);
            });

            for subagent in &subagents {
                assert_eq!(
                    subagent.read(cx).thinking_effort(),
                    None,
                    "Subagent thinking effort should be None after parent clears it"
                );
            }
        });
    }

    #[gpui::test]
    async fn test_subagent_inherits_settings_at_creation(cx: &mut TestAppContext) {
        let (parent, _event_stream) = setup_thread_for_test(cx).await;

        cx.update(|cx| {
            parent.update(cx, |thread, cx| {
                thread.set_speed(Speed::Fast, cx);
                thread.set_thinking_enabled(true, cx);
                thread.set_thinking_effort(Some("high".to_string()), cx);
                thread.set_profile(AgentProfileId("custom-profile".into()), cx);
            });
        });

        let subagents = setup_parent_with_subagents(cx, &parent, 1);

        cx.update(|cx| {
            let sub = subagents[0].read(cx);
            assert_eq!(sub.speed(), Some(Speed::Fast));
            assert!(sub.thinking_enabled());
            assert_eq!(sub.thinking_effort().map(|s| s.as_str()), Some("high"));
            assert_eq!(sub.profile(), &AgentProfileId("custom-profile".into()));
        });
    }

    #[gpui::test]
    async fn test_set_speed_propagates_to_subagents(cx: &mut TestAppContext) {
        let (parent, _event_stream) = setup_thread_for_test(cx).await;
        let subagents = setup_parent_with_subagents(cx, &parent, 2);

        cx.update(|cx| {
            parent.update(cx, |thread, cx| {
                thread.set_speed(Speed::Fast, cx);
            });

            for subagent in &subagents {
                assert_eq!(
                    subagent.read(cx).speed(),
                    Some(Speed::Fast),
                    "Subagent speed should match parent after set_speed"
                );
            }
        });
    }

    #[gpui::test]
    async fn test_dropped_subagent_does_not_panic(cx: &mut TestAppContext) {
        let (parent, _event_stream) = setup_thread_for_test(cx).await;
        let subagents = setup_parent_with_subagents(cx, &parent, 1);

        // Drop the subagent so the WeakEntity can no longer be upgraded
        drop(subagents);

        // Should not panic even though the subagent was dropped
        cx.update(|cx| {
            parent.update(cx, |thread, cx| {
                thread.set_thinking_enabled(true, cx);
                thread.set_speed(Speed::Fast, cx);
                thread.set_thinking_effort(Some("high".to_string()), cx);
            });
        });
    }

    #[gpui::test]
    async fn test_handle_tool_use_json_parse_error_adds_tool_use_to_content(
        cx: &mut TestAppContext,
    ) {
        let (thread, event_stream) = setup_thread_for_test(cx).await;

        let tool_use_id = LanguageModelToolUseId::from("test_tool_id");
        let tool_name: Arc<str> = Arc::from("test_tool");
        let raw_input: Arc<str> = Arc::from("{invalid json");
        let json_parse_error = "expected value at line 1 column 1".to_string();

        let (_cancellation_tx, cancellation_rx) = watch::channel(false);

        let result = cx
            .update(|cx| {
                thread.update(cx, |thread, cx| {
                    // Call the function under test
                    thread
                        .handle_tool_use_json_parse_error_event(
                            tool_use_id.clone(),
                            tool_name.clone(),
                            raw_input.clone(),
                            json_parse_error,
                            &event_stream,
                            cancellation_rx,
                            cx,
                        )
                        .unwrap()
                })
            })
            .await;

        // Verify the result is an error
        assert!(result.is_error);
        assert_eq!(result.tool_use_id, tool_use_id);
        assert_eq!(result.tool_name, tool_name);
        assert!(matches!(
            result.content.as_slice(),
            [LanguageModelToolResultContent::Text(_)]
        ));

        thread.update(cx, |thread, _cx| {
            // Verify the tool use was added to the message content
            {
                let last_message = thread.pending_message();
                assert_eq!(
                    last_message.content.len(),
                    1,
                    "Should have one tool_use in content"
                );

                match &last_message.content[0] {
                    AgentMessageContent::ToolUse(tool_use) => {
                        assert_eq!(tool_use.id, tool_use_id);
                        assert_eq!(tool_use.name, tool_name);
                        assert_eq!(tool_use.raw_input, raw_input.to_string());
                        assert!(tool_use.is_input_complete);
                        // Should fall back to empty object for invalid JSON
                        assert_eq!(tool_use.input, json!({}));
                    }
                    _ => panic!("Expected ToolUse content"),
                }
            }

            // Insert the tool result (simulating what the caller does)
            thread
                .pending_message()
                .tool_results
                .insert(result.tool_use_id.clone(), result);

            // Verify the tool result was added
            let last_message = thread.pending_message();
            assert_eq!(
                last_message.tool_results.len(),
                1,
                "Should have one tool_result"
            );
            assert!(last_message.tool_results.contains_key(&tool_use_id));
        })
    }
}
