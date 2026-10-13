//! Pure provider/model capability catalog and exact selection validation.
//!
//! This module performs no authentication, network I/O, inference, or tool execution.

use std::collections::BTreeMap;

use carapana_protocol::Selection;

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
    use carapana_protocol::{Autonomy, Selection, WorkMode};

    use super::{
        CatalogDefinitionError, ModelCapabilities, ModelDescriptor, ProviderCatalog,
        ProviderDescriptor, SelectionResolutionError, VariantDescriptor,
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
}
