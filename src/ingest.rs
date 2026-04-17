use std::{path::Path, process::Stdio};

use anyhow::{anyhow, bail, Context};

use crate::config::SourceRequestMethod;
use async_trait::async_trait;
use clap::ValueEnum;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, ValueEnum, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFetchMode {
    Fixture,
    Http,
    Browser,
    Auto,
}

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub source_name: String,
    pub url: String,
    pub method: SourceRequestMethod,
    pub body: Option<String>,
    pub mode: SourceFetchMode,
}

#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub source_name: String,
    pub url: String,
    pub body: String,
    pub method: FetchMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMethod {
    Http,
    Browser,
}

#[async_trait]
pub trait SourceFetcher: Send + Sync {
    async fn fetch(&self, request: &FetchRequest) -> anyhow::Result<FetchedPage>;
}

#[derive(Debug, Clone)]
pub struct BrowserFallbackFetcher {
    client: reqwest::Client,
    browser_command: String,
    session_name: String,
}

impl BrowserFallbackFetcher {
    pub fn new(browser_command: impl Into<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("sports-api/0.1 (+https://euripus.example)")
            .build()
            .context("building http client")?;

        Ok(Self {
            client,
            browser_command: browser_command.into(),
            session_name: "sports-api-ingest".into(),
        })
    }

    async fn fetch_http(&self, request: &FetchRequest) -> anyhow::Result<FetchedPage> {
        let builder = match request.method {
            SourceRequestMethod::Get => self.client.get(&request.url),
            SourceRequestMethod::Post => self.client.post(&request.url),
        };
        let builder = if matches!(request.method, SourceRequestMethod::Post) {
            builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request.body.clone().unwrap_or_default())
        } else {
            builder
        };
        let response = builder
            .send()
            .await
            .with_context(|| format!("http fetch failed for {}", request.url))?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("http fetch returned status {} for {}", status, request.url);
        }
        if looks_like_cloudflare_block(&body) {
            bail!("cloudflare block detected for {}", request.url);
        }

        Ok(FetchedPage {
            source_name: request.source_name.clone(),
            url: request.url.clone(),
            body,
            method: FetchMethod::Http,
        })
    }

    async fn fetch_browser(&self, request: &FetchRequest) -> anyhow::Result<FetchedPage> {
        if matches!(request.method, SourceRequestMethod::Post) {
            bail!(
                "browser fetch does not support POST requests for {}",
                request.url
            );
        }

        if is_chromium_like(&self.browser_command) {
            return self.fetch_browser_with_chromium(request).await;
        }

        self.fetch_browser_with_agent_browser(request).await
    }

    async fn fetch_browser_with_agent_browser(
        &self,
        request: &FetchRequest,
    ) -> anyhow::Result<FetchedPage> {
        let js = r#"(() => document.documentElement.outerHTML)()"#;
        let output = Command::new(&self.browser_command)
            .args(["--session-name", &self.session_name, "open", &request.url])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("running {} open", self.browser_command))?;
        if !output.status.success() {
            bail!(
                "browser open failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let wait = Command::new(&self.browser_command)
            .args([
                "--session-name",
                &self.session_name,
                "wait",
                "--load",
                "networkidle",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("running {} wait", self.browser_command))?;
        if !wait.status.success() {
            bail!(
                "browser wait failed: {}",
                String::from_utf8_lossy(&wait.stderr)
            );
        }

        let eval = Command::new(&self.browser_command)
            .args(["--session-name", &self.session_name, "eval", js])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("running {} eval", self.browser_command))?;
        if !eval.status.success() {
            bail!(
                "browser eval failed: {}",
                String::from_utf8_lossy(&eval.stderr)
            );
        }

        let body = String::from_utf8(eval.stdout).context("browser output was not utf-8")?;
        build_browser_page(request, body)
    }

    async fn fetch_browser_with_chromium(
        &self,
        request: &FetchRequest,
    ) -> anyhow::Result<FetchedPage> {
        let output = Command::new(&self.browser_command)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-sandbox",
                "--disable-dev-shm-usage",
                "--virtual-time-budget=15000",
                "--dump-dom",
                &request.url,
            ])
            .env("TZ", "Europe/Stockholm")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("running {} --dump-dom", self.browser_command))?;
        if !output.status.success() {
            bail!(
                "chromium browser fetch failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let body = String::from_utf8(output.stdout).context("browser output was not utf-8")?;
        build_browser_page(request, body)
    }
}

fn build_browser_page(request: &FetchRequest, body: String) -> anyhow::Result<FetchedPage> {
    if body.trim().is_empty() {
        return Err(anyhow!(
            "browser fetch returned empty document for {}",
            request.url
        ));
    }

    Ok(FetchedPage {
        source_name: request.source_name.clone(),
        url: request.url.clone(),
        body,
        method: FetchMethod::Browser,
    })
}

fn is_chromium_like(command: &str) -> bool {
    let name = Path::new(command)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(command)
        .to_ascii_lowercase();
    name.contains("chromium") || name.contains("chrome")
}

#[async_trait]
impl SourceFetcher for BrowserFallbackFetcher {
    async fn fetch(&self, request: &FetchRequest) -> anyhow::Result<FetchedPage> {
        match request.mode {
            SourceFetchMode::Fixture => bail!("fixture mode does not fetch network content"),
            SourceFetchMode::Http => self.fetch_http(request).await,
            SourceFetchMode::Browser => self.fetch_browser(request).await,
            SourceFetchMode::Auto => match self.fetch_http(request).await {
                Ok(page) => Ok(page),
                Err(error) => {
                    tracing::warn!(source = request.source_name, url = request.url, error = %error, "http fetch failed, falling back to browser");
                    self.fetch_browser(request).await
                }
            },
        }
    }
}

fn looks_like_cloudflare_block(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("attention required") && body.contains("cloudflare")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cloudflare_block_page() {
        assert!(looks_like_cloudflare_block(
            "<title>Attention Required! | Cloudflare</title>"
        ));
        assert!(!looks_like_cloudflare_block("<html><body>ok</body></html>"));
    }
}
