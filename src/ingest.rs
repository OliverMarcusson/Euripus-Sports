use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{anyhow, bail, Context};

use crate::config::SourceRequestMethod;
use async_trait::async_trait;
use clap::ValueEnum;
use serde::Deserialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const HTTP_BODY_LIMIT: usize = 8 * 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
struct FetchLimits {
    http_connect_timeout: Duration,
    http_request_timeout: Duration,
    browser_phase_timeout: Duration,
    body_limit: usize,
    diagnostic_limit: usize,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            http_connect_timeout: Duration::from_secs(10),
            http_request_timeout: Duration::from_secs(45),
            browser_phase_timeout: Duration::from_secs(30),
            body_limit: HTTP_BODY_LIMIT,
            diagnostic_limit: DIAGNOSTIC_LIMIT,
        }
    }
}

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
    limits: FetchLimits,
}

impl BrowserFallbackFetcher {
    pub fn new(browser_command: impl Into<String>) -> anyhow::Result<Self> {
        Self::with_limits(browser_command, FetchLimits::default())
    }

    fn with_limits(
        browser_command: impl Into<String>,
        limits: FetchLimits,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("sports-api/0.1 (+https://euripus.example)")
            .connect_timeout(limits.http_connect_timeout)
            .timeout(limits.http_request_timeout)
            .build()
            .context("building http client")?;

        Ok(Self {
            client,
            browser_command: browser_command.into(),
            session_name: "sports-api-ingest".into(),
            limits,
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
        let mut response = builder
            .send()
            .await
            .with_context(|| format!("http fetch failed for {}", request.url))?;
        let status = response.status();
        if !status.is_success() {
            bail!("http fetch returned status {} for {}", status, request.url);
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.limits.body_limit as u64)
        {
            bail!(
                "http body for {} exceeds {} bytes",
                request.url,
                self.limits.body_limit
            );
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .with_context(|| format!("reading http body from {}", request.url))?
        {
            let next_length = bytes
                .len()
                .checked_add(chunk.len())
                .context("http body length overflow")?;
            if next_length > self.limits.body_limit {
                bail!(
                    "http body for {} exceeds {} bytes",
                    request.url,
                    self.limits.body_limit
                );
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(bytes).context("http body was not utf-8")?;

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
        let mut open = Command::new(&self.browser_command);
        open.args(["--session-name", &self.session_name, "open", &request.url]);
        let output = run_bounded_command(
            open,
            "browser open",
            self.limits.browser_phase_timeout,
            None,
            self.limits.diagnostic_limit,
        )
        .await
        .with_context(|| format!("running {} open", self.browser_command))?;
        ensure_command_succeeded("browser open", &output)?;

        let mut wait = Command::new(&self.browser_command);
        wait.args([
            "--session-name",
            &self.session_name,
            "wait",
            "--load",
            "networkidle",
        ]);
        let output = run_bounded_command(
            wait,
            "browser wait",
            self.limits.browser_phase_timeout,
            None,
            self.limits.diagnostic_limit,
        )
        .await
        .with_context(|| format!("running {} wait", self.browser_command))?;
        ensure_command_succeeded("browser wait", &output)?;

        let mut eval = Command::new(&self.browser_command);
        eval.args(["--session-name", &self.session_name, "eval", js]);
        let output = run_bounded_command(
            eval,
            "browser eval",
            self.limits.browser_phase_timeout,
            Some(self.limits.body_limit),
            self.limits.diagnostic_limit,
        )
        .await
        .with_context(|| format!("running {} eval", self.browser_command))?;
        ensure_command_succeeded("browser eval", &output)?;

        let body = String::from_utf8(output.stdout).context("browser output was not utf-8")?;
        build_browser_page(request, body)
    }

    async fn fetch_browser_with_chromium(
        &self,
        request: &FetchRequest,
    ) -> anyhow::Result<FetchedPage> {
        let mut command = Command::new(&self.browser_command);
        command
            .args([
                "--headless=new",
                "--disable-gpu",
                "--user-data-dir=/tmp/chromium-sports-api",
                "--virtual-time-budget=15000",
                "--dump-dom",
                &request.url,
            ])
            .env("TZ", "Europe/Stockholm");
        let output = run_bounded_command(
            command,
            "chromium browser fetch",
            self.limits.browser_phase_timeout,
            Some(self.limits.body_limit),
            self.limits.diagnostic_limit,
        )
        .await
        .with_context(|| format!("running {} --dump-dom", self.browser_command))?;
        ensure_command_succeeded("chromium browser fetch", &output)?;

        let body = String::from_utf8(output.stdout).context("browser output was not utf-8")?;
        build_browser_page(request, body)
    }
}

#[derive(Debug)]
struct BoundedOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn run_bounded_command(
    mut command: Command,
    phase: &str,
    phase_timeout: Duration,
    stdout_limit: Option<usize>,
    stderr_limit: usize,
) -> anyhow::Result<BoundedOutput> {
    if stdout_limit.is_some() {
        command.stdout(Stdio::piped());
    } else {
        command.stdout(Stdio::null());
    }
    command.stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawning {phase}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take().context("capturing command stderr")?;

    let execution = async {
        let stdout_future = async {
            match (stdout, stdout_limit) {
                (Some(stream), Some(limit)) => read_limited(stream, limit, phase, "stdout").await,
                _ => Ok(Vec::new()),
            }
        };
        let stderr_future = read_limited(stderr, stderr_limit, phase, "stderr");
        let wait_future = async { child.wait().await.context("waiting for child process") };
        let (stdout, stderr, status) = tokio::try_join!(stdout_future, stderr_future, wait_future)?;
        Ok::<_, anyhow::Error>(BoundedOutput {
            status,
            stdout,
            stderr,
        })
    };

    match tokio::time::timeout(phase_timeout, execution).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => {
            terminate_child(&mut child).await;
            Err(error)
        }
        Err(_) => {
            terminate_child(&mut child).await;
            bail!(
                "{phase} timed out after {} seconds",
                phase_timeout.as_secs_f64()
            )
        }
    }
}

async fn read_limited(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
    phase: &str,
    stream_name: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .with_context(|| format!("reading {phase} {stream_name}"))?;
        if count == 0 {
            return Ok(output);
        }
        let next_length = output
            .len()
            .checked_add(count)
            .context("command output length overflow")?;
        if next_length > limit {
            bail!("{phase} {stream_name} exceeded {limit} bytes");
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

async fn terminate_child(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn ensure_command_succeeded(phase: &str, output: &BoundedOutput) -> anyhow::Result<()> {
    if !output.status.success() {
        bail!(
            "{phase} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn serve_once(response: Vec<u8>, delay: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            tokio::time::sleep(delay).await;
            stream.write_all(&response).await.unwrap();
        });
        format!("http://{address}/")
    }

    fn request(url: String) -> FetchRequest {
        FetchRequest {
            source_name: "test".into(),
            url,
            method: SourceRequestMethod::Get,
            body: None,
            mode: SourceFetchMode::Http,
        }
    }

    fn test_fetcher(body_limit: usize, timeout: Duration) -> BrowserFallbackFetcher {
        BrowserFallbackFetcher::with_limits(
            "agent-browser",
            FetchLimits {
                http_connect_timeout: timeout,
                http_request_timeout: timeout,
                browser_phase_timeout: timeout,
                body_limit,
                diagnostic_limit: 64,
            },
        )
        .unwrap()
    }

    #[test]
    fn detects_cloudflare_block_page() {
        assert!(looks_like_cloudflare_block(
            "<title>Attention Required! | Cloudflare</title>"
        ));
        assert!(!looks_like_cloudflare_block("<html><body>ok</body></html>"));
    }

    #[tokio::test]
    async fn http_body_at_limit_succeeds() {
        let body = b"12345678";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        )
        .into_bytes();
        let url = serve_once(response, Duration::ZERO).await;
        let page = test_fetcher(body.len(), Duration::from_secs(1))
            .fetch_http(&request(url))
            .await
            .unwrap();
        assert_eq!(page.body.as_bytes(), body);
    }

    #[tokio::test]
    async fn http_rejects_declared_and_streamed_oversize_bodies() {
        for headers in [
            "Content-Length: 9\r\n",
            "Transfer-Encoding: chunked\r\n",
            "",
        ] {
            let payload = if headers.contains("chunked") {
                "9\r\n123456789\r\n0\r\n\r\n".to_string()
            } else {
                "123456789".to_string()
            };
            let response =
                format!("HTTP/1.1 200 OK\r\n{headers}Connection: close\r\n\r\n{payload}")
                    .into_bytes();
            let url = serve_once(response, Duration::ZERO).await;
            let error = test_fetcher(8, Duration::from_secs(1))
                .fetch_http(&request(url))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("exceeds 8 bytes"), "{error:#}");
        }
    }

    #[tokio::test]
    async fn http_rejects_invalid_utf8_and_times_out() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n\xff".to_vec();
        let url = serve_once(response, Duration::ZERO).await;
        let error = test_fetcher(8, Duration::from_secs(1))
            .fetch_http(&request(url))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not utf-8"));

        let response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec();
        let url = serve_once(response, Duration::from_millis(200)).await;
        let error = test_fetcher(8, Duration::from_millis(30))
            .fetch_http(&request(url))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("http fetch failed"));
    }

    #[tokio::test]
    async fn bounded_command_limits_time_stdout_and_stderr() {
        let started = tokio::time::Instant::now();
        let mut sleeping = Command::new("sh");
        sleeping.args(["-c", "exec sleep 10"]);
        let error = run_bounded_command(
            sleeping,
            "sleep test",
            Duration::from_millis(40),
            Some(8),
            8,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));

        let mut stdout = Command::new("sh");
        stdout.args(["-c", "printf 123456789"]);
        let error = run_bounded_command(stdout, "stdout test", Duration::from_secs(1), Some(8), 8)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stdout exceeded 8 bytes"));

        let mut stderr = Command::new("sh");
        stderr.args(["-c", "printf 123456789 >&2"]);
        let error = run_bounded_command(stderr, "stderr test", Duration::from_secs(1), Some(8), 8)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stderr exceeded 8 bytes"));
    }

    #[tokio::test]
    async fn bounded_command_returns_small_dom() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf '<html>ok</html>'"]);
        let output = run_bounded_command(
            command,
            "dom test",
            Duration::from_secs(1),
            Some(HTTP_BODY_LIMIT),
            DIAGNOSTIC_LIMIT,
        )
        .await
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"<html>ok</html>");
    }
}
