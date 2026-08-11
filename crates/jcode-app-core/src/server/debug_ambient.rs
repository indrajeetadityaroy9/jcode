use crate::ambient_runner::AmbientRunnerHandle;
use crate::provider::Provider;
use anyhow::Result;
use std::sync::Arc;

pub(super) async fn maybe_handle_ambient_command(
    cmd: &str,
    ambient_runner: &Option<AmbientRunnerHandle>,
    provider: &Arc<dyn Provider>,
) -> Result<Option<String>> {
    if cmd == "ambient:status" {
        let output = if let Some(runner) = ambient_runner {
            runner.status_json().await
        } else {
            serde_json::json!({
                "enabled": false,
                "status": "disabled",
                "message": "Ambient mode is not enabled in config"
            })
            .to_string()
        };
        return Ok(Some(output));
    }

    if cmd == "ambient:queue" {
        let output = if let Some(runner) = ambient_runner {
            runner.queue_json().await
        } else {
            "[]".to_string()
        };
        return Ok(Some(output));
    }

    if cmd == "ambient:trigger" {
        let output = if let Some(runner) = ambient_runner {
            runner.trigger().await;
            "Ambient cycle triggered".to_string()
        } else {
            return Err(anyhow::anyhow!("Ambient mode is not enabled"));
        };
        return Ok(Some(output));
    }

    if cmd == "ambient:log" {
        let output = if let Some(runner) = ambient_runner {
            runner.log_json().await
        } else {
            "[]".to_string()
        };
        return Ok(Some(output));
    }

    if cmd == "ambient:stop" {
        let output = if let Some(runner) = ambient_runner {
            runner.stop().await;
            "Ambient mode stopped".to_string()
        } else {
            return Err(anyhow::anyhow!("Ambient mode is not enabled"));
        };
        return Ok(Some(output));
    }

    if cmd == "ambient:start" {
        let output = if let Some(runner) = ambient_runner {
            if runner.start(Arc::clone(provider)).await {
                "Ambient mode started".to_string()
            } else {
                "Ambient mode is already running".to_string()
            }
        } else {
            return Err(anyhow::anyhow!("Ambient mode is not enabled in config"));
        };
        return Ok(Some(output));
    }

    if cmd == "ambient:help" {
        return Ok(Some(
            r#"Ambient mode debug commands (ambient: prefix):
  ambient:status              - Ambient + schedule runner state, counts, next due items
  ambient:queue               - Scheduled queue contents with target/session metadata
  ambient:trigger             - Manually trigger an ambient cycle
  ambient:log                 - Recent transcript summaries
  ambient:start               - Start/restart ambient mode
  ambient:stop                - Stop ambient mode"#
                .to_string(),
        ));
    }

    Ok(None)
}
