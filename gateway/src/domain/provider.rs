use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMode {
    #[default]
    ApiKey,
    Account,
}

#[derive(Debug, Clone)]
pub enum ProviderCredentials {
    ApiKey {
        base_url: String,
        api_key: String,
    },
    Account {
        email: Option<String>,
        access_token: Option<String>,
        refresh_token: Option<String>,
        expiry_timestamp: Option<i64>,
        client_id: Option<String>,
        account_id: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Provider {
    pub id: String,
    pub name: Option<String>,
    pub credentials: ProviderCredentials,
}

impl Provider {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        self.name.as_deref().or(self.email()).unwrap_or("")
    }

    pub fn auth_mode(&self) -> ProviderAuthMode {
        match self.credentials {
            ProviderCredentials::ApiKey { .. } => ProviderAuthMode::ApiKey,
            ProviderCredentials::Account { .. } => ProviderAuthMode::Account,
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        match &self.credentials {
            ProviderCredentials::ApiKey { base_url, .. } => Some(base_url),
            ProviderCredentials::Account { .. } => None,
        }
    }

    pub fn api_key(&self) -> Option<&str> {
        match &self.credentials {
            ProviderCredentials::ApiKey { api_key, .. } => Some(api_key),
            ProviderCredentials::Account { .. } => None,
        }
    }

    pub fn email(&self) -> Option<&str> {
        match &self.credentials {
            ProviderCredentials::ApiKey { .. } => None,
            ProviderCredentials::Account { email, .. } => email.as_deref(),
        }
    }

    pub fn expiry_timestamp(&self) -> Option<i64> {
        match &self.credentials {
            ProviderCredentials::ApiKey { .. } => None,
            ProviderCredentials::Account {
                expiry_timestamp, ..
            } => *expiry_timestamp,
        }
    }

    pub fn access_token(&self) -> Option<&str> {
        match &self.credentials {
            ProviderCredentials::ApiKey { .. } => None,
            ProviderCredentials::Account { access_token, .. } => access_token.as_deref(),
        }
    }

    pub fn refresh_token(&self) -> Option<&str> {
        match &self.credentials {
            ProviderCredentials::ApiKey { .. } => None,
            ProviderCredentials::Account { refresh_token, .. } => refresh_token.as_deref(),
        }
    }

    pub fn client_id(&self) -> Option<&str> {
        match &self.credentials {
            ProviderCredentials::ApiKey { .. } => None,
            ProviderCredentials::Account { client_id, .. } => client_id.as_deref(),
        }
    }

    pub fn set_expiry_timestamp(&mut self, expiry_timestamp: i64) {
        if let ProviderCredentials::Account {
            expiry_timestamp: current,
            ..
        } = &mut self.credentials
        {
            *current = Some(expiry_timestamp);
        }
    }

    pub fn set_access_token(&mut self, access_token: String) {
        if let ProviderCredentials::Account {
            access_token: current,
            ..
        } = &mut self.credentials
        {
            *current = Some(access_token);
        }
    }

    pub fn set_refresh_token(&mut self, refresh_token: String) {
        if let ProviderCredentials::Account {
            refresh_token: current,
            ..
        } = &mut self.credentials
        {
            *current = Some(refresh_token);
        }
    }

    pub fn new_openai_account(
        email: String,
        access_token: String,
        refresh_token: String,
        expiry_timestamp: i64,
        client_id: Option<String>,
        account_id: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: None,
            credentials: ProviderCredentials::Account {
                email: Some(email),
                access_token: Some(access_token),
                refresh_token: Some(refresh_token),
                expiry_timestamp: Some(expiry_timestamp),
                client_id,
                account_id,
            },
        }
    }
}
