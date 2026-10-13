//! Pure provider/model capability catalog and exact selection validation.
//!
//! This module performs no authentication, network I/O, inference, or tool execution.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

use carapana_protocol::Selection;
use futures_core::Stream;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderMessageRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderMessage {
    pub role: ProviderMessageRole,
    pub content: String,
}

/// Content-bearing input. Callers must enforce the sharing confirmation required by PD-10
/// before passing this request to a remote provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub selection: ResolvedModelSelection,
    pub messages: Vec<ProviderMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta(String),
    /// Untrusted model output only; receiving this event never authorizes tool execution.
    ToolCallRequested {
        id: String,
        name: String,
        arguments_json: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderResponseShape {
    ErrorObject,
    Detail,
    Empty,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRemoteError {
    pub status: Option<u16>,
    pub code: Option<String>,
    pub parameter: Option<String>,
    pub request_id: Option<String>,
    pub response_shape: Option<ProviderResponseShape>,
}

impl ProviderRemoteError {
    pub fn local() -> Self {
        Self {
            status: None,
            code: None,
            parameter: None,
            request_id: None,
            response_shape: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    AuthenticationRequired(ProviderRemoteError),
    ModelUnavailable(ProviderRemoteError),
    RateLimited {
        retry_after: Option<Duration>,
        remote: ProviderRemoteError,
    },
    TemporarilyUnavailable(ProviderRemoteError),
    InvalidResponse(ProviderRemoteError),
    RemoteRejected(ProviderRemoteError),
    Cancelled,
}

pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<ProviderEvent, ProviderError>> + Send + 'static>>;

/// Runtime-independent cancellation signal for one provider request.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

#[derive(Debug, Default)]
struct CancellationState {
    cancelled: AtomicBool,
    next_waiter_id: AtomicU64,
    waiters: Mutex<BTreeMap<u64, Waker>>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self {
            state: Arc::new(CancellationState::default()),
        }
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if self.state.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }

        let waiters = {
            let mut registered = self
                .state
                .waiters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *registered)
        };

        for waiter in waiters.into_values() {
            waiter.wake();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    pub fn cancelled(&self) -> Cancelled<'_> {
        let waiter_id = self.state.next_waiter_id.fetch_add(1, Ordering::Relaxed);
        Cancelled {
            token: self,
            waiter_id,
        }
    }
}

#[must_use = "futures do nothing unless polled"]
pub struct Cancelled<'a> {
    token: &'a CancellationToken,
    waiter_id: u64,
}

impl Future for Cancelled<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let token = this.token;
        if token.is_cancelled() {
            return Poll::Ready(());
        }

        let mut waiters = token
            .state
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if token.is_cancelled() {
            return Poll::Ready(());
        }
        waiters.insert(this.waiter_id, context.waker().clone());
        if token.is_cancelled() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl Drop for Cancelled<'_> {
    fn drop(&mut self) {
        let mut waiters = self
            .token
            .state
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        waiters.remove(&self.waiter_id);
    }
}

/// Provider boundary. Implementations return model output as data and must observe the
/// cancellation token; this trait does not authorize or execute tool calls.
pub trait ProviderAdapter: Send + Sync {
    fn stream(&self, request: ProviderRequest, cancellation: CancellationToken) -> ProviderStream;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub streaming: bool,
    pub cancellation: bool,
    pub tool_calling: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDescriptor {
    pub id: String,
    pub capabilities: ModelCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub id: String,
    pub variants: Vec<VariantDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: String,
    pub models: Vec<ModelDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModelSelection {
    pub provider: String,
    pub model: String,
    pub variant: String,
    pub capabilities: ModelCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogDefinitionError {
    EmptyProviderId,
    DuplicateProviderId,
    ProviderWithoutModels,
    EmptyModelId,
    DuplicateModelId,
    ModelWithoutVariants,
    EmptyVariantId,
    DuplicateVariantId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionResolutionError {
    ProviderUnavailable,
    ModelUnavailable,
    VariantUnsupported,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderCatalog {
    providers: BTreeMap<String, ProviderDescriptor>,
}

impl ProviderCatalog {
    /// Build a catalog from already obtained, trusted metadata.
    ///
    /// An empty catalog is valid and represents no available providers. Provider adapters
    /// and callers remain responsible for obtaining or authorizing these descriptors.
    pub fn new(
        providers: impl IntoIterator<Item = ProviderDescriptor>,
    ) -> Result<Self, CatalogDefinitionError> {
        let mut catalog = Self::default();
        for provider in providers {
            validate_provider(&provider)?;
            if catalog
                .providers
                .insert(provider.id.clone(), provider)
                .is_some()
            {
                return Err(CatalogDefinitionError::DuplicateProviderId);
            }
        }
        Ok(catalog)
    }

    /// Resolve a captured selection exactly; this never substitutes another provider,
    /// model, or variant.
    pub fn resolve(
        &self,
        selection: &Selection,
    ) -> Result<ResolvedModelSelection, SelectionResolutionError> {
        let provider = self
            .providers
            .get(&selection.provider)
            .ok_or(SelectionResolutionError::ProviderUnavailable)?;
        let model = provider
            .models
            .iter()
            .find(|model| model.id == selection.model)
            .ok_or(SelectionResolutionError::ModelUnavailable)?;
        let variant = model
            .variants
            .iter()
            .find(|variant| variant.id == selection.variant)
            .ok_or(SelectionResolutionError::VariantUnsupported)?;

        Ok(ResolvedModelSelection {
            provider: provider.id.clone(),
            model: model.id.clone(),
            variant: variant.id.clone(),
            capabilities: variant.capabilities,
        })
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// Iterate through the known providers and their model/variant descriptors.
    pub fn providers(&self) -> impl Iterator<Item = &ProviderDescriptor> {
        self.providers.values()
    }
}

fn validate_provider(provider: &ProviderDescriptor) -> Result<(), CatalogDefinitionError> {
    if provider.id.trim().is_empty() {
        return Err(CatalogDefinitionError::EmptyProviderId);
    }
    if provider.models.is_empty() {
        return Err(CatalogDefinitionError::ProviderWithoutModels);
    }

    let mut model_ids = BTreeMap::new();
    for model in &provider.models {
        if model.id.trim().is_empty() {
            return Err(CatalogDefinitionError::EmptyModelId);
        }
        if model_ids.insert(model.id.as_str(), ()).is_some() {
            return Err(CatalogDefinitionError::DuplicateModelId);
        }
        if model.variants.is_empty() {
            return Err(CatalogDefinitionError::ModelWithoutVariants);
        }

        let mut variant_ids = BTreeMap::new();
        for variant in &model.variants {
            if variant.id.trim().is_empty() {
                return Err(CatalogDefinitionError::EmptyVariantId);
            }
            if variant_ids.insert(variant.id.as_str(), ()).is_some() {
                return Err(CatalogDefinitionError::DuplicateVariantId);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll, Wake, Waker},
        time::Duration,
    };

    use carapana_protocol::{Autonomy, Selection, WorkMode};
    use futures_core::Stream;

    use super::{
        CancellationToken, CatalogDefinitionError, ModelCapabilities, ModelDescriptor,
        ProviderAdapter, ProviderCatalog, ProviderDescriptor, ProviderError, ProviderEvent,
        ProviderMessage, ProviderMessageRole, ProviderRemoteError, ProviderRequest, ProviderStream,
        SelectionResolutionError, VariantDescriptor,
    };

    fn selection(provider: &str, model: &str, variant: &str) -> Selection {
        Selection {
            profile: "ask".into(),
            work: WorkMode::Execute,
            autonomy: Autonomy::Ask,
            provider: provider.into(),
            model: model.into(),
            variant: variant.into(),
        }
    }

    fn catalog() -> ProviderCatalog {
        ProviderCatalog::new([ProviderDescriptor {
            id: "mock".into(),
            models: vec![ModelDescriptor {
                id: "mock-model".into(),
                variants: vec![
                    VariantDescriptor {
                        id: "default".into(),
                        capabilities: ModelCapabilities {
                            streaming: true,
                            cancellation: true,
                            tool_calling: false,
                        },
                    },
                    VariantDescriptor {
                        id: "extended".into(),
                        capabilities: ModelCapabilities {
                            streaming: true,
                            cancellation: true,
                            tool_calling: true,
                        },
                    },
                ],
            }],
        }])
        .unwrap()
    }

    fn provider_request() -> ProviderRequest {
        ProviderRequest {
            selection: catalog()
                .resolve(&selection("mock", "mock-model", "extended"))
                .unwrap(),
            messages: vec![ProviderMessage {
                role: ProviderMessageRole::User,
                content: "hello".into(),
            }],
        }
    }

    struct ScriptedProvider {
        events: Vec<Result<ProviderEvent, ProviderError>>,
    }

    impl ProviderAdapter for ScriptedProvider {
        fn stream(
            &self,
            request: ProviderRequest,
            cancellation: CancellationToken,
        ) -> ProviderStream {
            assert_eq!(request.selection.provider, "mock");
            Box::pin(ScriptedStream {
                events: self.events.clone().into(),
                cancellation,
            })
        }
    }

    struct ScriptedStream {
        events: VecDeque<Result<ProviderEvent, ProviderError>>,
        cancellation: CancellationToken,
    }

    impl Stream for ScriptedStream {
        type Item = Result<ProviderEvent, ProviderError>;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();
            if this.cancellation.is_cancelled() {
                return Poll::Ready(None);
            }
            Poll::Ready(this.events.pop_front())
        }
    }

    struct FlagWake(AtomicBool);

    impl Wake for FlagWake {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::Release);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.store(true, Ordering::Release);
        }
    }

    fn test_waker() -> (Arc<FlagWake>, Waker) {
        let wake = Arc::new(FlagWake(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&wake));
        (wake, waker)
    }

    #[test]
    fn should_resolve_an_exact_model_variant_and_its_capabilities() {
        let catalog = catalog();
        let resolved = catalog
            .resolve(&selection("mock", "mock-model", "default"))
            .unwrap();

        assert_eq!(resolved.provider, "mock");
        assert_eq!(resolved.model, "mock-model");
        assert_eq!(resolved.variant, "default");
        assert_eq!(
            resolved.capabilities,
            ModelCapabilities {
                streaming: true,
                cancellation: true,
                tool_calling: false,
            }
        );

        let extended = catalog
            .resolve(&selection("mock", "mock-model", "extended"))
            .unwrap();
        assert!(extended.capabilities.tool_calling);
    }

    #[test]
    fn should_reject_unavailable_provider_model_and_variant_without_fallback() {
        let catalog = catalog();
        assert_eq!(
            catalog.resolve(&selection("unknown", "mock-model", "default")),
            Err(SelectionResolutionError::ProviderUnavailable)
        );
        assert_eq!(
            catalog.resolve(&selection("mock", "unknown", "default")),
            Err(SelectionResolutionError::ModelUnavailable)
        );
        assert_eq!(
            catalog.resolve(&selection("mock", "mock-model", "unsupported")),
            Err(SelectionResolutionError::VariantUnsupported)
        );
    }

    #[test]
    fn should_reject_duplicate_ids_at_each_catalog_level() {
        let duplicate_variant = ProviderDescriptor {
            id: "mock".into(),
            models: vec![ModelDescriptor {
                id: "model".into(),
                variants: vec![
                    VariantDescriptor {
                        id: "same".into(),
                        capabilities: ModelCapabilities::default(),
                    },
                    VariantDescriptor {
                        id: "same".into(),
                        capabilities: ModelCapabilities::default(),
                    },
                ],
            }],
        };
        assert_eq!(
            ProviderCatalog::new([duplicate_variant]),
            Err(CatalogDefinitionError::DuplicateVariantId)
        );

        let duplicate_model = ProviderDescriptor {
            id: "mock".into(),
            models: vec![
                ModelDescriptor {
                    id: "same".into(),
                    variants: vec![VariantDescriptor {
                        id: "default".into(),
                        capabilities: ModelCapabilities::default(),
                    }],
                },
                ModelDescriptor {
                    id: "same".into(),
                    variants: vec![VariantDescriptor {
                        id: "other".into(),
                        capabilities: ModelCapabilities::default(),
                    }],
                },
            ],
        };
        assert_eq!(
            ProviderCatalog::new([duplicate_model]),
            Err(CatalogDefinitionError::DuplicateModelId)
        );

        let provider = ProviderDescriptor {
            id: "mock".into(),
            models: vec![ModelDescriptor {
                id: "model".into(),
                variants: vec![VariantDescriptor {
                    id: "default".into(),
                    capabilities: ModelCapabilities::default(),
                }],
            }],
        };
        assert_eq!(
            ProviderCatalog::new([provider.clone(), provider]),
            Err(CatalogDefinitionError::DuplicateProviderId)
        );
    }

    #[test]
    fn should_reject_blank_ids_and_empty_provider_model_or_variant_lists() {
        assert_eq!(
            ProviderCatalog::new([ProviderDescriptor {
                id: "  ".into(),
                models: vec![],
            }]),
            Err(CatalogDefinitionError::EmptyProviderId)
        );
        assert_eq!(
            ProviderCatalog::new([ProviderDescriptor {
                id: "mock".into(),
                models: vec![],
            }]),
            Err(CatalogDefinitionError::ProviderWithoutModels)
        );
        assert_eq!(
            ProviderCatalog::new([ProviderDescriptor {
                id: "mock".into(),
                models: vec![ModelDescriptor {
                    id: "  ".into(),
                    variants: vec![VariantDescriptor {
                        id: "default".into(),
                        capabilities: ModelCapabilities::default(),
                    }],
                }],
            }]),
            Err(CatalogDefinitionError::EmptyModelId)
        );
        assert_eq!(
            ProviderCatalog::new([ProviderDescriptor {
                id: "mock".into(),
                models: vec![ModelDescriptor {
                    id: "model".into(),
                    variants: vec![],
                }],
            }]),
            Err(CatalogDefinitionError::ModelWithoutVariants)
        );
        assert_eq!(
            ProviderCatalog::new([ProviderDescriptor {
                id: "mock".into(),
                models: vec![ModelDescriptor {
                    id: "model".into(),
                    variants: vec![VariantDescriptor {
                        id: "  ".into(),
                        capabilities: ModelCapabilities::default(),
                    }],
                }],
            }]),
            Err(CatalogDefinitionError::EmptyVariantId)
        );
    }

    #[test]
    fn should_allow_an_empty_catalog_to_represent_no_available_providers() {
        let catalog = ProviderCatalog::new([]).unwrap();
        assert_eq!(catalog.provider_count(), 0);
        assert_eq!(catalog.providers().count(), 0);
        assert_eq!(
            catalog.resolve(&selection("mock", "model", "default")),
            Err(SelectionResolutionError::ProviderUnavailable)
        );
    }

    #[test]
    fn should_expose_provider_descriptors_for_capability_discovery() {
        let catalog = catalog();
        let providers = catalog.providers().collect::<Vec<_>>();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "mock");
        assert_eq!(providers[0].models[0].variants.len(), 2);
    }

    #[test]
    fn should_stream_provider_text_and_inert_tool_call_events() {
        let provider = ScriptedProvider {
            events: vec![
                Ok(ProviderEvent::TextDelta("answer".into())),
                Ok(ProviderEvent::ToolCallRequested {
                    id: "call-1".into(),
                    name: "read_file".into(),
                    arguments_json: r#"{"path":"README.md"}"#.into(),
                }),
            ],
        };
        let cancellation = CancellationToken::new();
        let mut stream = provider.stream(provider_request(), cancellation);
        let (_wake, waker) = test_waker();
        let mut context = Context::from_waker(&waker);

        assert_eq!(
            stream.as_mut().poll_next(&mut context),
            Poll::Ready(Some(Ok(ProviderEvent::TextDelta("answer".into()))))
        );
        assert_eq!(
            stream.as_mut().poll_next(&mut context),
            Poll::Ready(Some(Ok(ProviderEvent::ToolCallRequested {
                id: "call-1".into(),
                name: "read_file".into(),
                arguments_json: r#"{"path":"README.md"}"#.into(),
            })))
        );
        assert_eq!(stream.as_mut().poll_next(&mut context), Poll::Ready(None));
    }

    #[test]
    fn should_preserve_typed_provider_errors_without_automatic_retry() {
        let provider = ScriptedProvider {
            events: vec![Err(ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(3)),
                remote: ProviderRemoteError {
                    status: Some(429),
                    code: Some("rate_limit_exceeded".into()),
                    request_id: Some("req-test".into()),
                    ..ProviderRemoteError::local()
                },
            })],
        };
        let mut stream = provider.stream(provider_request(), CancellationToken::new());
        let (_wake, waker) = test_waker();
        let mut context = Context::from_waker(&waker);

        assert_eq!(
            stream.as_mut().poll_next(&mut context),
            Poll::Ready(Some(Err(ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(3)),
                remote: ProviderRemoteError {
                    status: Some(429),
                    code: Some("rate_limit_exceeded".into()),
                    request_id: Some("req-test".into()),
                    ..ProviderRemoteError::local()
                },
            })))
        );
        assert_eq!(stream.as_mut().poll_next(&mut context), Poll::Ready(None));
    }

    #[test]
    fn should_stop_a_stream_after_its_cancellation_token_is_cancelled() {
        let provider = ScriptedProvider {
            events: vec![Ok(ProviderEvent::TextDelta("first".into()))],
        };
        let cancellation = CancellationToken::new();
        let mut stream = provider.stream(provider_request(), cancellation.clone());
        let (_wake, waker) = test_waker();
        let mut context = Context::from_waker(&waker);

        assert!(matches!(
            stream.as_mut().poll_next(&mut context),
            Poll::Ready(Some(Ok(ProviderEvent::TextDelta(_))))
        ));
        cancellation.cancel();
        assert!(cancellation.is_cancelled());
        assert_eq!(stream.as_mut().poll_next(&mut context), Poll::Ready(None));
    }

    #[test]
    fn should_wake_async_waiters_when_cancellation_is_requested() {
        let cancellation = CancellationToken::new();
        let (wake, waker) = test_waker();
        let mut context = Context::from_waker(&waker);
        let mut cancelled = Box::pin(cancellation.cancelled());

        assert_eq!(cancelled.as_mut().poll(&mut context), Poll::Pending);
        cancellation.cancel();
        assert!(wake.0.load(Ordering::Acquire));
        assert_eq!(cancelled.as_mut().poll(&mut context), Poll::Ready(()));
    }

    #[test]
    fn should_unregister_dropped_async_cancellation_waiters() {
        let cancellation = CancellationToken::new();
        let (wake, waker) = test_waker();
        let mut context = Context::from_waker(&waker);
        {
            let mut cancelled = Box::pin(cancellation.cancelled());
            assert_eq!(cancelled.as_mut().poll(&mut context), Poll::Pending);
        }

        cancellation.cancel();
        assert!(!wake.0.load(Ordering::Acquire));
    }
}
