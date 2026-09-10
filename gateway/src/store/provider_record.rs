use crate::domain::{Provider, ProviderAuthMode, ProviderCredentials};

#[derive(Debug, Clone)]
pub struct ProviderRecord {
    pub id: String,
    pub name: String,
    pub auth_mode: ProviderAuthMode,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expiry_timestamp: Option<i64>,
    pub client_id: Option<String>,
}

impl TryFrom<ProviderRecord> for Provider {
    type Error = String;

    fn try_from(record: ProviderRecord) -> Result<Self, Self::Error> {
        let credentials = match record.auth_mode {
            ProviderAuthMode::ApiKey => ProviderCredentials::ApiKey {
                base_url: record
                    .base_url
                    .ok_or_else(|| "API Key 供应商缺少 base_url".to_string())?,
                api_key: record
                    .api_key
                    .ok_or_else(|| "API Key 供应商缺少 api_key".to_string())?,
            },
            ProviderAuthMode::Account => ProviderCredentials::Account {
                access_token: record
                    .access_token
                    .ok_or_else(|| "账户供应商缺少 access_token".to_string())?,
                refresh_token: record
                    .refresh_token
                    .ok_or_else(|| "账户供应商缺少 refresh_token".to_string())?,
                expiry_timestamp: record
                    .expiry_timestamp
                    .ok_or_else(|| "账户供应商缺少 expiry_timestamp".to_string())?,
                client_id: record
                    .client_id
                    .ok_or_else(|| "账户供应商缺少 client_id".to_string())?,
            },
        };
        Ok(Provider {
            id: record.id,
            name: record.name,
            credentials,
        })
    }
}

impl From<&Provider> for ProviderRecord {
    fn from(provider: &Provider) -> Self {
        match &provider.credentials {
            ProviderCredentials::ApiKey { base_url, api_key } => Self {
                id: provider.id.clone(),
                name: provider.name.clone(),
                auth_mode: ProviderAuthMode::ApiKey,
                base_url: Some(base_url.clone()),
                api_key: Some(api_key.clone()),
                access_token: None,
                refresh_token: None,
                expiry_timestamp: None,
                client_id: None,
            },
            ProviderCredentials::Account {
                access_token,
                refresh_token,
                expiry_timestamp,
                client_id,
            } => Self {
                id: provider.id.clone(),
                name: provider.name.clone(),
                auth_mode: ProviderAuthMode::Account,
                base_url: None,
                api_key: None,
                access_token: Some(access_token.clone()),
                refresh_token: Some(refresh_token.clone()),
                expiry_timestamp: Some(*expiry_timestamp),
                client_id: Some(client_id.clone()),
            },
        }
    }
}
