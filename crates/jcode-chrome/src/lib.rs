//! Drive the locally installed Chrome over the DevTools Protocol.
//!
//! This is a leaf crate: it depends on no other `jcode-*` crate, so every path
//! and deadline arrives as a parameter. It exposes one primitive —
//! [`ChromeEngine::eval_on_page`] — which navigates a throwaway tab and
//! evaluates a script in it. Web search and page fetching are both expressed in
//! terms of that single operation.

mod cdp;
mod process;

use anyhow::{Context, Result, bail};
use cdp::CdpClient;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::Mutex;

pub struct ChromeOptions {
    /// Explicit binary; `None` auto-detects the installed Chrome.
    pub binary: Option<PathBuf>,
    /// Profile directory. Must be unique per daemon: Chrome aborts rather than
    /// share one, so a collision leaves the second daemon with no browser.
    pub user_data_dir: PathBuf,
    /// Bounds every wait this crate performs — port discovery, readiness
    /// polling, CDP requests and page loads. The caller owns the only deadline.
    pub startup_timeout: Duration,
}

/// Outcome of evaluating a script on a freshly navigated page.
pub struct PageEval {
    /// Status the page reported for its own navigation. `0` means the page
    /// exposed none, which is not the same as an error.
    pub http_status: u16,
    pub final_url: String,
    pub value: Value,
}

pub struct ChromeEngine {
    child: Mutex<Child>,
    client: Arc<CdpClient>,
    /// Chrome's own User-Agent with the headless marker removed.
    user_agent: String,
    timeout: Duration,
}

/// Strip the token that marks a browser as headless.
///
/// Chrome reports `…HeadlessChrome/153.0.0.0 Safari/537.36`, and that single
/// token is what search engines gate on: with it present every request is
/// served a CAPTCHA, with it replaced the same request returns results.
/// Deriving the string from Chrome's own output keeps it correct across
/// upgrades without hard-coding a version.
pub fn humanize_user_agent(reported: &str) -> String {
    reported.replace("HeadlessChrome/", "Chrome/")
}

impl ChromeEngine {
    pub async fn launch(options: ChromeOptions) -> Result<Self> {
        let binary = process::resolve_binary(options.binary.as_deref())?;
        std::fs::create_dir_all(&options.user_data_dir).with_context(|| {
            format!(
                "failed to create Chrome profile directory {}",
                options.user_data_dir.display()
            )
        })?;
        if let Some(parent) = options.user_data_dir.parent() {
            process::sweep_abandoned_profiles(parent, &options.user_data_dir);
        }

        let child = process::spawn(&binary, &options.user_data_dir)?;
        let port = process::await_debug_port(&options.user_data_dir, options.startup_timeout)
            .await
            .inspect_err(|_| {
                // The child is useless without a port; do not leave it running.
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                }
            })?;

        let version = await_version(port, options.startup_timeout).await?;
        let ws_url = version
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .context("Chrome /json/version did not report a webSocketDebuggerUrl")?
            .to_string();
        let user_agent = humanize_user_agent(
            version
                .get("User-Agent")
                .and_then(Value::as_str)
                .context("Chrome /json/version did not report a User-Agent")?,
        );

        let client = Arc::new(CdpClient::connect(&ws_url).await?);
        Ok(Self {
            child: Mutex::new(child),
            client,
            user_agent,
            timeout: options.startup_timeout,
        })
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Navigate a throwaway tab to `url`, wait for load, and evaluate `script`.
    ///
    /// `script` must evaluate to a Promise; it runs with `awaitPromise` and
    /// `returnByValue`. The load wait means the document is already complete
    /// when the script runs, so a script that waits on a `load` listener would
    /// never resolve — resolve on `document.readyState` instead.
    ///
    /// `wants_retry` exists because a search engine's first document can be a
    /// redirect stub: Startpage fires `loadEventFired` at ~200 ms on a page
    /// with no results, schedules a second navigation ~215-245 ms later, and
    /// only that second document carries the results. Measured on both cold
    /// and warmed profiles, so it is the engine's normal behaviour, not a
    /// cold-cache artifact. When the predicate says the evaluated value is
    /// unusable, this waits for the follow-up navigation to land and evaluates
    /// again. Pages that never navigate again return on the first evaluation,
    /// so a genuinely empty result set costs nothing extra.
    pub async fn eval_on_page(
        &self,
        url: &str,
        script: &str,
        timeout: Duration,
        wants_retry: &(dyn Fn(&Value) -> bool + Sync),
    ) -> Result<PageEval> {
        let target = self
            .client
            .send(
                None,
                "Target.createTarget",
                json!({ "url": "about:blank" }),
                self.timeout,
            )
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .context("Target.createTarget returned no targetId")?
            .to_string();

        let result = self
            .drive_target(&target_id, url, script, timeout, wants_retry)
            .await;

        // Always close the tab, including on the error path, or tabs leak for
        // the life of the daemon.
        let _ = self
            .client
            .send(
                None,
                "Target.closeTarget",
                json!({ "targetId": target_id }),
                self.timeout,
            )
            .await;
        result
    }

    async fn drive_target(
        &self,
        target_id: &str,
        url: &str,
        script: &str,
        timeout: Duration,
        wants_retry: &(dyn Fn(&Value) -> bool + Sync),
    ) -> Result<PageEval> {
        let attached = self
            .client
            .send(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                self.timeout,
            )
            .await?;
        let session = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .context("Target.attachToTarget returned no sessionId")?
            .to_string();

        let mut events = self.client.register_session(&session).await;
        let outcome = self
            .navigate_and_eval(&session, &mut events, url, script, timeout, wants_retry)
            .await;
        self.client.unregister_session(&session).await;
        outcome
    }

    async fn navigate_and_eval(
        &self,
        session: &str,
        events: &mut tokio::sync::mpsc::UnboundedReceiver<Value>,
        url: &str,
        script: &str,
        timeout: Duration,
        wants_retry: &(dyn Fn(&Value) -> bool + Sync),
    ) -> Result<PageEval> {
        self.client
            .send(Some(session), "Page.enable", json!({}), self.timeout)
            .await?;
        // Must precede navigation so the request itself carries the UA.
        self.client
            .send(
                Some(session),
                "Emulation.setUserAgentOverride",
                json!({ "userAgent": self.user_agent }),
                self.timeout,
            )
            .await?;
        self.client
            .send(
                Some(session),
                "Page.navigate",
                json!({ "url": url }),
                self.timeout,
            )
            .await?;
        // A timeout here is not an error: some pages never fire `load`, and the
        // script below is still worth running against whatever has rendered.
        CdpClient::wait_for_event(events, "Page.loadEventFired", timeout).await?;

        let deadline = tokio::time::Instant::now() + timeout;
        let mut value = self.evaluate(session, script, timeout).await?;

        // The first document can be a redirect stub carrying nothing. When the
        // caller says the value is unusable, wait for the page's own follow-up
        // navigation to finish and read the document it lands on. A page that
        // does not navigate again simply times out this wait and keeps the
        // first value.
        if wants_retry(&value) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if CdpClient::wait_for_event(events, "Page.loadEventFired", remaining)
                .await?
                .is_some()
            {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                value = self.evaluate(session, script, remaining).await?;
            }
        }

        Ok(PageEval {
            http_status: value.get("status").and_then(Value::as_u64).unwrap_or(0) as u16,
            final_url: value
                .get("finalUrl")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            value,
        })
    }

    async fn evaluate(&self, session: &str, script: &str, timeout: Duration) -> Result<Value> {
        let evaluated = self
            .client
            .send(
                Some(session),
                "Runtime.evaluate",
                json!({
                    "expression": script,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                timeout,
            )
            .await?;
        if let Some(details) = evaluated.get("exceptionDetails") {
            bail!("page script failed: {details}");
        }
        evaluated
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .context("Runtime.evaluate returned no value")
    }

    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}

impl Drop for ChromeEngine {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

async fn await_version(port: u16, timeout: Duration) -> Result<Value> {
    let endpoint = format!("http://127.0.0.1:{port}/json/version");
    let http = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_error = String::from("never responded");
    while tokio::time::Instant::now() < deadline {
        match http.get(&endpoint).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<Value>().await {
                    Ok(value) => return Ok(value),
                    Err(err) => last_error = err.to_string(),
                }
            }
            Ok(response) => last_error = format!("HTTP {}", response.status()),
            Err(err) => last_error = err.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("Chrome DevTools endpoint {endpoint} not ready: {last_error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headless marker is the single string whose corruption silently
    /// reintroduces CAPTCHAs, so pin it exactly.
    #[test]
    fn humanize_user_agent_removes_the_headless_marker() {
        let reported = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
             (KHTML, like Gecko) HeadlessChrome/153.0.0.0 Safari/537.36";

        let humanized = humanize_user_agent(reported);

        assert_eq!(
            humanized,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36"
        );
        assert!(!humanized.contains("Headless"));
    }

    /// A non-headless UA must survive untouched, so the same helper can run
    /// against any Chrome build without corrupting a already-clean string.
    #[test]
    fn humanize_user_agent_leaves_a_normal_agent_alone() {
        let reported = "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 Chrome/153.0.0.0 Safari/537.36";

        assert_eq!(humanize_user_agent(reported), reported);
    }
}
