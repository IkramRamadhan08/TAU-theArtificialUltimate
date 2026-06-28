use client::TAU_URL_SCHEME;
use gpui::{AsyncApp, actions};

actions!(
    cli,
    [
        /// Registers the tau:// URL scheme handler.
        RegisterTauScheme
    ]
);

pub async fn register_tau_scheme(cx: &AsyncApp) -> anyhow::Result<()> {
    cx.update(|cx| cx.register_url_scheme(TAU_URL_SCHEME)).await
}
