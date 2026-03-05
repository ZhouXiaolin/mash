use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_MAX_TOKENS: u32 = 131072;

/// One model provider in settings.json (e.g. deepseek, openai).
#[derive(Debug, Clone, Deserialize)]
pub struct ModelProvider {
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
}

/// Contents of ~/.mash/settings.json
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub model_provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_providers: Vec<ModelProvider>,
}

#[derive(Clone)]
pub struct ApiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
}

impl Settings {
    /// Load from ~/.mash/settings.json. Returns None if file missing or invalid.
    pub fn load() -> Option<Self> {
        let path = dirs::home_dir()?.join(".mash").join("settings.json");
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }
}

impl ApiConfig {
    /// Load from ~/.mash/settings.json using model_provider and model, then fall back to env.
    pub fn load() -> Self {
        if let Some(settings) = Settings::load()
            && let Some(provider) = settings
                .model_providers
                .iter()
                .find(|p| p.name == settings.model_provider)
        {
            let base_url = if provider.base_url.is_empty() {
                DEFAULT_BASE_URL.to_string()
            } else {
                provider.base_url.clone()
            };
            if !provider.api_key.is_empty() {
                return Self {
                    base_url,
                    api_key: provider.api_key.clone(),
                    model: if settings.model.is_empty() {
                        "deepseek-chat".to_string()
                    } else {
                        settings.model
                    },
                    max_tokens: DEFAULT_MAX_TOKENS,
                };
            }
        }

        Self::from_env()
    }

    pub fn from_env() -> Self {
        let api_key = std::env::var("API_KEY").unwrap_or_else(|_| {
            eprintln!("Error: API_KEY not set");
            std::process::exit(1);
        });

        let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());

        let model =
            std::env::var("MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());

        let max_tokens = std::env::var("MAX_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);

        Self {
            base_url,
            api_key,
            model,
            max_tokens,
        }
    }
}

pub fn mash_config_path(filename: &str) -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    Ok(home.join(".mash").join(filename))
}
