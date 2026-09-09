use reqwest::Client;

use crate::scproxy::SysConfigProxy;

pub fn build_http_client() -> Client {
    let builder = SysConfigProxy::apply(Client::builder());
    builder.build().unwrap_or_else(|_| Client::new())
}
