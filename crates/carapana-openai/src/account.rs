use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

const PLAN_USAGE_SCOPE: &str = "chatgpt.tokens.use.direct";

/// Non-sensitive account information suitable for CLI output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountSummary {
    pub client_id: String,
    pub subject: String,
    pub email: Option<String>,
    pub plan_usage_enabled: bool,
    pub has_refresh_token: bool,
    pub expires_at_unix_seconds: u64,
}

/// Validated SIWC account credentials. Secret values intentionally have no `Debug` or
/// `Display` implementation and are only exposed inside this integration crate.
pub struct ChatGptAccount {
    client_id: String,
    subject: String,
    email: Option<String>,
    scopes: Vec<String>,
    access_token: SecretString,
    refresh_token: Option<SecretString>,
    id_token: SecretString,
    expires_at_unix_seconds: u64,
}

pub(crate) struct ChatGptAccountData {
    pub(crate) client_id: String,
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) scopes: Vec<String>,
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: String,
    pub(crate) expires_at_unix_seconds: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredAccount {
    pub(crate) version: u8,
    pub(crate) client_id: String,
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) scopes: Vec<String>,
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: String,
    pub(crate) expires_at_unix_seconds: u64,
}

impl ChatGptAccount {
    pub(crate) fn new(data: ChatGptAccountData) -> Self {
        Self {
            client_id: data.client_id,
            subject: data.subject,
            email: data.email,
            scopes: data.scopes,
            access_token: SecretString::from(data.access_token),
            refresh_token: data.refresh_token.map(SecretString::from),
            id_token: SecretString::from(data.id_token),
            expires_at_unix_seconds: data.expires_at_unix_seconds,
        }
    }

    pub fn summary(&self) -> AccountSummary {
        AccountSummary {
            client_id: self.client_id.clone(),
            subject: self.subject.clone(),
            email: self.email.clone(),
            plan_usage_enabled: self.plan_usage_enabled(),
            has_refresh_token: self.refresh_token.is_some(),
            expires_at_unix_seconds: self.expires_at_unix_seconds,
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn email(&self) -> Option<&str> {
        self.email.as_deref()
    }

    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub fn plan_usage_enabled(&self) -> bool {
        self.scopes.iter().any(|scope| scope == PLAN_USAGE_SCOPE)
    }

    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }

    pub(crate) fn access_token(&self) -> &str {
        self.access_token.expose_secret()
    }

    pub(crate) fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_ref().map(ExposeSecret::expose_secret)
    }

    pub(crate) fn id_token(&self) -> &str {
        self.id_token.expose_secret()
    }

    pub(crate) fn without_refresh_token(&self) -> Self {
        Self::new(ChatGptAccountData {
            client_id: self.client_id.clone(),
            subject: self.subject.clone(),
            email: self.email.clone(),
            scopes: self.scopes.clone(),
            access_token: self.access_token.expose_secret().to_owned(),
            refresh_token: None,
            id_token: self.id_token.expose_secret().to_owned(),
            expires_at_unix_seconds: self.expires_at_unix_seconds,
        })
    }

    pub(crate) fn to_stored(&self) -> StoredAccount {
        StoredAccount {
            version: 1,
            client_id: self.client_id.clone(),
            subject: self.subject.clone(),
            email: self.email.clone(),
            scopes: self.scopes.clone(),
            access_token: self.access_token.expose_secret().to_owned(),
            refresh_token: self
                .refresh_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            id_token: self.id_token.expose_secret().to_owned(),
            expires_at_unix_seconds: self.expires_at_unix_seconds,
        }
    }

    pub(crate) fn from_stored(record: StoredAccount) -> Option<Self> {
        if record.version != 1
            || !valid_client_id(&record.client_id)
            || record.subject.is_empty()
            || record.subject.len() > 4096
            || record
                .email
                .as_ref()
                .is_some_and(|email| email.len() > 4096)
            || record.scopes.len() > 100
            || record
                .scopes
                .iter()
                .any(|scope| scope.is_empty() || scope.len() > 256)
            || record.access_token.is_empty()
            || record.access_token.len() > 16_384
            || record
                .refresh_token
                .as_ref()
                .is_some_and(|token| token.is_empty() || token.len() > 16_384)
            || record.id_token.is_empty()
            || record.id_token.len() > 32_768
            || record.expires_at_unix_seconds == 0
        {
            return None;
        }

        Some(Self::new(ChatGptAccountData {
            client_id: record.client_id,
            subject: record.subject,
            email: record.email,
            scopes: record.scopes,
            access_token: record.access_token,
            refresh_token: record.refresh_token,
            id_token: record.id_token,
            expires_at_unix_seconds: record.expires_at_unix_seconds,
        }))
    }
}

fn valid_client_id(client_id: &str) -> bool {
    !client_id.is_empty()
        && client_id.len() <= 200
        && client_id != "dynamic_agent_client"
        && client_id.bytes().all(|byte| byte.is_ascii_graphic())
}
