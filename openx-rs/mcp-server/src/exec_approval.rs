use std::path::PathBuf;
use std::sync::Arc;

use mcp_types::ElicitRequest;
use mcp_types::ElicitRequestParamsRequestedSchema;
use mcp_types::JSONRPCErrorError;
use mcp_types::ModelContextProtocolRequest;
use mcp_types::RequestId;
use openx_core::OpenXConversation;
use openx_core::protocol::Op;
use openx_core::protocol::ReviewDecision;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use tracing::error;

use crate::openx_tool_runner::INVALID_PARAMS_ERROR_CODE;

/// Conforms to [`mcp_types::ElicitRequestParams`] so that it can be used as the
/// `params` field of an [`ElicitRequest`].
#[derive(Debug, Deserialize, Serialize)]
pub struct ExecApprovalElicitRequestParams {
    // These fields are required so that `params`
    // conforms to ElicitRequestParams.
    pub message: String,

    #[serde(rename = "requestedSchema")]
    pub requested_schema: ElicitRequestParamsRequestedSchema,

    // These are additional fields the client can use to
    // correlate the request with the codex tool call.
    pub openx_elicitation: String,
    pub openx_mcp_tool_call_id: String,
    pub openx_event_id: String,
    pub openx_call_id: String,
    pub openx_command: Vec<String>,
    pub openx_cwd: PathBuf,
}

// TODO(mbolin): ExecApprovalResponse does not conform to ElicitResult. See:
// - https://github.com/modelcontextprotocol/modelcontextprotocol/blob/f962dc1780fa5eed7fb7c8a0232f1fc83ef220cd/schema/2025-06-18/schema.json#L617-L636
// - https://modelcontextprotocol.io/specification/draft/client/elicitation#protocol-messages
// It should have "action" and "content" fields.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecApprovalResponse {
    pub decision: ReviewDecision,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_exec_approval_request(
    command: Vec<String>,
    cwd: PathBuf,
    outgoing: Arc<crate::outgoing_message::OutgoingMessageSender>,
    codex: Arc<OpenXConversation>,
    request_id: RequestId,
    tool_call_id: String,
    event_id: String,
    call_id: String,
) {
    let escaped_command =
        shlex::try_join(command.iter().map(|s| s.as_str())).unwrap_or_else(|_| command.join(" "));
    let message = format!(
        "Allow OpenX to run `{escaped_command}` in `{cwd}`?",
        cwd = cwd.to_string_lossy()
    );

    let params = ExecApprovalElicitRequestParams {
        message,
        requested_schema: ElicitRequestParamsRequestedSchema {
            r#type: "object".to_string(),
            properties: json!({}),
            required: None,
        },
        openx_elicitation: "exec-approval".to_string(),
        openx_mcp_tool_call_id: tool_call_id.clone(),
        openx_event_id: event_id.clone(),
        openx_call_id: call_id,
        openx_command: command,
        openx_cwd: cwd,
    };
    let params_json = match serde_json::to_value(&params) {
        Ok(value) => value,
        Err(err) => {
            let message = format!("Failed to serialize ExecApprovalElicitRequestParams: {err}");
            error!("{message}");

            outgoing
                .send_error(
                    request_id.clone(),
                    JSONRPCErrorError {
                        code: INVALID_PARAMS_ERROR_CODE,
                        message,
                        data: None,
                    },
                )
                .await;

            return;
        }
    };

    let on_response = outgoing
        .send_request(ElicitRequest::METHOD, Some(params_json))
        .await;

    // Listen for the response on a separate task so we don't block the main agent loop.
    {
        let codex = codex.clone();
        let event_id = event_id.clone();
        tokio::spawn(async move {
            on_exec_approval_response(event_id, on_response, codex).await;
        });
    }
}

async fn on_exec_approval_response(
    event_id: String,
    receiver: tokio::sync::oneshot::Receiver<mcp_types::Result>,
    codex: Arc<OpenXConversation>,
) {
    let response = receiver.await;
    let value = match response {
        Ok(value) => value,
        Err(err) => {
            error!("request failed: {err:?}");
            return;
        }
    };

    // Try to deserialize `value` and then make the appropriate call to `codex`.
    let response = serde_json::from_value::<ExecApprovalResponse>(value).unwrap_or_else(|err| {
        error!("failed to deserialize ExecApprovalResponse: {err}");
        // If we cannot deserialize the response, we deny the request to be
        // conservative.
        ExecApprovalResponse {
            decision: ReviewDecision::Denied,
        }
    });

    if let Err(err) = codex
        .submit(Op::ExecApproval {
            id: event_id,
            decision: response.decision,
        })
        .await
    {
        error!("failed to submit ExecApproval: {err}");
    }
}
