use crate::upstream::shared::truncate_for_log;
use reqwest::Client;
use serde_json::Value;
use tracing::info;

pub const XCODE_BEST_SITE_URL: &str = "https://xcode.best";

#[derive(Clone, Debug)]
pub struct NewApiSiteClient {
    http: Client,
}

impl NewApiSiteClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    pub async fn fetch_user_self(
        &self,
        request_id: &str,
        token: &str,
        user_id: &str,
    ) -> Result<Value, String> {
        let url = format!("{XCODE_BEST_SITE_URL}/api/user/self");
        info!(request_id = %request_id, url = %url, user_id = %user_id, "sending new-api user self request");
        self.get_with_user_header(&url, token, user_id, "new-api user self")
            .await
    }

    pub async fn fetch_subscription_self(
        &self,
        request_id: &str,
        token: &str,
        user_id: &str,
    ) -> Result<Value, String> {
        let url = format!("{XCODE_BEST_SITE_URL}/api/subscription/self");
        info!(request_id = %request_id, url = %url, user_id = %user_id, "sending new-api subscription self request");
        self.get_with_user_header(&url, token, user_id, "new-api subscription self")
            .await
    }

    async fn get_with_user_header(
        &self,
        url: &str,
        token: &str,
        user_id: &str,
        label: &str,
    ) -> Result<Value, String> {
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .header("New-Api-User", user_id)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|err| format!("{label} request failed: {err}"))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|err| format!("{label} parse failed: {err}"))
        } else {
            let status = response.status();
            let response_body = response.text().await.unwrap_or_default();
            Err(format!(
                "{label} upstream returned {status}: {}",
                truncate_for_log(&response_body, 1_000)
            ))
        }
    }
}
