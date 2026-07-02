use super::event_stream::{ToolCallEventStream};
use super::ToolInput;
use super::thread_types::ToolError;
use agent_client_protocol::schema as acp;
use anyhow::{Result, anyhow};
use gpui::{App, SharedString, Task};
use language_model::{
    LanguageModelProviderId, LanguageModelToolResultContent, LanguageModelToolSchemaFormat,
};
use schemars::{JsonSchema, Schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;
use futures::StreamExt;
use futures::channel::mpsc;
impl<T: DeserializeOwned> ToolInput<T> {
    pub fn resolved(input: impl Serialize) -> Self {
        let value = serde_json::to_value(input).expect("failed to serialize tool input");
        Self::ready(value)
    }

    pub fn ready(value: serde_json::Value) -> Self {
        let (tx, rx) = mpsc::unbounded();
        tx.unbounded_send(ToolInputPayload::Full(value)).ok();
        Self {
            rx,
            _phantom: PhantomData,
        }
    }

    pub fn invalid_json(error_message: String) -> Self {
        let (tx, rx) = mpsc::unbounded();
        tx.unbounded_send(ToolInputPayload::InvalidJson { error_message })
            .ok();
        Self {
            rx,
            _phantom: PhantomData,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test() -> (ToolInputSender, Self) {
        let (sender, input) = ToolInputSender::channel();
        (sender, input.cast())
    }

    /// Wait for the final deserialized input, ignoring all partial updates.
    /// Non-streaming tools can use this to wait until the whole input is available.
    pub async fn recv(mut self) -> Result<T> {
        while let Ok(value) = self.next().await {
            match value {
                ToolInputPayload::Full(value) => return Ok(value),
                ToolInputPayload::Partial(_) => {}
                ToolInputPayload::InvalidJson { error_message } => {
                    return Err(anyhow!(error_message));
                }
            }
        }
        Err(anyhow!("tool input was not fully received"))
    }

    pub async fn next(&mut self) -> Result<ToolInputPayload<T>> {
        let value = self
            .rx
            .next()
            .await
            .ok_or_else(|| anyhow!("tool input was not fully received"))?;

        Ok(match value {
            ToolInputPayload::Partial(payload) => ToolInputPayload::Partial(payload),
            ToolInputPayload::Full(payload) => {
                ToolInputPayload::Full(serde_json::from_value(payload)?)
            }
            ToolInputPayload::InvalidJson { error_message } => {
                ToolInputPayload::InvalidJson { error_message }
            }
        })
    }

    fn cast<U: DeserializeOwned>(self) -> ToolInput<U> {
        ToolInput {
            rx: self.rx,
            _phantom: PhantomData,
        }
    }
}

pub enum ToolInputPayload<T> {
    Partial(serde_json::Value),
    Full(T),
    InvalidJson { error_message: String },
}

pub struct ToolInputSender {
    has_received_final: bool,
    tx: mpsc::UnboundedSender<ToolInputPayload<serde_json::Value>>,
}

impl ToolInputSender {
    pub(crate) fn channel() -> (Self, ToolInput<serde_json::Value>) {
        let (tx, rx) = mpsc::unbounded();
        let sender = Self {
            tx,
            has_received_final: false,
        };
        let input = ToolInput {
            rx,
            _phantom: PhantomData,
        };
        (sender, input)
    }

    pub(crate) fn has_received_final(&self) -> bool {
        self.has_received_final
    }

    pub fn send_partial(&mut self, payload: serde_json::Value) {
        self.tx
            .unbounded_send(ToolInputPayload::Partial(payload))
            .ok();
    }

    pub fn send_full(&mut self, payload: serde_json::Value) {
        self.has_received_final = true;
        self.tx.unbounded_send(ToolInputPayload::Full(payload)).ok();
    }

    pub fn send_invalid_json(&mut self, error_message: String) {
        self.has_received_final = true;
        self.tx
            .unbounded_send(ToolInputPayload::InvalidJson { error_message })
            .ok();
    }
}

pub trait AgentTool
where
    Self: 'static + Sized,
{
    type Input: for<'de> Deserialize<'de> + Serialize + JsonSchema;
    type Output: for<'de> Deserialize<'de> + Serialize + Into<LanguageModelToolResultContent>;

    const NAME: &'static str;

    fn description() -> SharedString {
        let schema = schemars::schema_for!(Self::Input);
        SharedString::new(
            schema
                .get("description")
                .and_then(|description| description.as_str())
                .unwrap_or_default(),
        )
    }

    fn kind() -> acp::ToolKind;

    /// The initial tool title to display. Can be updated during the tool run.
    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        cx: &mut App,
    ) -> SharedString;

    /// Returns the JSON schema that describes the tool's input.
    fn input_schema(format: LanguageModelToolSchemaFormat) -> Schema {
        language_model::tool_schema::root_schema_for::<Self::Input>(format)
    }

    /// Returns whether the tool supports streaming of tool use parameters.
    fn supports_input_streaming() -> bool {
        false
    }

    /// Some tools rely on a provider for the underlying billing or other reasons.
    /// Allow the tool to check if they are compatible, or should be filtered out.
    fn supports_provider(_provider: &LanguageModelProviderId) -> bool {
        true
    }

    /// Runs the tool with the provided input.
    ///
    /// Returns `Result<Self::Output, Self::Output>` rather than `Result<Self::Output, anyhow::Error>`
    /// because tool errors are sent back to the model as tool results. This means error output must
    /// be structured and readable by the agent — not an arbitrary `anyhow::Error`. Returning the
    /// same `Output` type for both success and failure lets tools provide structured data while
    /// still signaling whether the invocation succeeded or failed.
    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>>;

    /// Emits events for a previous execution of the tool.
    fn replay(
        &self,
        _input: Self::Input,
        _output: Self::Output,
        _event_stream: ToolCallEventStream,
        _cx: &mut App,
    ) -> Result<()> {
        Ok(())
    }

    fn erase(self) -> Arc<dyn AnyAgentTool> {
        Arc::new(Erased(Arc::new(self)))
    }
}

pub struct Erased<T>(T);

pub struct AgentToolOutput {
    pub llm_output: Vec<LanguageModelToolResultContent>,
    pub raw_output: serde_json::Value,
    pub error: Option<ToolError>,
}

impl From<anyhow::Error> for AgentToolOutput {
    fn from(error: anyhow::Error) -> Self {
        let msg = error.to_string().to_lowercase();
        let tool_error = if msg.contains("timed out") || msg.contains("timeout") {
            ToolError::TimeoutError(error.to_string())
        } else if msg.contains("permission") || msg.contains("denied") || msg.contains("unauthorized") {
            ToolError::PermissionDenied(error.to_string())
        } else if msg.contains("not found") || msg.contains("no such") {
            ToolError::ToolNotFound(error.to_string())
        } else if msg.contains("validation") || msg.contains("invalid") || msg.contains("malformed") {
            ToolError::ValidationError(error.to_string())
        } else {
            ToolError::ExecutionError(error.to_string())
        };
        let llm_output = vec![LanguageModelToolResultContent::Text(Arc::from(
            format!("Error: {}", tool_error),
        ))];
        let raw_output = serde_json::to_value(&llm_output).unwrap_or_else(|e| {
            log::error!("Failed to serialize tool output: {e}");
            serde_json::Value::Null
        });
        Self {
            raw_output,
            llm_output,
            error: Some(tool_error),
        }
    }
}

pub trait AnyAgentTool {
    fn name(&self) -> SharedString;
    fn description(&self) -> SharedString;
    fn kind(&self) -> acp::ToolKind;
    fn initial_title(&self, input: serde_json::Value, _cx: &mut App) -> SharedString;
    fn input_schema(&self, format: LanguageModelToolSchemaFormat) -> Result<serde_json::Value>;
    fn supports_input_streaming(&self) -> bool {
        false
    }
    fn supports_provider(&self, _provider: &LanguageModelProviderId) -> bool {
        true
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<serde_json::Value>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<AgentToolOutput, AgentToolOutput>>;
    fn replay(
        &self,
        input: serde_json::Value,
        output: serde_json::Value,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Result<()>;
}

impl<T> AnyAgentTool for Erased<Arc<T>>
where
    T: AgentTool,
{
    fn name(&self) -> SharedString {
        T::NAME.into()
    }

    fn description(&self) -> SharedString {
        T::description()
    }

    fn kind(&self) -> acp::ToolKind {
        T::kind()
    }

    fn supports_input_streaming(&self) -> bool {
        T::supports_input_streaming()
    }

    fn initial_title(&self, input: serde_json::Value, _cx: &mut App) -> SharedString {
        let parsed_input = serde_json::from_value(input.clone()).map_err(|_| input);
        self.0.initial_title(parsed_input, _cx)
    }

    fn input_schema(&self, format: LanguageModelToolSchemaFormat) -> Result<serde_json::Value> {
        let mut json = serde_json::to_value(T::input_schema(format))?;
        language_model::tool_schema::adapt_schema_to_format(&mut json, format)?;
        Ok(json)
    }

    fn supports_provider(&self, provider: &LanguageModelProviderId) -> bool {
        T::supports_provider(provider)
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<serde_json::Value>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<AgentToolOutput, AgentToolOutput>> {
        let tool_input: ToolInput<T::Input> = input.cast();
        let task = self.0.clone().run(tool_input, event_stream, cx);
        cx.spawn(async move |_cx| match task.await {
            Ok(output) => {
                let raw_output = serde_json::to_value(&output).unwrap_or_else(|e| {
                    log::error!("Failed to serialize tool output: {e}");
                    serde_json::Value::Null
                });
                Ok(AgentToolOutput {
                    raw_output,
                    llm_output: vec![output.into()],
                    error: None,
                })
            }
            Err(error_output) => {
                let raw_value = serde_json::to_value(&error_output).unwrap_or_else(|e| {
                    log::error!("Failed to serialize tool error output: {e}");
                    serde_json::Value::Null
                });
                let error_text = serde_json::to_string(&raw_value).unwrap_or_default();
                Err(AgentToolOutput {
                    llm_output: vec![error_output.into()],
                    raw_output: raw_value,
                    error: Some(ToolError::ExecutionError(error_text)),
                })
            }
        })
    }

    fn replay(
        &self,
        input: serde_json::Value,
        output: serde_json::Value,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Result<()> {
        let input = serde_json::from_value(input)?;
        let output = serde_json::from_value(output)?;
        self.0.replay(input, output, event_stream, cx)
    }
}

