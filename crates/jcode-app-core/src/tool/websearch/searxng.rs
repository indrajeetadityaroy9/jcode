//! Web search through a self-hosted SearXNG instance's JSON API.
//!
//! SearXNG is a metasearch front end: it queries the upstream engines itself
//! and returns their aggregated results, so one request is the whole search.
//! This backend is opt-in (`[websearch] backend = "searxng"`) and is never
//! reached by failover — an instance that is down or out of quota should
//! surface as an error rather than silently divert traffic to a browser.

use super::SearchResult;
use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

/// The subset of SearXNG's JSON response this backend reads.
#[derive(Deserialize)]
pub(super) struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
    /// Upstream engines that failed, timed out, served a CAPTCHA or were rate
    /// limited, as `["engine", "reason"]` pairs. Diagnostic only: individual
    /// upstream failure is normal for a metasearch engine and never makes a
    /// search unsuccessful on its own.
    #[serde(default)]
    unresponsive_engines: Vec<Value>,
}

#[derive(Deserialize)]
pub(super) struct SearxngResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: Option<String>,
    /// Which upstream engine produced this result.
    #[serde(default)]
    engine: Option<String>,
}

/// Query the configured instance and map its response onto [`SearchResult`].
pub(super) async fn search(
    client: &reqwest::Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    let endpoint = format!(
        "{}/search",
        crate::config::config().websearch.url.trim_end_matches('/')
    );

    let response = client
        .get(&endpoint)
        .query(&[("q", query), ("format", "json")])
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "SearXNG search failed with status {} (endpoint: {endpoint}).",
            response.status()
        ));
    }

    let parsed: SearxngResponse = response.json().await.map_err(|err| {
        anyhow::anyhow!(
            "SearXNG returned a non-JSON response ({err}). The instance may have \
             the JSON format disabled; enable `formats: [html, json]` in its settings."
        )
    })?;

    accept_response(query, parsed, num_results)
}

/// Apply the success rule: the request succeeded *and* the response carries at
/// least one usable result. A non-empty `unresponsive_engines` is recorded as a
/// diagnostic and never fails the search on its own - failing on it would
/// discard good results whenever any single upstream engine hit a CAPTCHA.
///
/// The emptiness check runs on the mapped results rather than the raw list, so
/// a response whose every entry lacks a URL fails here instead of reporting
/// success with nothing to show.
fn accept_response(
    query: &str,
    response: SearxngResponse,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    let unresponsive = describe_engine_entries(&response.unresponsive_engines);
    let contributing = contributing_engines(&response.results);
    let results = usable_results(response.results, num_results);

    if results.is_empty() {
        return Err(anyhow::anyhow!(
            "SearXNG returned no results for '{query}'; unresponsive engines: {unresponsive}"
        ));
    }

    crate::logging::info(&format!(
        "websearch: {} result(s) for '{query}' from [{contributing}]; \
         unresponsive engines: {unresponsive}",
        results.len()
    ));

    Ok(results)
}

/// Drop entries with no URL and cap to `num_results`.
///
/// An entry with an empty `title` falls back to its URL: some engines return
/// untitled hits, and rendering them as `**` would leave the model a bullet it
/// cannot identify even though the link is good.
fn usable_results(results: Vec<SearxngResult>, num_results: usize) -> Vec<SearchResult> {
    results
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
            snippet: result.content.unwrap_or_default(),
        })
        .collect()
}

/// The engines that produced at least one result, in first-seen order.
///
/// SearXNG's top-level `engines` field is `null` on some instances, so this
/// derives the same information - engines that contributed a returned result -
/// from the results themselves, which always carry their origin.
fn contributing_engines(results: &[SearxngResult]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for engine in results.iter().filter_map(|result| result.engine.as_deref()) {
        let engine = engine.trim();
        if !engine.is_empty() && !seen.contains(&engine) {
            seen.push(engine);
        }
    }
    if seen.is_empty() {
        "unknown".to_string()
    } else {
        seen.join(", ")
    }
}

/// Render `unresponsive_engines` for diagnostics. Entries arrive as
/// `["engine", "reason"]` pairs; anything else is rendered verbatim rather than
/// dropped, so an unexpected shape still reaches the log.
fn describe_engine_entries(entries: &[Value]) -> String {
    if entries.is_empty() {
        return "none".to_string();
    }
    entries
        .iter()
        .map(|entry| match entry.as_array() {
            Some(fields) => fields.iter().map(value_text).collect::<Vec<_>>().join(": "),
            None => value_text(entry),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(body: &str) -> SearxngResponse {
        serde_json::from_str(body).expect("searxng response")
    }

    #[test]
    fn maps_titles_urls_and_snippets() {
        let response = parse(
            r#"{"results":[
                {"title":"Rust","url":"https://rust-lang.org","content":"Systems language","engine":"brave"},
                {"title":"Docs","url":"https://docs.rs","content":null,"engine":"google cse"}
            ]}"#,
        );

        let results = accept_response("rust", response, 8).expect("results");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust");
        assert_eq!(results[0].url, "https://rust-lang.org");
        assert_eq!(results[0].snippet, "Systems language");
        // A null `content` becomes an empty snippet, not a dropped result.
        assert_eq!(results[1].snippet, "");
    }

    /// An untitled hit is still a usable link, so it keeps its place in the
    /// list under its URL rather than rendering as an empty bold span.
    #[test]
    fn an_empty_title_falls_back_to_the_url() {
        let response = parse(r#"{"results":[{"title":"  ","url":"https://example.org/page"}]}"#);

        let results = accept_response("q", response, 8).expect("results");

        assert_eq!(results[0].title, "https://example.org/page");
    }

    /// The rule that matters: partial upstream failure is normal metasearch
    /// behaviour, so results still win. Failing on `unresponsive_engines`
    /// would throw away every result whenever one engine hit a CAPTCHA.
    #[test]
    fn unresponsive_engines_do_not_fail_a_search_that_returned_results() {
        let response = parse(
            r#"{"results":[
                {"title":"Hit","url":"https://example.org","content":"c","engine":"brave"}
            ],"unresponsive_engines":[["duckduckgo","CAPTCHA"],["startpage","timeout"]]}"#,
        );

        let results = accept_response("q", response, 8).expect("results survive engine failures");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.org");
    }

    #[test]
    fn empty_results_fail_and_name_the_unresponsive_engines() {
        let response = parse(r#"{"results":[],"unresponsive_engines":[["duckduckgo","CAPTCHA"]]}"#);

        let err = accept_response("q", response, 8).expect_err("no results is a failure");
        let message = err.to_string();

        assert!(message.contains("no results for 'q'"), "{message}");
        assert!(message.contains("duckduckgo: CAPTCHA"), "{message}");
    }

    /// A response can be non-empty yet unusable; reporting success with nothing
    /// to show would leave the caller unable to tell search from silence.
    #[test]
    fn results_without_urls_are_treated_as_no_results() {
        let response = parse(r#"{"results":[{"title":"No link","url":"  ","content":"x"}]}"#);

        assert!(accept_response("q", response, 8).is_err());
    }

    #[test]
    fn results_respect_the_requested_limit() {
        let response = parse(
            r#"{"results":[
                {"title":"1","url":"https://a.example"},
                {"title":"2","url":"https://b.example"},
                {"title":"3","url":"https://c.example"}
            ]}"#,
        );

        let results = accept_response("q", response, 2).expect("results");

        assert_eq!(results.len(), 2);
        assert_eq!(results[1].url, "https://b.example");
    }

    #[test]
    fn diagnostics_name_contributing_engines_without_repeating_them() {
        let response = parse(
            r#"{"results":[
                {"title":"1","url":"https://a.example","engine":"google cse"},
                {"title":"2","url":"https://b.example","engine":"brave"},
                {"title":"3","url":"https://c.example","engine":"google cse"}
            ]}"#,
        );

        assert_eq!(contributing_engines(&response.results), "google cse, brave");
        assert!(accept_response("q", response, 8).is_ok());
    }

    #[test]
    fn engine_diagnostics_render_missing_and_unexpected_shapes() {
        assert_eq!(describe_engine_entries(&[]), "none");
        assert_eq!(
            describe_engine_entries(&[json!(["duckduckgo", "CAPTCHA"])]),
            "duckduckgo: CAPTCHA"
        );
        // An unexpected shape is rendered rather than silently dropped.
        assert_eq!(describe_engine_entries(&[json!("bare")]), "bare");
        assert_eq!(
            describe_engine_entries(&[json!({"engine": "x"})]),
            "{\"engine\":\"x\"}"
        );
    }
}
