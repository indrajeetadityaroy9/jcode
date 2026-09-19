//! Web search by driving real search engines in a headless Chrome.
//!
//! Every provider rate-limits, and they do it at different times: Startpage
//! suspended this network with an HTTP 200 redirect to `/sp/captcha-block`
//! while DuckDuckGo — 403 hours earlier — was serving normally, and the
//! previous SearXNG backend died because its single upstream ran out of quota.
//! A single engine is therefore a single point of failure, so this tries a
//! configured chain and returns the first engine that actually answers.

use super::SearchResult;
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

/// A search provider reachable through the browser.
struct Engine {
    id: &'static str,
    url: fn(&str) -> String,
    /// Tried in order; the first selector yielding off-host anchors wins.
    selectors: &'static [&'static str],
    /// Path prefix of a genuine results page. Leaving it means the provider
    /// bounced us to an interstitial.
    search_path: &'static str,
}

const ENGINES: &[Engine] = &[
    Engine {
        id: "startpage",
        url: |query| {
            format!(
                "https://www.startpage.com/sp/search?query={}",
                urlencoding::encode(query)
            )
        },
        selectors: &["a.result-link", "a.w-gl__result-title"],
        search_path: "/sp/search",
    },
    Engine {
        id: "duckduckgo",
        url: |query| format!("https://duckduckgo.com/?q={}", urlencoding::encode(query)),
        selectors: &["a[data-testid='result-title-a']"],
        search_path: "/",
    },
];

/// What one engine did with the query.
#[derive(Debug, PartialEq)]
pub(super) enum EngineOutcome {
    /// The engine answered with usable results.
    Results(Vec<SearchResult>),
    /// The engine answered, and the query genuinely matched nothing.
    Empty,
    /// The engine refused to answer: rate limited, challenged, or bounced to
    /// an interstitial. Reformulating the query cannot help.
    Blocked,
}

/// What the in-page extractor resolves to.
#[derive(Deserialize)]
struct PageReport {
    #[serde(default)]
    status: u16,
    /// True when the provider redirected away from its own results path, which
    /// is how a suspension looks when it still returns HTTP 200.
    #[serde(default)]
    off_search_path: bool,
    #[serde(default)]
    results: Vec<PageResult>,
}

#[derive(Deserialize)]
struct PageResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    snippet: String,
}

/// Extract results in the page.
///
/// Runs in a document that `eval_on_page` has already waited for, so the
/// `readyState` check is what keeps the promise from waiting on a `load` event
/// that has already fired.
fn extractor(engine: &Engine) -> String {
    let selectors = serde_json::to_string(engine.selectors).unwrap_or_else(|_| "[]".to_string());
    let search_path =
        serde_json::to_string(engine.search_path).unwrap_or_else(|_| "\"/\"".to_string());
    format!(
        r#"(() => new Promise((resolve) => {{
  const SELECTORS = {selectors};
  const SEARCH_PATH = {search_path};
  const engineHost = location.host;

  const collect = () => {{
    for (const sel of SELECTORS) {{
      const found = [...document.querySelectorAll(sel)].filter((a) => {{
        if (!a.href || !/^https?:/.test(a.href)) return false;
        try {{ return new URL(a.href).host !== engineHost; }} catch {{ return false; }}
      }});
      if (found.length) return found;
    }}
    return [];
  }};

  const snippetFor = (anchor, anchors) => {{
    const owns = (n) => anchors.filter((a) => n.contains(a)).length;
    let c = anchor;
    while (c.parentElement && owns(c.parentElement) === 1) c = c.parentElement;
    const clone = c.cloneNode(true);
    clone.querySelectorAll("a").forEach((n) => n.replaceWith(document.createTextNode("\n")));
    const lines = (clone.innerText || "")
      .split("\n")
      .map((s) => s.replace(/\s+/g, " ").trim())
      .filter(Boolean);
    return lines.reduce((best, l) => (l.length > best.length ? l : best), "");
  }};

  const finish = () => {{
    const nav = performance.getEntriesByType("navigation")[0];
    const anchors = collect();
    const seen = new Set();
    const results = [];
    for (const a of anchors) {{
      if (seen.has(a.href)) continue;
      seen.add(a.href);
      results.push({{
        title: (a.innerText || "").trim() || a.href,
        url: a.href,
        snippet: snippetFor(a, anchors),
      }});
    }}
    resolve({{
      status: (nav && nav.responseStatus) || 0,
      off_search_path: !location.pathname.startsWith(SEARCH_PATH),
      finalUrl: location.href,
      results,
    }});
  }};

  if (collect().length) return finish();
  if (document.readyState === "complete") return finish();
  const obs = new MutationObserver(() => {{
    if (collect().length) {{ obs.disconnect(); finish(); }}
  }});
  obs.observe(document.documentElement, {{ childList: true, subtree: true }});
  window.addEventListener("load", () => {{ obs.disconnect(); finish(); }}, {{ once: true }});
}}))()"#
    )
}

/// Decide what one engine's page means.
///
/// `off_search_path` is decisive rather than the status code: a Startpage
/// suspension redirects to `/sp/captcha-block` and still returns HTTP 200, so
/// a status-only rule would read it as an ordinary empty result set. A status
/// of `0` means the page exposed none, which is not an error.
fn classify(report: &Value, num_results: usize) -> Result<EngineOutcome> {
    let report: PageReport = serde_json::from_value(report.clone())?;
    if report.status >= 400 || report.off_search_path {
        return Ok(EngineOutcome::Blocked);
    }
    let results: Vec<SearchResult> = report
        .results
        .into_iter()
        .filter(|result| !result.url.trim().is_empty())
        .take(num_results)
        .map(|result| SearchResult {
            title: if result.title.trim().is_empty() {
                result.url.clone()
            } else {
                result.title
            },
            url: result.url,
            snippet: result.snippet,
        })
        .collect();
    if results.is_empty() {
        return Ok(EngineOutcome::Empty);
    }
    Ok(EngineOutcome::Results(results))
}

/// Turn the chain's per-engine outcomes into the caller's error.
///
/// Distinguishing "nothing matched" from "every provider refused" is the whole
/// point: the old backend reported a dead upstream as "no results", which told
/// the model its query was wrong and drove it into futile rewrites.
fn chain_failure(query: &str, tried: &[(&str, EngineOutcome)]) -> anyhow::Error {
    if tried
        .iter()
        .any(|(_, outcome)| matches!(outcome, EngineOutcome::Empty))
    {
        return anyhow::anyhow!("No web results for '{query}'.");
    }
    let ids = tried
        .iter()
        .map(|(id, _)| *id)
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::anyhow!(
        "web search backends unavailable: {ids} all blocked or rate-limited. \
         Do not reformulate the query; retry later or fetch a known URL with webfetch."
    )
}

fn configured_engines() -> Result<Vec<&'static Engine>> {
    let ids = &crate::config::config().websearch.engines;
    if ids.is_empty() {
        bail!(
            "[websearch] engines is empty; list at least one of: {}",
            valid_ids()
        );
    }
    ids.iter()
        .map(|id| {
            ENGINES
                .iter()
                .find(|engine| engine.id == id.trim())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "[websearch] unknown engine '{id}'; valid ids: {}",
                        valid_ids()
                    )
                })
        })
        .collect()
}

fn valid_ids() -> String {
    ENGINES
        .iter()
        .map(|engine| engine.id)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) async fn search(query: &str, num_results: usize) -> Result<Vec<SearchResult>> {
    let engines = configured_engines()?;
    let chrome = super::super::web::chrome_engine().await?;
    let timeout = Duration::from_secs(super::super::web::DEFAULT_TIMEOUT);

    let mut tried: Vec<(&str, EngineOutcome)> = Vec::with_capacity(engines.len());
    for engine in engines {
        let page = chrome
            .eval_on_page(
                &(engine.url)(query),
                &extractor(engine),
                timeout,
                // A page that yielded nothing may still be the engine's
                // redirect stub rather than an empty result set, so let the
                // follow-up navigation land before believing it.
                &|value: &serde_json::Value| {
                    value
                        .get("results")
                        .and_then(serde_json::Value::as_array)
                        .is_none_or(|results| results.is_empty())
                },
            )
            .await?;
        match classify(&page.value, num_results)? {
            EngineOutcome::Results(results) => {
                crate::logging::info(&format!(
                    "websearch: {} result(s) for '{query}' via {}",
                    results.len(),
                    engine.id
                ));
                return Ok(results);
            }
            outcome => {
                crate::logging::info(&format!(
                    "websearch: {} answered {outcome:?} for '{query}'; trying the next engine",
                    engine.id
                ));
                tried.push((engine.id, outcome));
            }
        }
    }
    Err(chain_failure(query, &tried))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The measured Startpage suspension: redirected to `/sp/captcha-block`
    /// and still HTTP 200. A status-only rule would call this an empty result
    /// set and tell the model to rewrite its query.
    #[test]
    fn a_two_hundred_redirect_off_the_search_path_is_blocked() {
        let report = json!({"status": 200, "off_search_path": true, "results": []});

        assert_eq!(classify(&report, 8).unwrap(), EngineOutcome::Blocked);
    }

    #[test]
    fn an_error_status_is_blocked() {
        let report = json!({"status": 429, "off_search_path": false, "results": []});

        assert_eq!(classify(&report, 8).unwrap(), EngineOutcome::Blocked);
    }

    /// Some pages expose no navigation status at all; that is missing
    /// information, not a refusal.
    #[test]
    fn a_missing_status_on_a_results_page_is_not_blocked() {
        let report = json!({"status": 0, "off_search_path": false, "results": []});

        assert_eq!(classify(&report, 8).unwrap(), EngineOutcome::Empty);
    }

    #[test]
    fn results_are_mapped_and_capped() {
        let report = json!({
            "status": 200,
            "off_search_path": false,
            "results": [
                {"title": "Templating - chezmoi", "url": "https://www.chezmoi.io/user-guide/templating/", "snippet": "s"},
                {"title": "Variables", "url": "https://www.chezmoi.io/reference/", "snippet": "t"},
            ],
        });

        let EngineOutcome::Results(results) = classify(&report, 1).unwrap() else {
            panic!("expected results");
        };
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].url,
            "https://www.chezmoi.io/user-guide/templating/"
        );
        assert_eq!(results[0].snippet, "s");
    }

    /// A hit with no URL is not a usable result, and a page of only those is
    /// silence rather than an answer.
    #[test]
    fn entries_without_urls_do_not_count_as_results() {
        let report = json!({
            "status": 200,
            "off_search_path": false,
            "results": [{"title": "No link", "url": "  ", "snippet": "x"}],
        });

        assert_eq!(classify(&report, 8).unwrap(), EngineOutcome::Empty);
    }

    /// Every provider refused, so the query was never actually run. Telling
    /// the model there were "no results" here is what caused futile rewrites.
    #[test]
    fn all_engines_blocked_reports_the_backends_not_the_query() {
        let tried = vec![
            ("startpage", EngineOutcome::Blocked),
            ("duckduckgo", EngineOutcome::Blocked),
        ];

        let message = chain_failure("q", &tried).to_string();

        assert!(message.contains("backends unavailable"), "{message}");
        assert!(message.contains("startpage, duckduckgo"), "{message}");
        assert!(!message.contains("No web results"), "{message}");
    }

    /// One engine was blocked but another genuinely answered nothing, so the
    /// query really did match nothing and rewriting it is the right move.
    #[test]
    fn one_empty_engine_means_the_query_found_nothing() {
        let tried = vec![
            ("startpage", EngineOutcome::Blocked),
            ("duckduckgo", EngineOutcome::Empty),
        ];

        let message = chain_failure("q", &tried).to_string();

        assert!(message.contains("No web results for 'q'"), "{message}");
        assert!(!message.contains("unavailable"), "{message}");
    }
}
