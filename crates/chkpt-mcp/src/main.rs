use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::Deserialize;

use std::path::Path;

// ---------- Parameter types ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SaveParams {
    /// Path to the workspace directory
    workspace_path: String,
    /// Optional message for the checkpoint
    message: Option<String>,
    /// Include dependency directories (node_modules, .venv, etc.)
    include_deps: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListParams {
    /// Path to the workspace directory
    workspace_path: String,
    /// Maximum number of checkpoints to return
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RestoreParams {
    /// Path to the workspace directory
    workspace_path: String,
    /// Snapshot ID to restore (or "latest")
    snapshot_id: String,
    /// If true, only report what would change without modifying files
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeleteParams {
    /// Path to the workspace directory
    workspace_path: String,
    /// Snapshot ID to delete
    snapshot_id: String,
}

// ---------- Server ----------

#[derive(Debug, Clone)]
struct ChkpttServer {
    tool_router: ToolRouter<Self>,
}

impl ChkpttServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl ChkpttServer {
    /// Save a workspace checkpoint.
    ///
    /// Creates a new snapshot of the workspace at the given path,
    /// capturing all tracked files with content-addressed deduplication.
    #[tool(
        name = "checkpoint_save",
        description = "Save a workspace checkpoint. Creates a snapshot of all tracked files with content-addressed deduplication."
    )]
    fn checkpoint_save(&self, Parameters(params): Parameters<SaveParams>) -> String {
        let workspace_path = Path::new(&params.workspace_path);
        let options = chkpt_core::ops::save::SaveOptions {
            message: params.message,
            include_deps: params.include_deps.unwrap_or(false),
        };
        match chkpt_core::ops::save::save(workspace_path, options) {
            Ok(result) => serde_json::json!({
                "snapshot_id": result.snapshot_id,
                "stats": {
                    "total_files": result.stats.total_files,
                    "total_bytes": result.stats.total_bytes,
                    "new_objects": result.stats.new_objects,
                }
            })
            .to_string(),
            Err(e) => serde_json::json!({
                "error": e.to_string()
            })
            .to_string(),
        }
    }

    /// List checkpoints for a workspace.
    ///
    /// Returns a list of snapshots ordered by creation time (newest first).
    #[tool(
        name = "checkpoint_list",
        description = "List checkpoints for a workspace. Returns snapshots ordered by creation time (newest first)."
    )]
    fn checkpoint_list(&self, Parameters(params): Parameters<ListParams>) -> String {
        let workspace_path = Path::new(&params.workspace_path);
        match chkpt_core::ops::list::list(workspace_path, params.limit) {
            Ok(snapshots) => {
                let entries: Vec<serde_json::Value> = snapshots
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id,
                            "created_at": s.created_at.to_rfc3339(),
                            "message": s.message,
                            "stats": {
                                "total_files": s.stats.total_files,
                                "total_bytes": s.stats.total_bytes,
                                "new_objects": s.stats.new_objects,
                            }
                        })
                    })
                    .collect();
                serde_json::json!(entries).to_string()
            }
            Err(e) => serde_json::json!({
                "error": e.to_string()
            })
            .to_string(),
        }
    }

    /// Restore a workspace to a checkpoint.
    ///
    /// Restores the workspace to match the state of the specified snapshot.
    /// Supports dry-run mode to preview changes without modifying files.
    #[tool(
        name = "checkpoint_restore",
        description = "Restore a workspace to a checkpoint. Supports dry-run mode to preview changes without modifying files."
    )]
    fn checkpoint_restore(&self, Parameters(params): Parameters<RestoreParams>) -> String {
        let workspace_path = Path::new(&params.workspace_path);
        let options = chkpt_core::ops::restore::RestoreOptions {
            dry_run: params.dry_run.unwrap_or(false),
        };
        match chkpt_core::ops::restore::restore(workspace_path, &params.snapshot_id, options) {
            Ok(result) => serde_json::json!({
                "snapshot_id": result.snapshot_id,
                "files_added": result.files_added,
                "files_changed": result.files_changed,
                "files_removed": result.files_removed,
                "files_unchanged": result.files_unchanged,
            })
            .to_string(),
            Err(e) => serde_json::json!({
                "error": e.to_string()
            })
            .to_string(),
        }
    }

    /// Delete a checkpoint.
    ///
    /// Deletes the specified snapshot and runs garbage collection to
    /// remove unreachable objects.
    #[tool(
        name = "checkpoint_delete",
        description = "Delete a checkpoint and run garbage collection to remove unreachable objects."
    )]
    fn checkpoint_delete(&self, Parameters(params): Parameters<DeleteParams>) -> String {
        let workspace_path = Path::new(&params.workspace_path);
        match chkpt_core::ops::delete::delete(workspace_path, &params.snapshot_id) {
            Ok(()) => serde_json::json!({
                "deleted": true,
                "snapshot_id": params.snapshot_id,
                "message": format!("Snapshot {} deleted successfully", params.snapshot_id),
            })
            .to_string(),
            Err(e) => serde_json::json!({
                "error": e.to_string()
            })
            .to_string(),
        }
    }
}

#[tool_handler]
impl ServerHandler for ChkpttServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "chkpt".to_string(),
                title: Some("chkpt - Workspace Checkpoint Manager".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: Some(
                    "MCP server for saving, listing, restoring, and deleting workspace checkpoints"
                        .to_string(),
                ),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use the checkpoint tools to save, list, restore, and delete workspace snapshots. \
                 All tools require a workspace_path parameter pointing to the directory you want \
                 to checkpoint."
                    .to_string(),
            ),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Log to stderr so stdout remains clean for MCP JSON-RPC messages
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Starting chkpt MCP server on stdio");

    let server = ChkpttServer::new();
    let service = server.serve(stdio()).await?;

    service.waiting().await?;

    Ok(())
}
