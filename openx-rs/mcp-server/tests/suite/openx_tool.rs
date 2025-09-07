use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::path::PathBuf;

use mcp_types::ElicitRequest;
use mcp_types::ElicitRequestParamsRequestedSchema;
use mcp_types::JSONRPC_VERSION;
use mcp_types::JSONRPCRequest;
use mcp_types::JSONRPCResponse;
use mcp_types::ModelContextProtocolRequest;
use mcp_types::RequestId;
use openx_core::protocol::FileChange;
use openx_core::protocol::ReviewDecision;
use openx_core::spawn::CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR;
use openx_mcp_server::CodexToolCallParam;
use openx_mcp_server::ExecApprovalElicitRequestParams;
use openx_mcp_server::ExecApprovalResponse;
use openx_mcp_server::PatchApprovalElicitRequestParams;
use openx_mcp_server::PatchApprovalResponse;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::MockServer;

use mcp_test_support::McpProcess;
use mcp_test_support::create_apply_patch_sse_response;
use mcp_test_support::create_final_assistant_message_sse_response;
use mcp_test_support::create_mock_chat_completions_server;
use mcp_test_support::create_shell_sse_response;

// Allow ample time on slower CI or under load to avoid flakes.
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Test that a shell command that is not on the "trusted" list triggers an
/// elicitation request to the MCP and that sending the approval runs the
/// command, as expected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_shell_command_approval_triggers_elicitation() {
    if env::var(CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR).is_ok() {
        println!(
            "Skipping test because it cannot execute when network is disabled in a OpenX sandbox."
        );
        return;
    }

    // Apparently `#[tokio::test]` must return `()`, so we create a helper
    // function that returns `Result` so we can use `?` in favor of `unwrap`.
    if let Err(err) = shell_command_approval_triggers_elicitation().await {
        panic!("failure: {err}");
    }
}

async fn shell_command_approval_triggers_elicitation() -> anyhow::Result<()> {
    // Use a simple, untrusted command that creates a file so we can
    // observe a side-effect.
    //
    // Cross‑platform approach: run a tiny Python snippet to touch the file
    // using `python3 -c ...` on all platforms.
    let workdir_for_shell_function_call = TempDir::new()?;
    let created_filename = "created_by_shell_tool.txt";
    let created_file = workdir_for_shell_function_call
        .path()
        .join(created_filename);

    let shell_command = vec![
        "python3".to_string(),
        "-c".to_string(),
        format!("import pathlib; pathlib.Path('{created_filename}').touch()"),
    ];

    let McpHandle {
        process: mut mcp_process,
        server: _server,
        dir: _dir,
    } = create_mcp_process(vec![
        create_shell_sse_response(
            shell_command.clone(),
            Some(workdir_for_shell_function_call.path()),
            Some(5_000),
            "call1234",
        )?,
        create_final_assistant_message_sse_response("File created!")?,
    ])
    .await?;

    // Send a "codex" tool request, which should hit the completions endpoint.
    // In turn, it should reply with a tool call, which the MCP should forward
    // as an elicitation.
    let openx_request_id = mcp_process
        .send_openx_tool_call(CodexToolCallParam {
            prompt: "run `git init`".to_string(),
            ..Default::default()
        })
        .await?;
    let elicitation_request = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp_process.read_stream_until_request_message(),
    )
    .await??;

    let elicitation_request_id = elicitation_request.id.clone();
    let params = serde_json::from_value::<ExecApprovalElicitRequestParams>(
        elicitation_request
            .params
            .clone()
            .ok_or_else(|| anyhow::anyhow!("elicitation_request.params must be set"))?,
    )?;
    let expected_elicitation_request = create_expected_elicitation_request(
        elicitation_request_id.clone(),
        shell_command.clone(),
        workdir_for_shell_function_call.path(),
        openx_request_id.to_string(),
        params.openx_event_id.clone(),
    )?;
    assert_eq!(expected_elicitation_request, elicitation_request);

    // Accept the `git init` request by responding to the elicitation.
    mcp_process
        .send_response(
            elicitation_request_id,
            serde_json::to_value(ExecApprovalResponse {
                decision: ReviewDecision::Approved,
            })?,
        )
        .await?;

    // Verify task_complete notification arrives before the tool call completes.
    #[expect(clippy::expect_used)]
    let _task_complete = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp_process.read_stream_until_legacy_task_complete_notification(),
    )
    .await
    .expect("task_complete_notification timeout")
    .expect("task_complete_notification resp");

    // Verify the original `codex` tool call completes and that the file was created.
    let openx_response = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp_process.read_stream_until_response_message(RequestId::Integer(openx_request_id)),
    )
    .await??;
    assert_eq!(
        JSONRPCResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId::Integer(openx_request_id),
            result: json!({
                "content": [
                    {
                        "text": "File created!",
                        "type": "text"
                    }
                ]
            }),
        },
        openx_response
    );

    assert!(created_file.is_file(), "created file should exist");

    Ok(())
}

fn create_expected_elicitation_request(
    elicitation_request_id: RequestId,
    command: Vec<String>,
    workdir: &Path,
    openx_mcp_tool_call_id: String,
    openx_event_id: String,
) -> anyhow::Result<JSONRPCRequest> {
    let expected_message = format!(
        "Allow OpenX to run `{}` in `{}`?",
        shlex::try_join(command.iter().map(|s| s.as_ref()))?,
        workdir.to_string_lossy()
    );
    Ok(JSONRPCRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: elicitation_request_id,
        method: ElicitRequest::METHOD.to_string(),
        params: Some(serde_json::to_value(&ExecApprovalElicitRequestParams {
            message: expected_message,
            requested_schema: ElicitRequestParamsRequestedSchema {
                r#type: "object".to_string(),
                properties: json!({}),
                required: None,
            },
            openx_elicitation: "exec-approval".to_string(),
            openx_mcp_tool_call_id,
            openx_event_id,
            openx_command: command,
            openx_cwd: workdir.to_path_buf(),
            openx_call_id: "call1234".to_string(),
        })?),
    })
}

/// Test that patch approval triggers an elicitation request to the MCP and that
/// sending the approval applies the patch, as expected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_patch_approval_triggers_elicitation() {
    if env::var(CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR).is_ok() {
        println!(
            "Skipping test because it cannot execute when network is disabled in a OpenX sandbox."
        );
        return;
    }

    if let Err(err) = patch_approval_triggers_elicitation().await {
        panic!("failure: {err}");
    }
}

async fn patch_approval_triggers_elicitation() -> anyhow::Result<()> {
    let cwd = TempDir::new()?;
    let test_file = cwd.path().join("destination_file.txt");
    std::fs::write(&test_file, "original content\n")?;

    let patch_content = format!(
        "*** Begin Patch\n*** Update File: {}\n-original content\n+modified content\n*** End Patch",
        test_file.as_path().to_string_lossy()
    );

    let McpHandle {
        process: mut mcp_process,
        server: _server,
        dir: _dir,
    } = create_mcp_process(vec![
        create_apply_patch_sse_response(&patch_content, "call1234")?,
        create_final_assistant_message_sse_response("Patch has been applied successfully!")?,
    ])
    .await?;

    // Send a "codex" tool request that will trigger the apply_patch command
    let openx_request_id = mcp_process
        .send_openx_tool_call(CodexToolCallParam {
            cwd: Some(cwd.path().to_string_lossy().to_string()),
            prompt: "please modify the test file".to_string(),
            ..Default::default()
        })
        .await?;
    let elicitation_request = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp_process.read_stream_until_request_message(),
    )
    .await??;

    let elicitation_request_id = RequestId::Integer(0);

    let mut expected_changes = HashMap::new();
    expected_changes.insert(
        test_file.as_path().to_path_buf(),
        FileChange::Update {
            unified_diff: "@@ -1 +1 @@\n-original content\n+modified content\n".to_string(),
            move_path: None,
        },
    );

    let expected_elicitation_request = create_expected_patch_approval_elicitation_request(
        elicitation_request_id.clone(),
        expected_changes,
        None, // No grant_root expected
        None, // No reason expected
        openx_request_id.to_string(),
        "1".to_string(),
    )?;
    assert_eq!(expected_elicitation_request, elicitation_request);

    // Accept the patch approval request by responding to the elicitation
    mcp_process
        .send_response(
            elicitation_request_id,
            serde_json::to_value(PatchApprovalResponse {
                decision: ReviewDecision::Approved,
            })?,
        )
        .await?;

    // Verify the original `codex` tool call completes
    let openx_response = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp_process.read_stream_until_response_message(RequestId::Integer(openx_request_id)),
    )
    .await??;
    assert_eq!(
        JSONRPCResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId::Integer(openx_request_id),
            result: json!({
                "content": [
                    {
                        "text": "Patch has been applied successfully!",
                        "type": "text"
                    }
                ]
            }),
        },
        openx_response
    );

    let file_contents = std::fs::read_to_string(test_file.as_path())?;
    assert_eq!(file_contents, "modified content\n");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_openx_tool_passes_base_instructions() {
    if std::env::var(CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR).is_ok() {
        println!(
            "Skipping test because it cannot execute when network is disabled in a OpenX sandbox."
        );
        return;
    }

    // Apparently `#[tokio::test]` must return `()`, so we create a helper
    // function that returns `Result` so we can use `?` in favor of `unwrap`.
    if let Err(err) = openx_tool_passes_base_instructions().await {
        panic!("failure: {err}");
    }
}

async fn openx_tool_passes_base_instructions() -> anyhow::Result<()> {
    #![expect(clippy::unwrap_used)]

    let server =
        create_mock_chat_completions_server(vec![create_final_assistant_message_sse_response(
            "Enjoy!",
        )?])
        .await;

    // Run `codex mcp` with a specific config.toml.
    let openx_home = TempDir::new()?;
    create_config_toml(openx_home.path(), &server.uri())?;
    let mut mcp_process = McpProcess::new(openx_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp_process.initialize()).await??;

    // Send a "codex" tool request, which should hit the completions endpoint.
    let openx_request_id = mcp_process
        .send_openx_tool_call(CodexToolCallParam {
            prompt: "How are you?".to_string(),
            base_instructions: Some("You are a helpful assistant.".to_string()),
            ..Default::default()
        })
        .await?;

    let openx_response = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp_process.read_stream_until_response_message(RequestId::Integer(openx_request_id)),
    )
    .await??;
    assert_eq!(
        JSONRPCResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId::Integer(openx_request_id),
            result: json!({
                "content": [
                    {
                        "text": "Enjoy!",
                        "type": "text"
                    }
                ]
            }),
        },
        openx_response
    );

    let requests = server.received_requests().await.unwrap();
    let request = requests[0].body_json::<serde_json::Value>().unwrap();
    let instructions = request["messages"][0]["content"].as_str().unwrap();
    assert!(instructions.starts_with("You are a helpful assistant."));

    Ok(())
}

fn create_expected_patch_approval_elicitation_request(
    elicitation_request_id: RequestId,
    changes: HashMap<PathBuf, FileChange>,
    grant_root: Option<PathBuf>,
    reason: Option<String>,
    openx_mcp_tool_call_id: String,
    openx_event_id: String,
) -> anyhow::Result<JSONRPCRequest> {
    let mut message_lines = Vec::new();
    if let Some(r) = &reason {
        message_lines.push(r.clone());
    }
    message_lines.push("Allow OpenX to apply proposed code changes?".to_string());

    Ok(JSONRPCRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: elicitation_request_id,
        method: ElicitRequest::METHOD.to_string(),
        params: Some(serde_json::to_value(&PatchApprovalElicitRequestParams {
            message: message_lines.join("\n"),
            requested_schema: ElicitRequestParamsRequestedSchema {
                r#type: "object".to_string(),
                properties: json!({}),
                required: None,
            },
            openx_elicitation: "patch-approval".to_string(),
            openx_mcp_tool_call_id,
            openx_event_id,
            openx_reason: reason,
            openx_grant_root: grant_root,
            openx_changes: changes,
            openx_call_id: "call1234".to_string(),
        })?),
    })
}

/// This handle is used to ensure that the MockServer and TempDir are not dropped while
/// the McpProcess is still running.
pub struct McpHandle {
    pub process: McpProcess,
    /// Retain the server for the lifetime of the McpProcess.
    #[allow(dead_code)]
    server: MockServer,
    /// Retain the temporary directory for the lifetime of the McpProcess.
    #[allow(dead_code)]
    dir: TempDir,
}

async fn create_mcp_process(responses: Vec<String>) -> anyhow::Result<McpHandle> {
    let server = create_mock_chat_completions_server(responses).await;
    let openx_home = TempDir::new()?;
    create_config_toml(openx_home.path(), &server.uri())?;
    let mut mcp_process = McpProcess::new(openx_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp_process.initialize()).await??;
    Ok(McpHandle {
        process: mcp_process,
        server,
        dir: openx_home,
    })
}

/// Create a OpenX config that uses the mock server as the model provider.
/// It also uses `approval_policy = "untrusted"` so that we exercise the
/// elicitation code path for shell commands.
fn create_config_toml(openx_home: &Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = openx_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "untrusted"
sandbox_policy = "read-only"

model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "chat"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}
