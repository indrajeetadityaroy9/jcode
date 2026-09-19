use super::{Tool, ToolContext, ToolOutput};
use crate::config::WebSearchBackend;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

mod chain;
mod searxng;

/// Web search.
///
/// The default backend drives a chain of real search engines in a headless
/// Chrome. A self-hosted SearXNG instance is available behind
/// `[websearch] backend = "searxng"`, and is never reached by failover: an
/// instance that is down should say so rather than silently divert traffic.
pub struct WebSearchTool {
    /// Only the SearXNG backend issues HTTP itself; the Chrome chain goes
    /// through the browser.
    client: reqwest::Client,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            client: crate::provider::shared_http_client(),
        }
    }
}

#[derive(Deserialize)]
struct WebSearchInput {
    query: String,
    #[serde(default)]
    num_results: Option<usize>,
}

/// One result, independent of which backend produced it.
#[derive(Debug, PartialEq)]
pub(super) struct SearchResult {
    pub(super) title: String,
    pub(super) url: String,
    pub(super) snippet: String,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "websearch"
    }

    fn description(&self) -> &str {
        "Search the web."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "intent": super::intent_schema_property(),
                "query": {
                    "type": "string",
                    "description": "Search query."
                },
                "num_results": {
                    "type": "integer",
                    "description": "Max results."
                }
            }
        })
    }

    async fn execute(&self, input: Value, _ctx: ToolContext) -> Result<ToolOutput> {
        let params: WebSearchInput = serde_json::from_value(input)?;
        let num_results = params.num_results.unwrap_or(8).min(20);

        let results = match crate::config::config().websearch.backend {
            WebSearchBackend::Chrome => chain::search(&params.query, num_results).await?,
            WebSearchBackend::Searxng => {
                searxng::search(&self.client, &params.query, num_results).await?
            }
        };

        Ok(ToolOutput::new(render_results(&params.query, &results)))
    }
}

/// Render results for the model. No other crate parses this text, but the
/// shape is what the model has learned to read, so it must not drift.
fn render_results(query: &str, results: &[SearchResult]) -> String {
    let mut output = format!("Search results for: {}\n\n", query);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. **{}**\n   {}\n   {}\n\n",
            i + 1,
            result.title,
            result.url,
            result.snippet
        ));
    }
    output
}
