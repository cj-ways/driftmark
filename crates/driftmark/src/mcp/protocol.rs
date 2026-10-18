//! The Model Context Protocol itself, through the rmcp crate.
//!
//! This is the only file that names rmcp. rmcp had three major versions in
//! 2026 (decision 0003), so its next breaking release should touch this file
//! and nothing else. The rest of the connector speaks in plain Rust and
//! `serde_json` values: a tool is a name, a description and an input schema,
//! and a call gives back JSON text or a sentence that says why it failed.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};

/// One tool as a client lists it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolInfo {
    /// The name a client calls it by.
    pub name: &'static str,
    /// A short name for people.
    pub title: &'static str,
    /// What it does, for the agent that decides whether to call it.
    pub description: &'static str,
    /// The JSON Schema of its arguments: an object schema.
    pub schema: Map<String, Value>,
    /// Whether it only reads. The note tools that reply, propose or record
    /// commits write to the notes file; every other tool only reads.
    pub read_only: bool,
}

/// What a tool call gives back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// The answer, as JSON text.
    Answer(String),
    /// Why the tool could not answer, as a sentence the agent can act on,
    /// such as a wrong argument or a range that names no commit.
    Refused(String),
}

/// What the connector offers: its tools and their answers. Calls run on a
/// blocking thread, because they read git and parse documents.
pub(crate) trait Connector: Send + Sync + 'static {
    /// The text a client shows the agent about the whole server.
    fn instructions(&self) -> String;
    /// The tools, in the order a client lists them.
    fn tools(&self) -> Vec<ToolInfo>;
    /// Runs one tool for `client`, as its handshake names
    /// itself, such as `claude-code`. `None` when no tool has this name.
    fn call(&self, name: &str, arguments: Map<String, Value>, client: &Client) -> Option<Reply>;
    /// Why no tool has this name, as the sentence of the protocol error.
    fn missing(&self, name: &str) -> String;
}

/// The name a client goes by when its handshake names none.
pub(crate) const UNNAMED_CLIENT: &str = "agent";

/// The client that called a tool, as its handshake names itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Client {
    /// Its name, such as `claude-code`: the name an agent's replies and
    /// records carry.
    pub name: String,
    /// Its version, as it gives it; empty when it gives none.
    pub version: String,
}

impl Client {
    /// A client that named itself `name`, at `version`.
    pub(crate) fn new(name: &str, version: &str) -> Client {
        Client {
            name: name.to_owned(),
            version: version.to_owned(),
        }
    }

    /// What it calls itself in full, such as `claude-code 2.1.0`, for the
    /// history of the notes it acts on: the MCP connector's `User-Agent`.
    pub(crate) fn agent(&self) -> String {
        let version = self.version.trim();
        if version.is_empty() {
            self.name.clone()
        } else {
            format!("{} {version}", self.name)
        }
    }
}

/// Serves `connector` over standard input and output until the client
/// closes standard input.
///
/// # Errors
///
/// A sentence when the session could not start or ended abnormally.
pub(crate) async fn serve_stdio<C: Connector>(connector: Arc<C>) -> Result<(), String> {
    let running = Server(connector)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| format!("the MCP session could not start: {error}"))?;
    running
        .waiting()
        .await
        .map(drop)
        .map_err(|error| format!("the MCP session stopped: {error}"))
}

/// rmcp's view of the connector.
struct Server<C>(Arc<C>);

impl<C: Connector> ServerHandler for Server<C> {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("driftmark", driftmark_core::version()).with_title("driftmark"),
            )
            .with_instructions(self.0.instructions())
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(
            self.0.tools().into_iter().map(tool).collect(),
        )))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let connector = Arc::clone(&self.0);
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();
        // MCP 2026-07-28 names the client on every request; earlier versions
        // name it once, in the `initialize` handshake.
        let client = context
            .meta
            .client_info()
            .or_else(|| {
                context
                    .peer
                    .peer_info()
                    .map(|info| info.client_info.clone())
            })
            .filter(|info| !info.name.trim().is_empty())
            .map_or_else(
                || Client::new(UNNAMED_CLIENT, ""),
                |info| Client::new(&info.name, &info.version),
            );
        let asked = name.clone();
        let reply = tokio::task::spawn_blocking(move || connector.call(&asked, arguments, &client))
            .await
            .map_err(|_| {
                ErrorData::internal_error(
                    "driftmark stopped while answering; this is a bug in driftmark",
                    None,
                )
            })?;
        match reply {
            None => Err(ErrorData::invalid_params(self.0.missing(&name), None)),
            Some(Reply::Answer(text)) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
            }
            Some(Reply::Refused(text)) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(text)]).into())
            }
        }
    }
}

/// A tool in rmcp's terms. No tool deletes anything or reaches beyond the
/// repository, and a tool that only reads gives the same answer when called
/// twice on the same files. A tool that writes a note reply, proposal or
/// commit adds one each time it is called, so it is not idempotent.
fn tool(info: ToolInfo) -> Tool {
    Tool::new(info.name, info.description, Arc::new(info.schema))
        .with_title(info.title)
        .with_annotations(
            ToolAnnotations::with_title(info.title)
                .read_only(info.read_only)
                .destructive(false)
                .idempotent(info.read_only)
                .open_world(false),
        )
}
