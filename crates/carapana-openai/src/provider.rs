use crate::ChatGptAccount;
use async_stream::stream;
use carapana_core::provider::{
    CancellationToken, ModelCapabilities, ModelDescriptor, ProviderAdapter, ProviderDescriptor,
    ProviderError, ProviderEvent, ProviderMessageRole, ProviderRemoteError, ProviderRequest,
    ProviderResponseShape, ProviderStream, VariantDescriptor,
};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PROVIDER_ID: &str = "chatgpt";
const DEFAULT_VARIANT_ID: &str = "default";
const OFFICIAL_API_BASE: &str = "https://api.openai.com/v1/";
const MAX_API_BODY_BYTES: usize = 1024 * 1024;
const MAX_STREAM_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_OUTPUT_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatGptModel {
    pub slug: String,
    pub display_name: String,
}

/// ChatGPT Responses adapter. This sends only messages supplied in a provider request;
/// it does not load sessions, workspace files, tools, or queued work.
pub struct ChatGptProvider {
    client: reqwest::Client,
    access_token: SecretString,
    api_base: openidconnect::url::Url,
}

impl ChatGptProvider {
    pub fn new(account: ChatGptAccount) -> Result<Self, ProviderError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        if !account.plan_usage_enabled() || now >= account.expires_at_unix_seconds() {
            return Err(ProviderError::AuthenticationRequired(
                ProviderRemoteError::local(),
            ));
        }
        let access_token = SecretString::from(account.access_token().to_owned());
        Self::with_base_url(access_token, OFFICIAL_API_BASE)
    }

    fn with_base_url(access_token: SecretString, api_base: &str) -> Result<Self, ProviderError> {
        let api_base = openidconnect::url::Url::parse(api_base)
            .map_err(|_| ProviderError::InvalidResponse(ProviderRemoteError::local()))?;
        if api_base.cannot_be_a_base()
            || !matches!(api_base.scheme(), "https" | "http")
            || api_base.host_str().is_none()
            || !api_base.path().ends_with('/')
        {
            return Err(ProviderError::InvalidResponse(ProviderRemoteError::local()));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|_| ProviderError::TemporarilyUnavailable(ProviderRemoteError::local()))?;
        Ok(Self {
            client,
            access_token,
            api_base,
        })
    }

    pub async fn available_models(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<ChatGptModel>, ProviderError> {
        let url = self
            .api_base
            .join("models")
            .map_err(|_| ProviderError::InvalidResponse(ProviderRemoteError::local()))?;
        let request = self.authorized(self.client.get(url))?;
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            response = request.send() => response.map_err(|_| {
                ProviderError::TemporarilyUnavailable(ProviderRemoteError::local())
            })?,
        };
        let remote = remote_error_from_headers(response.status().as_u16(), response.headers());
        let retry_after = retry_after_from_headers(response.headers());
        let status = response.status();
        let body = self.read_limited_body(response, &cancellation).await?;
        if !status.is_success() {
            let payload = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            return Err(classify_remote_error(
                remote_with_payload(remote, &payload),
                retry_after,
            ));
        }
        let payload = serde_json::from_slice::<Value>(&body).map_err(|_| {
            ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(status.as_u16()),
                ..ProviderRemoteError::local()
            })
        })?;
        parse_models(payload, status.as_u16())
    }

    pub async fn catalog(
        &self,
        cancellation: CancellationToken,
    ) -> Result<ProviderDescriptor, ProviderError> {
        let models = self.available_models(cancellation).await?;
        let mut descriptors = Vec::with_capacity(models.len());
        for model in models {
            descriptors.push(ModelDescriptor {
                id: model.slug,
                variants: vec![VariantDescriptor {
                    id: DEFAULT_VARIANT_ID.to_owned(),
                    capabilities: ModelCapabilities {
                        streaming: true,
                        cancellation: true,
                        tool_calling: false,
                    },
                }],
            });
        }
        if descriptors.is_empty() {
            return Err(ProviderError::ModelUnavailable(ProviderRemoteError {
                status: Some(200),
                ..ProviderRemoteError::local()
            }));
        }
        Ok(ProviderDescriptor {
            id: PROVIDER_ID.to_owned(),
            models: descriptors,
        })
    }

    fn authorized(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let token = self.access_token.expose_secret();
        let mut authorization =
            match reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
                Ok(value) => value,
                Err(_) => {
                    return Err(ProviderError::AuthenticationRequired(
                        ProviderRemoteError::local(),
                    ));
                }
            };
        authorization.set_sensitive(true);
        Ok(request
            .header(reqwest::header::AUTHORIZATION, authorization)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            ))
    }

    async fn read_limited_body(
        &self,
        response: reqwest::Response,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, ProviderError> {
        let mut body = Vec::new();
        let mut chunks = response.bytes_stream();
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                next = chunks.next() => next,
            };
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk
                .map_err(|_| ProviderError::TemporarilyUnavailable(ProviderRemoteError::local()))?;
            if body.len().saturating_add(chunk.len()) > MAX_API_BODY_BYTES {
                return Err(ProviderError::InvalidResponse(ProviderRemoteError::local()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn response_stream(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> ProviderStream {
        let client = self.client.clone();
        let token = self.access_token.clone();
        let api_base = self.api_base.clone();
        Box::pin(stream! {
            let request_body = match response_request_body(&request) {
                Ok(body) => body,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            let Ok(url) = api_base.join("responses") else {
                yield Err(ProviderError::InvalidResponse(ProviderRemoteError::local()));
                return;
            };
            let mut authorization = match reqwest::header::HeaderValue::from_str(
                &format!("Bearer {}", token.expose_secret()),
            ) {
                Ok(value) => value,
                Err(_) => {
                    yield Err(ProviderError::AuthenticationRequired(ProviderRemoteError::local()));
                    return;
                }
            };
            authorization.set_sensitive(true);
            let request = client
                .post(url)
                .header(reqwest::header::AUTHORIZATION, authorization)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&request_body);
            let response = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    yield Err(ProviderError::Cancelled);
                    return;
                }
                response = request.send() => match response {
                    Ok(response) => response,
                    Err(_) => {
                        yield Err(ProviderError::TemporarilyUnavailable(ProviderRemoteError::local()));
                        return;
                    }
                }
            };
            let status = response.status();
            let remote = remote_error_from_headers(status.as_u16(), response.headers());
            if !status.is_success() {
                let retry_after = retry_after_from_headers(response.headers());
                let body = match collect_body(response, &cancellation).await {
                    Ok(body) => body,
                    Err(error) => {
                        yield Err(error);
                        return;
                    }
                };
                let payload = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
                let remote = remote_with_payload(remote, &payload);
                yield Err(classify_remote_error(remote, retry_after));
                return;
            }

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::trim)
                .map(str::to_ascii_lowercase);
            if matches!(content_type.as_deref(), Some(value) if value != "text/event-stream") {
                let content_type_for_log = content_type
                    .as_deref()
                    .filter(|value| {
                        value.len() <= 80
                            && value.bytes().all(|byte| byte.is_ascii_graphic())
                    })
                    .unwrap_or("<missing-or-invalid>");
                eprintln!(
                    "ChatGPT Responses API returned HTTP 200 with unexpected Content-Type: {content_type_for_log}"
                );
                let code = unexpected_content_type_code(content_type.as_deref());
                yield Err(invalid_stream_response(
                    remote,
                    &code,
                ));
                return;
            }

            let mut response_chunks = response.bytes_stream();
            let bounded_chunks = stream! {
                let mut total_bytes = 0usize;
                loop {
                    match response_chunks.next().await {
                        Some(Ok(chunk)) => {
                            total_bytes = total_bytes.saturating_add(chunk.len());
                            if total_bytes > MAX_STREAM_BODY_BYTES {
                                yield Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "provider event stream exceeded its size limit",
                                ));
                                return;
                            }
                            yield Ok(chunk);
                        }
                        Some(Err(_)) => {
                            yield Err(std::io::Error::other("provider event stream transport failed"));
                            return;
                        }
                        None => break,
                    }
                }
            };
            let mut events = Box::pin(bounded_chunks.eventsource());
            let mut completed = false;
            let mut output_bytes = 0usize;
            loop {
                let next = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        yield Err(ProviderError::Cancelled);
                        return;
                    }
                    next = events.next() => next,
                };
                let Some(event) = next else {
                    break;
                };
                let event = match event {
                    Ok(event) => event,
                    Err(eventsource_stream::EventStreamError::Transport(error))
                        if error.kind() == std::io::ErrorKind::InvalidData =>
                    {
                        yield Err(invalid_stream_response(
                            remote.clone(),
                            "client_invalid_event_stream",
                        ));
                        return;
                    }
                    Err(eventsource_stream::EventStreamError::Transport(_)) => {
                        yield Err(ProviderError::TemporarilyUnavailable(remote.clone()));
                        return;
                    }
                    Err(_) => {
                        yield Err(invalid_stream_response(
                            remote.clone(),
                            "client_malformed_event_stream",
                        ));
                        return;
                    }
                };
                let payload = match serde_json::from_str::<Value>(&event.data) {
                    Ok(payload) => payload,
                    Err(_) => {
                        yield Err(invalid_stream_response(
                            remote.clone(),
                            "client_invalid_event_json",
                        ));
                        return;
                    }
                };
                let event_type = payload
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or(event.event.as_str());
                match event_type {
                    "response.output_text.delta" => {
                        let Some(delta) = payload.get("delta").and_then(Value::as_str) else {
                            yield Err(invalid_stream_response(
                                remote.clone(),
                                "client_invalid_text_delta",
                            ));
                            return;
                        };
                        output_bytes = output_bytes.saturating_add(delta.len());
                        if output_bytes > MAX_OUTPUT_TEXT_BYTES {
                            yield Err(ProviderError::InvalidResponse(ProviderRemoteError {
                                status: Some(status.as_u16()),
                                code: Some("response_too_large".to_owned()),
                                request_id: remote.request_id.clone(),
                                ..ProviderRemoteError::local()
                            }));
                            return;
                        }
                        yield Ok(ProviderEvent::TextDelta(delta.to_owned()));
                    }
                    "response.output_item.done" => {
                        match parse_tool_call(&payload) {
                            Ok(Some(event)) => yield Ok(event),
                            Ok(None) => {}
                            Err(()) => {
                                yield Err(invalid_stream_response(
                                    remote.clone(),
                                    "client_invalid_tool_event",
                                ));
                                return;
                            }
                        }
                    }
                    "response.failed" | "error" => {
                        let details = remote_with_payload(remote.clone(), &payload);
                        yield Err(classify_remote_error(
                            details,
                            None,
                        ));
                        return;
                    }
                    "response.incomplete" => {
                        yield Err(ProviderError::InvalidResponse(
                            remote_with_code(remote.clone(), Some("response_incomplete".to_owned())),
                        ));
                        return;
                    }
                    "response.completed" => {
                        completed = true;
                        break;
                    }
                    _ => {}
                }
            }
            if !completed {
                yield Err(ProviderError::TemporarilyUnavailable(
                    remote_with_code(remote, Some("stream_interrupted".to_owned())),
                ));
            }
        })
    }
}

impl ProviderAdapter for ChatGptProvider {
    fn stream(&self, request: ProviderRequest, cancellation: CancellationToken) -> ProviderStream {
        self.response_stream(request, cancellation)
    }
}

fn response_request_body(request: &ProviderRequest) -> Result<Value, ProviderError> {
    if request.selection.provider != PROVIDER_ID
        || request.selection.variant != DEFAULT_VARIANT_ID
        || request.selection.model.trim().is_empty()
        || request.selection.model.len() > 200
    {
        return Err(ProviderError::ModelUnavailable(ProviderRemoteError::local()));
    }
    let mut instructions: Vec<&str> = Vec::new();
    let mut input = Vec::new();
    for message in &request.messages {
        match message.role {
            ProviderMessageRole::System => instructions.push(message.content.as_str()),
            ProviderMessageRole::User => input.push(json!({
                "role": "user",
                "content": message.content,
            })),
            ProviderMessageRole::Assistant => input.push(json!({
                "role": "assistant",
                "content": message.content,
            })),
        }
    }
    if input.is_empty() {
        return Err(ProviderError::InvalidResponse(ProviderRemoteError::local()));
    }
    let mut body = json!({
        "model": request.selection.model,
        "input": input,
        "store": false,
        "stream": true,
    });
    if !instructions.is_empty() {
        body["instructions"] = Value::String(instructions.join("\n\n"));
    }
    Ok(body)
}

fn parse_models(payload: Value, status: u16) -> Result<Vec<ChatGptModel>, ProviderError> {
    let Some(models) = payload.get("models").and_then(Value::as_array) else {
        return Err(ProviderError::InvalidResponse(ProviderRemoteError {
            status: Some(status),
            ..ProviderRemoteError::local()
        }));
    };
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for model in models {
        if model.get("visibility").and_then(Value::as_str) != Some("list") {
            continue;
        }
        let Some(slug) = model.get("slug").and_then(Value::as_str) else {
            return Err(ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(status),
                ..ProviderRemoteError::local()
            }));
        };
        let Some(display_name) = model.get("display_name").and_then(Value::as_str) else {
            return Err(ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(status),
                ..ProviderRemoteError::local()
            }));
        };
        if slug.trim().is_empty()
            || slug.len() > 200
            || !slug.bytes().all(|byte| byte.is_ascii_graphic())
            || display_name.trim().is_empty()
            || display_name.len() > 200
            || display_name.chars().any(char::is_control)
            || !seen.insert(slug.to_owned())
        {
            return Err(ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(status),
                ..ProviderRemoteError::local()
            }));
        }
        result.push(ChatGptModel {
            slug: slug.to_owned(),
            display_name: display_name.to_owned(),
        });
    }
    Ok(result)
}

fn parse_tool_call(payload: &Value) -> Result<Option<ProviderEvent>, ()> {
    let Some(item) = payload.get("item") else {
        return Ok(None);
    };
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return Ok(None);
    }
    let id = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .ok_or(())?;
    let name = item.get("name").and_then(Value::as_str).ok_or(())?;
    let arguments_json = item.get("arguments").and_then(Value::as_str).ok_or(())?;
    if id.is_empty()
        || id.len() > 256
        || name.is_empty()
        || name.len() > 256
        || arguments_json.is_empty()
        || arguments_json.len() > 64 * 1024
    {
        return Err(());
    }
    Ok(Some(ProviderEvent::ToolCallRequested {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments_json: arguments_json.to_owned(),
    }))
}

fn remote_error_from_headers(
    status: u16,
    headers: &reqwest::header::HeaderMap,
) -> ProviderRemoteError {
    ProviderRemoteError {
        status: Some(status),
        code: None,
        parameter: None,
        request_id: headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 200 && value.bytes().all(|byte| byte.is_ascii_graphic()))
            .map(str::to_owned),
        response_shape: None,
    }
}

fn remote_with_code(mut remote: ProviderRemoteError, code: Option<String>) -> ProviderRemoteError {
    remote.code = code;
    remote
}

fn invalid_stream_response(remote: ProviderRemoteError, code: &str) -> ProviderError {
    ProviderError::InvalidResponse(remote_with_code(remote, Some(code.to_owned())))
}

fn unexpected_content_type_code(content_type: Option<&str>) -> String {
    let Some(content_type) = content_type else {
        return "client_missing_content_type".to_owned();
    };
    if content_type.len() > 80
        || !content_type.is_ascii()
        || content_type.matches('/').count() != 1
        || content_type.starts_with('/')
        || content_type.ends_with('/')
        || !content_type.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'+' | b'_')
        })
    {
        return "client_invalid_content_type_header".to_owned();
    }
    let safe_media_type = content_type
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("client_unexpected_content_type_{safe_media_type}")
}

fn remote_with_payload(mut remote: ProviderRemoteError, payload: &Value) -> ProviderRemoteError {
    remote.code = extract_error_code(payload);
    remote.parameter = payload
        .pointer("/error/param")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .pointer("/response/error/param")
                .and_then(Value::as_str)
        })
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value.bytes().all(|byte| byte.is_ascii_graphic())
        })
        .map(str::to_owned);
    remote.response_shape = Some(
        if payload.get("error").is_some() || payload.pointer("/response/error").is_some() {
            ProviderResponseShape::ErrorObject
        } else if payload.get("detail").is_some() {
            ProviderResponseShape::Detail
        } else if payload.is_null() {
            ProviderResponseShape::Empty
        } else {
            ProviderResponseShape::Other
        },
    );
    remote
}

fn extract_error_code(payload: &Value) -> Option<String> {
    let code = payload
        .pointer("/error/code")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .pointer("/response/error/code")
                .and_then(Value::as_str)
        })
        .or_else(|| payload.pointer("/error").and_then(Value::as_str))?;
    if code.is_empty()
        || code.len() > 128
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return None;
    }
    Some(code.to_owned())
}

fn retry_after_from_headers(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn classify_remote_error(
    remote: ProviderRemoteError,
    retry_after: Option<Duration>,
) -> ProviderError {
    match (remote.status, remote.code.as_deref()) {
        (Some(401), _) => ProviderError::AuthenticationRequired(remote),
        (Some(404), _) | (_, Some("model_not_found" | "model_unavailable")) => {
            ProviderError::ModelUnavailable(remote)
        }
        (Some(429), _) | (_, Some("subscription_sharing_usage_limit_exceeded")) => {
            ProviderError::RateLimited {
                retry_after,
                remote,
            }
        }
        (Some(status), _) if status >= 500 => ProviderError::TemporarilyUnavailable(remote),
        _ => ProviderError::RemoteRejected(remote),
    }
}

async fn collect_body(
    response: reqwest::Response,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ProviderError> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    loop {
        let next = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            next = chunks.next() => next,
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk
            .map_err(|_| ProviderError::TemporarilyUnavailable(ProviderRemoteError::local()))?;
        if body.len().saturating_add(chunk.len()) > MAX_API_BODY_BYTES {
            return Err(ProviderError::InvalidResponse(ProviderRemoteError::local()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::{
        ChatGptModel, ChatGptProvider, parse_models, parse_tool_call, remote_with_payload,
        response_request_body,
    };
    use crate::{ChatGptAccount, account::ChatGptAccountData};
    use carapana_core::provider::{
        CancellationToken, ModelCapabilities, ProviderAdapter, ProviderError, ProviderEvent,
        ProviderMessage, ProviderMessageRole, ProviderRemoteError, ProviderRequest,
        ProviderResponseShape, ResolvedModelSelection,
    };
    use futures_util::StreamExt;
    use secrecy::SecretString;
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        task::JoinHandle,
    };

    fn request() -> ProviderRequest {
        ProviderRequest {
            selection: ResolvedModelSelection {
                provider: "chatgpt".to_owned(),
                model: "gpt-test-model".to_owned(),
                variant: "default".to_owned(),
                capabilities: ModelCapabilities {
                    streaming: true,
                    cancellation: true,
                    tool_calling: false,
                },
            },
            messages: vec![
                ProviderMessage {
                    role: ProviderMessageRole::System,
                    content: "Synthetic instruction".to_owned(),
                },
                ProviderMessage {
                    role: ProviderMessageRole::User,
                    content: "Synthetic smoke prompt".to_owned(),
                },
                ProviderMessage {
                    role: ProviderMessageRole::Assistant,
                    content: "Synthetic prior response".to_owned(),
                },
            ],
        }
    }

    fn provider(api_base: &str) -> ChatGptProvider {
        ChatGptProvider::with_base_url(
            SecretString::from("synthetic-test-access-token".to_owned()),
            api_base,
        )
        .unwrap()
    }

    #[test]
    fn provider_refuses_inference_without_direct_scope_or_with_expired_access() {
        let no_scope = ChatGptAccount::new(ChatGptAccountData {
            client_id: "oaiapp_test".to_owned(),
            subject: "synthetic-subject".to_owned(),
            email: None,
            scopes: vec!["openid".to_owned()],
            access_token: "synthetic-token".to_owned(),
            refresh_token: Some("synthetic-refresh".to_owned()),
            id_token: "synthetic-id-token".to_owned(),
            expires_at_unix_seconds: u64::MAX,
        });
        let expired = ChatGptAccount::new(ChatGptAccountData {
            client_id: "oaiapp_test".to_owned(),
            subject: "synthetic-subject".to_owned(),
            email: None,
            scopes: vec!["chatgpt.tokens.use.direct".to_owned()],
            access_token: "synthetic-token".to_owned(),
            refresh_token: Some("synthetic-refresh".to_owned()),
            id_token: "synthetic-id-token".to_owned(),
            expires_at_unix_seconds: 1,
        });

        assert!(matches!(
            ChatGptProvider::new(no_scope),
            Err(ProviderError::AuthenticationRequired(_))
        ));
        assert!(matches!(
            ChatGptProvider::new(expired),
            Err(ProviderError::AuthenticationRequired(_))
        ));
    }

    #[test]
    fn response_request_uses_supported_fields_and_keeps_tool_requests_outbound_disabled() {
        let body = response_request_body(&request()).unwrap();

        assert_eq!(body["model"], "gpt-test-model");
        assert_eq!(body["instructions"], "Synthetic instruction");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"], "Synthetic smoke prompt");
        assert_eq!(body["input"][1]["role"], "assistant");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert!(body.get("tools").is_none());
        assert!(body.get("previous_response_id").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn model_catalog_only_exposes_list_visibility_and_preserves_order() {
        let payload = serde_json::json!({
            "models": [
                {"slug": "model-b", "display_name": "Model B", "visibility": "list"},
                {"slug": "hidden-model", "display_name": "Hidden", "visibility": "hidden"},
                {"slug": "model-a", "display_name": "Model A", "visibility": "list"}
            ]
        });

        assert_eq!(
            parse_models(payload, 200).unwrap(),
            vec![
                ChatGptModel {
                    slug: "model-b".to_owned(),
                    display_name: "Model B".to_owned(),
                },
                ChatGptModel {
                    slug: "model-a".to_owned(),
                    display_name: "Model A".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn malformed_listed_model_is_rejected_instead_of_silently_skipped() {
        let payload = serde_json::json!({
            "models": [{"slug": "model-without-name", "visibility": "list"}]
        });

        assert!(matches!(
            parse_models(payload, 200),
            Err(ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(200),
                ..
            }))
        ));
    }

    #[test]
    fn untrusted_model_and_error_fields_cannot_inject_terminal_controls() {
        let model_payload = serde_json::json!({
            "models": [{
                "slug": "model-test",
                "display_name": "Model\u{001b}[31m",
                "visibility": "list"
            }]
        });
        let error_payload = serde_json::json!({
            "error": {"code": "invalid_request", "param": "model\u{001b}[31m"}
        });

        assert!(matches!(
            parse_models(model_payload, 200),
            Err(ProviderError::InvalidResponse(_))
        ));
        assert_eq!(
            remote_with_payload(ProviderRemoteError::local(), &error_payload).parameter,
            None
        );
    }

    #[tokio::test]
    async fn real_http_client_streams_text_and_inert_tool_request_with_minimal_payload() {
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
        );
        let (base_url, server) =
            spawn_server("200 OK", "Content-Type: text/event-stream\r\n", body).await;
        let adapter = provider(&base_url);
        let mut events = adapter.stream(request(), CancellationToken::new());
        let mut observed = Vec::new();
        while let Some(event) = events.next().await {
            observed.push(event.unwrap());
        }
        let wire_request = server.await.unwrap();
        let (headers, request_body) = split_request(&wire_request);
        let json_body: serde_json::Value = serde_json::from_slice(request_body).unwrap();

        assert!(headers.starts_with("POST /v1/responses HTTP/1.1\r\n"));
        assert!(headers.contains("authorization: Bearer synthetic-test-access-token"));
        assert_eq!(json_body["store"], false);
        assert_eq!(json_body["stream"], true);
        assert!(json_body.get("tools").is_none());
        assert_eq!(
            observed,
            vec![
                ProviderEvent::TextDelta("hello".to_owned()),
                ProviderEvent::ToolCallRequested {
                    id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments_json: r#"{"path":"README.md"}"#.to_owned(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn model_catalog_uses_authenticated_account_scoped_endpoint() {
        let (base_url, server) = spawn_server(
            "200 OK",
            "Content-Type: application/json\r\n",
            r#"{"models":[{"slug":"model-b","display_name":"Model B","visibility":"list"},{"slug":"hidden","display_name":"Hidden","visibility":"hidden"}]}"#,
        )
        .await;
        let adapter = provider(&base_url);

        let models = adapter
            .available_models(CancellationToken::new())
            .await
            .unwrap();
        let wire_request = server.await.unwrap();
        let (headers, _) = split_request(&wire_request);

        assert!(headers.starts_with("GET /v1/models HTTP/1.1\r\n"));
        assert!(headers.contains("authorization: Bearer synthetic-test-access-token"));
        assert_eq!(
            models,
            vec![ChatGptModel {
                slug: "model-b".to_owned(),
                display_name: "Model B".to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn usage_limit_preserves_status_code_parameter_request_id_and_retry_after() {
        let (base_url, server) = spawn_server(
            "429 Too Many Requests",
            "Content-Type: application/json\r\nRetry-After: 7\r\nX-Request-Id: req-synthetic\r\n",
            r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","param":"model"}}"#,
        )
        .await;
        let adapter = provider(&base_url);
        let mut events = adapter.stream(request(), CancellationToken::new());
        let error = events.next().await.unwrap().unwrap_err();
        let _ = server.await.unwrap();

        assert_eq!(
            error,
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(7)),
                remote: ProviderRemoteError {
                    status: Some(429),
                    code: Some("subscription_sharing_usage_limit_exceeded".to_owned()),
                    parameter: Some("model".to_owned()),
                    request_id: Some("req-synthetic".to_owned()),
                    response_shape: Some(ProviderResponseShape::ErrorObject),
                },
            }
        );
    }

    #[tokio::test]
    async fn stream_requires_event_stream_content_type() {
        let (base_url, server) = spawn_server(
            "200 OK",
            "Content-Type: application/json\r\n",
            r#"{"type":"response.completed"}"#,
        )
        .await;
        let adapter = provider(&base_url);
        let mut events = adapter.stream(request(), CancellationToken::new());

        assert_eq!(
            events.next().await,
            Some(Err(ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(200),
                code: Some("client_unexpected_content_type_application_json".to_owned()),
                ..ProviderRemoteError::local()
            })))
        );
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_successful_event_reports_a_safe_local_diagnostic_code() {
        let (base_url, server) = spawn_server(
            "200 OK",
            "Content-Type: text/event-stream\r\n",
            "data: not-json\n\n",
        )
        .await;
        let adapter = provider(&base_url);
        let mut events = adapter.stream(request(), CancellationToken::new());

        assert_eq!(
            events.next().await,
            Some(Err(ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(200),
                code: Some("client_invalid_event_json".to_owned()),
                ..ProviderRemoteError::local()
            })))
        );
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn unexpected_media_type_reports_only_its_sanitized_type() {
        let (base_url, server) =
            spawn_server("200 OK", "Content-Type: application/x-ndjson\r\n", "{}").await;
        let adapter = provider(&base_url);
        let mut events = adapter.stream(request(), CancellationToken::new());

        assert_eq!(
            events.next().await,
            Some(Err(ProviderError::InvalidResponse(ProviderRemoteError {
                status: Some(200),
                code: Some("client_unexpected_content_type_application_x_ndjson".to_owned()),
                ..ProviderRemoteError::local()
            })))
        );
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn missing_content_type_accepts_only_a_complete_valid_event_stream() {
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
        );
        let (base_url, server) = spawn_server("200 OK", "", body).await;
        let adapter = provider(&base_url);
        let mut events = adapter.stream(request(), CancellationToken::new());

        assert_eq!(
            events.next().await.unwrap().unwrap(),
            ProviderEvent::TextDelta("hello".to_owned())
        );
        assert!(events.next().await.is_none());
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn stream_eof_before_response_completed_is_not_reported_as_success() {
        let (base_url, server) = spawn_server(
            "200 OK",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
        )
        .await;
        let adapter = provider(&base_url);
        let mut events = adapter.stream(request(), CancellationToken::new());

        assert_eq!(
            events.next().await.unwrap().unwrap(),
            ProviderEvent::TextDelta("partial".to_owned())
        );
        assert!(matches!(
            events.next().await,
            Some(Err(ProviderError::TemporarilyUnavailable(_)))
        ));
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_interrupts_an_open_remote_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _request = read_http_request(&mut stream).await;
            let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ready\"}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
                body.len() + 1024
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            let mut byte = [0_u8; 1];
            let _ = stream.read(&mut byte).await;
        });
        let adapter = provider(&format!("http://{address}/v1/"));
        let cancellation = CancellationToken::new();
        let mut events = adapter.stream(request(), cancellation.clone());

        assert_eq!(
            events.next().await.unwrap().unwrap(),
            ProviderEvent::TextDelta("ready".to_owned())
        );
        cancellation.cancel();
        assert_eq!(events.next().await.unwrap(), Err(ProviderError::Cancelled));
        server.abort();
    }

    #[test]
    fn oversized_tool_request_is_rejected_as_data_without_execution() {
        let payload = serde_json::json!({
            "item": {
                "type": "function_call",
                "call_id": "call-1",
                "name": "read_file",
                "arguments": "x".repeat(64 * 1024 + 1),
            }
        });

        assert_eq!(parse_tool_call(&payload), Err(()));
    }

    #[tokio::test]
    async fn cancellation_before_polling_never_sends_a_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move { listener.accept().await });
        let adapter = provider(&format!("http://{address}/v1/"));
        let cancellation = CancellationToken::new();
        let mut events = adapter.stream(request(), cancellation.clone());
        cancellation.cancel();

        assert_eq!(events.next().await.unwrap(), Err(ProviderError::Cancelled));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), accept)
                .await
                .is_err()
        );
    }

    async fn spawn_server(
        status: &'static str,
        headers: &'static str,
        body: &'static str,
    ) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            let response = format!(
                "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (format!("http://{address}/v1/"), task)
    }

    async fn read_http_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0, "client closed before sending request");
            request.extend_from_slice(&chunk[..read]);
            if let Some((header_end, content_length)) = request_content_length(&request)
                && request.len() >= header_end + content_length
            {
                break;
            }
        }
        String::from_utf8(request).unwrap()
    }

    fn request_content_length(request: &[u8]) -> Option<(usize, usize)> {
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")?
            + 4;
        let headers = std::str::from_utf8(&request[..header_end]).ok()?;
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        Some((header_end, content_length))
    }

    fn split_request(request: &str) -> (&str, &[u8]) {
        let header_end = request.find("\r\n\r\n").unwrap() + 4;
        (&request[..header_end], &request.as_bytes()[header_end..])
    }
}
