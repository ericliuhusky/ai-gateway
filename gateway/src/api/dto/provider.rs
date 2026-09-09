use crate::domain::{Provider, ProviderCredentials};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProviderReq {
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummaryResp {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    auth: ProviderSummaryAuth,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "auth_mode", rename_all = "snake_case")]
pub enum ProviderSummaryAuth {
    ApiKey {
        base_url: String,
    },
    Account {
        account_email: String,
        account_expires_at: i64,
    },
}

impl From<&Provider> for ProviderSummaryResp {
    fn from(provider: &Provider) -> Self {
        match &provider.credentials {
            ProviderCredentials::ApiKey { base_url, .. } => Self {
                id: provider.id().to_string(),
                name: provider.name().to_string(),
                auth: ProviderSummaryAuth::ApiKey {
                    base_url: base_url.clone(),
                },
            },
            ProviderCredentials::Account {
                email,
                expiry_timestamp,
                ..
            } => Self {
                id: provider.id().to_string(),
                name: provider.name().to_string(),
                auth: ProviderSummaryAuth::Account {
                    account_email: email.clone(),
                    account_expires_at: *expiry_timestamp,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderSummaryResp;
    use crate::domain::{Provider, ProviderCredentials};
    use serde_json::json;

    #[test]
    fn serializes_variant_specific_provider_fields() {
        let api_key_provider = Provider {
            id: "api-key-id".to_string(),
            name: Some("Official".to_string()),
            credentials: ProviderCredentials::ApiKey {
                base_url: "https://api.example.com/v1".to_string(),
                api_key: "secret".to_string(),
            },
        };
        let account_provider = Provider::new_openai_account(
            "user@example.com".to_string(),
            "access".to_string(),
            "refresh".to_string(),
            1_700_000_000,
            Some("client".to_string()),
            Some("account".to_string()),
        );

        assert_eq!(
            serde_json::to_value(ProviderSummaryResp::from(&api_key_provider)).unwrap(),
            json!({
                "auth_mode": "api_key",
                "id": "api-key-id",
                "name": "Official",
                "base_url": "https://api.example.com/v1",
            })
        );
        assert_eq!(
            serde_json::to_value(ProviderSummaryResp::from(&account_provider)).unwrap(),
            json!({
                "auth_mode": "account",
                "id": account_provider.id,
                "name": "user@example.com",
                "account_email": "user@example.com",
                "account_expires_at": 1_700_000_000,
            })
        );
    }
}
