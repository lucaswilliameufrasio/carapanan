//! Typed configuration layering and provenance; no filesystem or environment IO.

use carapana_protocol::{Autonomy, WorkMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigSource {
    Defaults,
    System,
    User,
    Project,
    Environment,
    Cli,
    Session,
}

impl ConfigSource {
    fn precedence(self) -> u8 {
        match self {
            Self::Defaults => 0,
            Self::System => 1,
            Self::User => 2,
            Self::Project => 3,
            Self::Environment => 4,
            Self::Cli => 5,
            Self::Session => 6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigOverrides {
    pub work_mode: Option<WorkMode>,
    pub autonomy: Option<Autonomy>,
    pub model: Option<ModelSelection>,
    pub reasoning: Option<ReasoningEffort>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReasoningEffort {
    #[default]
    Default,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigLayer {
    pub source: ConfigSource,
    pub overrides: ConfigOverrides,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub source: ConfigSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub work_mode: Sourced<WorkMode>,
    pub autonomy: Sourced<Autonomy>,
    pub model: Sourced<Option<ModelSelection>>,
    pub reasoning: Sourced<ReasoningEffort>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    DuplicateSource(ConfigSource),
    DefaultsLayerNotAllowed,
    ProjectCannotElevateAutonomy,
    EmptyProvider,
    EmptyModel,
}

impl ConfigLayer {
    pub fn new(source: ConfigSource, overrides: ConfigOverrides) -> Self {
        Self { source, overrides }
    }
}

impl EffectiveConfig {
    pub fn resolve(layers: &[ConfigLayer]) -> Result<Self, ConfigError> {
        let mut ordered: Vec<&ConfigLayer> = layers.iter().collect();
        ordered.sort_by_key(|layer| layer.source.precedence());

        let mut result = Self {
            work_mode: Sourced {
                value: WorkMode::Execute,
                source: ConfigSource::Defaults,
            },
            autonomy: Sourced {
                value: Autonomy::Ask,
                source: ConfigSource::Defaults,
            },
            model: Sourced {
                value: None,
                source: ConfigSource::Defaults,
            },
            reasoning: Sourced {
                value: ReasoningEffort::Default,
                source: ConfigSource::Defaults,
            },
        };

        let mut previous_source = None;
        for layer in ordered {
            if previous_source == Some(layer.source) {
                return Err(ConfigError::DuplicateSource(layer.source));
            }
            previous_source = Some(layer.source);

            if layer.source == ConfigSource::Defaults {
                return Err(ConfigError::DefaultsLayerNotAllowed);
            }
            if layer.source == ConfigSource::Project
                && matches!(
                    layer.overrides.autonomy,
                    Some(Autonomy::Auto | Autonomy::Yolo)
                )
            {
                return Err(ConfigError::ProjectCannotElevateAutonomy);
            }
            if let Some(model) = &layer.overrides.model {
                if model.provider.trim().is_empty() {
                    return Err(ConfigError::EmptyProvider);
                }
                if model.model.trim().is_empty() {
                    return Err(ConfigError::EmptyModel);
                }
            }

            if let Some(value) = layer.overrides.work_mode {
                result.work_mode = Sourced {
                    value,
                    source: layer.source,
                };
            }
            if let Some(value) = layer.overrides.autonomy {
                result.autonomy = Sourced {
                    value,
                    source: layer.source,
                };
            }
            if let Some(value) = &layer.overrides.model {
                result.model = Sourced {
                    value: Some(value.clone()),
                    source: layer.source,
                };
            }
            if let Some(value) = layer.overrides.reasoning {
                result.reasoning = Sourced {
                    value,
                    source: layer.source,
                };
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use carapana_protocol::{Autonomy, WorkMode};

    use super::{
        ConfigError, ConfigLayer, ConfigOverrides, ConfigSource, EffectiveConfig, ModelSelection,
        ReasoningEffort,
    };

    #[test]
    fn should_resolve_each_field_by_precedence_and_report_its_source() {
        let layers = [
            ConfigLayer::new(
                ConfigSource::Session,
                ConfigOverrides {
                    reasoning: Some(ReasoningEffort::High),
                    ..ConfigOverrides::default()
                },
            ),
            ConfigLayer::new(
                ConfigSource::Project,
                ConfigOverrides {
                    work_mode: Some(WorkMode::Plan),
                    model: Some(ModelSelection {
                        provider: "project-provider".into(),
                        model: "project-model".into(),
                    }),
                    ..ConfigOverrides::default()
                },
            ),
            ConfigLayer::new(
                ConfigSource::User,
                ConfigOverrides {
                    work_mode: Some(WorkMode::Execute),
                    autonomy: Some(Autonomy::Auto),
                    model: Some(ModelSelection {
                        provider: "local".into(),
                        model: "qwen-coder".into(),
                    }),
                    reasoning: Some(ReasoningEffort::Low),
                },
            ),
            ConfigLayer::new(
                ConfigSource::System,
                ConfigOverrides {
                    model: Some(ModelSelection {
                        provider: "system-provider".into(),
                        model: "system-model".into(),
                    }),
                    ..ConfigOverrides::default()
                },
            ),
            ConfigLayer::new(
                ConfigSource::Environment,
                ConfigOverrides {
                    autonomy: Some(Autonomy::Yolo),
                    model: Some(ModelSelection {
                        provider: "environment-provider".into(),
                        model: "environment-model".into(),
                    }),
                    ..ConfigOverrides::default()
                },
            ),
            ConfigLayer::new(
                ConfigSource::Cli,
                ConfigOverrides {
                    autonomy: Some(Autonomy::Ask),
                    model: Some(ModelSelection {
                        provider: "cli-provider".into(),
                        model: "cli-model".into(),
                    }),
                    ..ConfigOverrides::default()
                },
            ),
        ];

        let effective = EffectiveConfig::resolve(&layers).unwrap();
        assert_eq!(effective.work_mode.value, WorkMode::Plan);
        assert_eq!(effective.work_mode.source, ConfigSource::Project);
        assert_eq!(effective.autonomy.value, Autonomy::Ask);
        assert_eq!(effective.autonomy.source, ConfigSource::Cli);
        assert_eq!(
            effective.model.value,
            Some(ModelSelection {
                provider: "cli-provider".into(),
                model: "cli-model".into(),
            })
        );
        assert_eq!(effective.model.source, ConfigSource::Cli);
        assert_eq!(effective.reasoning.value, ReasoningEffort::High);
        assert_eq!(effective.reasoning.source, ConfigSource::Session);
    }

    #[test]
    fn should_keep_defaults_explicit_and_reject_duplicate_sources() {
        let defaults = EffectiveConfig::resolve(&[]).unwrap();
        assert_eq!(defaults.autonomy.value, Autonomy::Ask);
        assert_eq!(defaults.autonomy.source, ConfigSource::Defaults);
        assert_eq!(defaults.model.value, None);
        assert_eq!(defaults.model.source, ConfigSource::Defaults);

        let layers = [
            ConfigLayer::new(ConfigSource::User, ConfigOverrides::default()),
            ConfigLayer::new(ConfigSource::User, ConfigOverrides::default()),
        ];
        assert_eq!(
            EffectiveConfig::resolve(&layers),
            Err(ConfigError::DuplicateSource(ConfigSource::User))
        );
        assert_eq!(
            EffectiveConfig::resolve(&[ConfigLayer::new(
                ConfigSource::Defaults,
                ConfigOverrides::default(),
            )]),
            Err(ConfigError::DefaultsLayerNotAllowed)
        );
    }

    #[test]
    fn should_not_allow_project_config_to_elevate_autonomy_or_accept_empty_models() {
        let project_yolo = [ConfigLayer::new(
            ConfigSource::Project,
            ConfigOverrides {
                autonomy: Some(Autonomy::Yolo),
                ..ConfigOverrides::default()
            },
        )];
        assert_eq!(
            EffectiveConfig::resolve(&project_yolo),
            Err(ConfigError::ProjectCannotElevateAutonomy)
        );

        let empty_model = [ConfigLayer::new(
            ConfigSource::Cli,
            ConfigOverrides {
                model: Some(ModelSelection {
                    provider: "provider".into(),
                    model: "  ".into(),
                }),
                ..ConfigOverrides::default()
            },
        )];
        assert_eq!(
            EffectiveConfig::resolve(&empty_model),
            Err(ConfigError::EmptyModel)
        );
    }
}
