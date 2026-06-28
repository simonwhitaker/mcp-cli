use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use nu_ansi_term::{Color, Style};
use reedline::{
    ColumnarMenu, Completer, DefaultHinter, DefaultPrompt, DefaultPromptSegment, Emacs,
    ExampleHighlighter, FileBackedHistory, KeyCode, KeyModifiers, MenuBuilder, Reedline,
    ReedlineEvent, ReedlineMenu, Signal, Span, Suggestion, default_emacs_keybindings,
};
use rmcp::model::{Resource, Tool};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::{
    client_handler::ClientNotification,
    format::Formatter,
    session::{McpSession, object_from_value},
};

struct CommandSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    usage_hint: Option<&'static str>,
    description: &'static str,
}

impl CommandSpec {
    fn doc_string(&self) -> String {
        let mut doc = String::new();
        let mut entry = self.usage_hint.unwrap_or(self.name).to_string();
        for alias in self.aliases {
            entry.push_str(", ");
            entry.push_str(alias);
        }
        doc.push_str(format!("{:30} ", entry).as_str());
        doc.push_str(self.description);
        doc
    }
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        aliases: &["?"],
        usage_hint: None,
        description: "Show this help",
    },
    CommandSpec {
        name: "info",
        aliases: &[],
        usage_hint: None,
        description: "Show server metadata and capabilities",
    },
    CommandSpec {
        name: "tools",
        aliases: &[],
        usage_hint: None,
        description: "List tools",
    },
    CommandSpec {
        name: "resources",
        aliases: &[],
        usage_hint: None,
        description: "List resources",
    },
    CommandSpec {
        name: "resource",
        aliases: &[],
        usage_hint: Some("resource RESOURCE"),
        description: "Show a resource's contents",
    },
    CommandSpec {
        name: "schema",
        aliases: &[],
        usage_hint: Some("schema TOOL"),
        description: "Pretty-print a tool input schema",
    },
    CommandSpec {
        name: "tool",
        aliases: &[],
        usage_hint: Some("tool TOOL [key=value ... | --json '{...}' | @file.json]"),
        description: "Call a tool with arguments specified as key=value pairs, raw JSON, or a JSON file.",
    },
    CommandSpec {
        name: "raw",
        aliases: &[],
        usage_hint: Some("raw METHOD [JSON]"),
        description: "Send a raw MCP request with optional JSON parameters.",
    },
    CommandSpec {
        name: "reload",
        aliases: &[],
        usage_hint: None,
        description: "Refresh tool metadata from the server.",
    },
    CommandSpec {
        name: "quit",
        aliases: &["exit"],
        usage_hint: None,
        description: "Close the session and exit the REPL.",
    },
];

const COMPLETION_MENU: &str = "completion_menu";

#[derive(Debug, Error)]
pub enum ReplError {
    #[error("{0}")]
    Command(String),
}

pub struct Repl {
    editor: Reedline,
    formatter: Formatter,
    prompt: DefaultPrompt,
    completion_state: CompletionState,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionState {
    tool_names: Vec<String>,
    tool_args: Vec<(String, Vec<String>)>,
    resource_uris: Vec<String>,
}

impl CompletionState {
    pub fn from_mcp_primitives(tools: &[Tool], resources: &[Resource]) -> Self {
        let mut tool_names = tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        tool_names.sort();
        let tool_args = tools
            .iter()
            .map(|tool| {
                (
                    tool.name.to_string(),
                    schema_arg_names(&Value::Object((*tool.input_schema).clone())),
                )
            })
            .collect();

        let mut resource_uris = resources
            .iter()
            .map(|resource| resource.uri.to_string())
            .collect::<Vec<_>>();
        resource_uris.sort();

        Self {
            tool_names,
            tool_args,
            resource_uris,
        }
    }

    fn arg_names(&self, tool_name: &str) -> Vec<String> {
        self.tool_args
            .iter()
            .find(|(name, _)| name == tool_name)
            .map(|(_, args)| args.clone())
            .unwrap_or_default()
    }
}

impl Repl {
    pub fn new(
        server_name: &str,
        tools: &[Tool],
        resources: &[Resource],
        history_path: Option<PathBuf>,
        formatter: Formatter,
    ) -> Result<Self> {
        let completion_state = CompletionState::from_mcp_primitives(tools, resources);
        let prompt_text = if server_name.is_empty() {
            "mcp".to_string()
        } else {
            format!("mcp:{server_name}")
        };
        let prompt = DefaultPrompt::new(
            DefaultPromptSegment::Basic(prompt_text),
            DefaultPromptSegment::Empty,
        );

        let mut editor = build_editor(completion_state.clone());

        if let Some(path) = history_path.or_else(default_history_path) {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create history directory: {}", parent.display())
                })?;
            }
            let history = FileBackedHistory::with_file(10_000, path)?;
            editor = editor.with_history(Box::new(history));
        }

        Ok(Self {
            editor,
            formatter,
            prompt,
            completion_state,
        })
    }

    pub async fn run(&mut self, session: &mut McpSession) -> Result<()> {
        loop {
            self.print_notifications(session).await;
            match self.editor.read_line(&self.prompt) {
                Ok(Signal::Success(line)) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    match self.dispatch(line, session).await {
                        Ok(Dispatch::Continue) => {}
                        Ok(Dispatch::Quit) => break,
                        Err(error) => eprintln!("{}", self.formatter.error(&error.to_string())),
                    }
                }
                Ok(Signal::CtrlD) => break,
                Ok(Signal::CtrlC) => {
                    println!("^C");
                    continue;
                }
                Ok(Signal::HostCommand(command) | Signal::ExternalBreak(command)) => {
                    if !command.trim().is_empty() {
                        match self.dispatch(command.trim(), session).await {
                            Ok(Dispatch::Quit) => break,
                            Ok(Dispatch::Continue) => {}
                            Err(error) => eprintln!("{}", self.formatter.error(&error.to_string())),
                        }
                    }
                }
                Ok(_) => continue,
                Err(error) => bail!("failed to read from terminal: {error}"),
            }
        }
        Ok(())
    }

    async fn dispatch(&mut self, line: &str, session: &mut McpSession) -> Result<Dispatch> {
        let command = parse_command(line)?;
        match command {
            ReplCommand::Help => {
                println!("{}", help_text(self.formatter));
            }
            ReplCommand::Info => {
                let info = session.server_info()?;
                println!("{}", self.formatter.server_info(info.as_ref()));
            }
            ReplCommand::Tools => {
                println!("{}", self.formatter.tools(session.tools()));
            }
            ReplCommand::Resources => {
                println!("{}", self.formatter.resources(session.resources()));
            }
            ReplCommand::Schema { tool } => {
                let tool = session
                    .tool(&tool)
                    .with_context(|| format!("unknown tool: {tool}"))?;
                println!("{}", self.formatter.schema(tool));
            }
            ReplCommand::Resource { uri } => {
                let result = session.get_resource(&uri).await?;
                println!("{}", self.formatter.resource(&result));
            }
            ReplCommand::Tool { name, arguments } => {
                let result = session.call_tool(&name, arguments).await?;
                println!("{}", self.formatter.tool_result(&result));
            }
            ReplCommand::Raw { method, params } => {
                let result = session.raw_request(method, params).await?;
                println!("{}", self.formatter.json_value(&result));
            }
            ReplCommand::Reload => {
                session.refresh().await?;
                self.completion_state =
                    CompletionState::from_mcp_primitives(session.tools(), session.resources());
                self.rebuild_editor_completer();
                println!("reloaded {} tools", session.tools().len());
            }
            ReplCommand::Quit => return Ok(Dispatch::Quit),
        }
        Ok(Dispatch::Continue)
    }

    async fn print_notifications(&self, session: &McpSession) {
        for notification in session.drain_notifications().await {
            match notification {
                ClientNotification::Log(log) => {
                    eprintln!(
                        "{} {}",
                        self.formatter.dim(&format!("[{:?}]", log.level)),
                        self.formatter.json_value(&log.data)
                    );
                }
                ClientNotification::Progress(progress) => {
                    eprintln!(
                        "{} {}",
                        self.formatter.dim("[progress]"),
                        self.formatter.json_value(&progress)
                    );
                }
                ClientNotification::Cancelled(cancelled) => {
                    eprintln!(
                        "{} {}",
                        self.formatter.dim("[cancelled]"),
                        self.formatter.json_value(&cancelled)
                    );
                }
                ClientNotification::ElicitationDeclined(message) => {
                    eprintln!("{} {message}", self.formatter.dim("[elicitation]"));
                }
            }
        }
    }

    fn rebuild_editor_completer(&mut self) {
        let editor = std::mem::replace(&mut self.editor, Reedline::create());
        self.editor = editor.with_completer(Box::new(InspectorCompleter::new(
            self.completion_state.clone(),
        )));
    }
}

fn build_editor(completion_state: CompletionState) -> Reedline {
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(COMPLETION_MENU.to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );

    let completion_menu = Box::new(ColumnarMenu::default().with_name(COMPLETION_MENU));
    let edit_mode = Box::new(Emacs::new(keybindings));

    Reedline::create()
        .with_completer(Box::new(InspectorCompleter::new(completion_state)))
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode)
        .with_hinter(Box::new(DefaultHinter::default()))
        .with_highlighter(Box::new(ExampleHighlighter::new(
            // TODO: also include aliases here?
            COMMANDS
                .iter()
                .map(|command| command.name.to_string())
                .collect(),
        )))
        .with_partial_completions(true)
        .with_quick_completions(true)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReplCommand {
    Help,
    Info,
    Tools,
    Schema {
        tool: String,
    },
    Tool {
        name: String,
        arguments: Value,
    },
    Raw {
        method: String,
        params: Option<Value>,
    },
    Resources,
    Resource {
        uri: String,
    },
    Reload,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dispatch {
    Continue,
    Quit,
}

pub fn parse_command(line: &str) -> Result<ReplCommand> {
    let words = shlex::split(line).context("failed to parse command line")?;
    let Some((command, rest)) = words.split_first() else {
        bail!("empty command");
    };
    match command.as_str() {
        "help" | "?" => Ok(ReplCommand::Help),
        "info" => Ok(ReplCommand::Info),
        "tools" => Ok(ReplCommand::Tools),
        "resources" => Ok(ReplCommand::Resources),
        "resource" => {
            let uri = expect_one(rest, "resource RESOURCE")?;
            Ok(ReplCommand::Resource { uri })
        }
        "schema" => {
            let tool = expect_one(rest, "schema TOOL")?;
            Ok(ReplCommand::Schema { tool })
        }
        "tool" => parse_tool_command(rest),
        "raw" => parse_raw_command(rest),
        "reload" => Ok(ReplCommand::Reload),
        "quit" | "exit" => Ok(ReplCommand::Quit),
        other => bail!("unknown command: {other}"),
    }
}

fn parse_tool_command(words: &[String]) -> Result<ReplCommand> {
    let Some((name, rest)) = words.split_first() else {
        bail!("usage: tool TOOL [key=value ... | --json '{{...}}' | @file.json]");
    };
    let arguments = parse_arguments(rest)?;
    Ok(ReplCommand::Tool {
        name: name.clone(),
        arguments,
    })
}

fn parse_raw_command(words: &[String]) -> Result<ReplCommand> {
    let Some((method, rest)) = words.split_first() else {
        bail!("usage: raw METHOD [JSON]");
    };
    let params = if rest.is_empty() {
        None
    } else {
        Some(serde_json::from_str(&rest.join(" ")).context("raw params must be valid JSON")?)
    };
    Ok(ReplCommand::Raw {
        method: method.clone(),
        params,
    })
}

pub fn parse_arguments(words: &[String]) -> Result<Value> {
    if words.is_empty() {
        return Ok(Value::Object(Map::new()));
    }

    if words[0] == "--json" {
        let json_text = words.get(1).context("usage: tool TOOL --json '{...}'")?;
        if words.len() > 2 {
            bail!("--json accepts exactly one JSON object argument");
        }
        return Ok(Value::Object(object_from_value(serde_json::from_str(
            json_text,
        )?)?));
    }

    if words.len() == 1 && words[0].starts_with('@') {
        let path = &words[0][1..];
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read JSON argument file: {path}"))?;
        return Ok(Value::Object(object_from_value(serde_json::from_str(
            &text,
        )?)?));
    }

    let mut root = Map::new();
    for word in words {
        if word.starts_with('@') {
            bail!("@file must be the only tool argument");
        }
        let Some((key, raw_value)) = word.split_once('=') else {
            bail!("expected key=value argument, got {word}");
        };
        if key.is_empty() {
            bail!("argument key cannot be empty");
        }
        insert_dotted(&mut root, key, parse_scalar(raw_value))?;
    }
    Ok(Value::Object(root))
}

fn parse_scalar(raw: &str) -> Value {
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ if raw.starts_with('{') || raw.starts_with('[') => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
        }
        _ => serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string())),
    }
}

fn insert_dotted(root: &mut Map<String, Value>, key: &str, value: Value) -> Result<()> {
    let parts = key.split('.').collect::<Vec<_>>();
    if parts.iter().any(|part| part.is_empty()) {
        bail!("invalid dotted key: {key}");
    }
    insert_path(root, &parts, value)
}

fn insert_path(root: &mut Map<String, Value>, parts: &[&str], value: Value) -> Result<()> {
    if parts.len() == 1 {
        root.insert(parts[0].to_string(), value);
        return Ok(());
    }

    let entry = root
        .entry(parts[0].to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(next) = entry else {
        bail!("cannot assign nested key through non-object: {}", parts[0]);
    };
    insert_path(next, &parts[1..], value)
}

fn expect_one(words: &[String], usage: &str) -> Result<String> {
    match words {
        [one] => Ok(one.clone()),
        _ => bail!("usage: {usage}"),
    }
}

fn help_text(formatter: Formatter) -> String {
    if formatter.json_mode() {
        return formatter.json_value(&json!({
            "commands": COMMANDS.iter().map(|command| {
                json!({
                    "name": command.name,
                    "aliases": command.aliases,
                    "usage_hint": &command.usage_hint.unwrap_or(""),
                    "description": command.description,
                })
            }).collect::<Vec<_>>()
        }));
    }
    let mut doc_strings = COMMANDS
        .iter()
        .map(|command| command.doc_string())
        .collect::<Vec<_>>();
    doc_strings.sort();
    ["Commands:", doc_strings.join("\n").as_str(), ""].join("\n")
}

#[derive(Debug, Clone)]
struct InspectorCompleter {
    state: CompletionState,
    command_set: HashSet<String>,
}

impl InspectorCompleter {
    fn new(state: CompletionState) -> Self {
        Self {
            state,
            command_set: COMMANDS
                .iter()
                .map(|command| command.name.to_string())
                .collect(),
        }
    }

    fn suggestions_for(&self, line: &str, pos: usize) -> Vec<String> {
        let prefix_line = &line[..pos.min(line.len())];
        let words = shlex::split(prefix_line).unwrap_or_default();
        let ends_with_space = prefix_line.ends_with(char::is_whitespace);

        match words.as_slice() {
            [] => COMMANDS
                .iter()
                .map(|command| command.name.to_string())
                .collect(),
            [first] if !ends_with_space => COMMANDS
                .iter()
                .filter(|command| command.name.starts_with(first.as_str()))
                .map(|command| command.name.to_string())
                .collect(),
            [command] if ends_with_space && command == "tool" => self.state.tool_names.clone(),
            [command, partial] if command == "tool" && !ends_with_space => self
                .state
                .tool_names
                .iter()
                .filter(|name| name.starts_with(partial.as_str()))
                .cloned()
                .collect(),
            [command, tool, partial] if command == "tool" && !ends_with_space => {
                let args = self.state.arg_names(tool);
                args.into_iter()
                    .map(|arg| format!("{arg}="))
                    .filter(|arg| arg.starts_with(partial.as_str()))
                    .collect()
            }
            [command, tool, ..] if command == "tool" => self
                .state
                .arg_names(tool)
                .into_iter()
                .map(|arg| format!("{arg}="))
                .collect(),
            [command] if ends_with_space && command == "resource" => {
                self.state.resource_uris.clone()
            }
            [command, partial] if command == "resource" && !ends_with_space => self
                .state
                .resource_uris
                .iter()
                .filter(|uri| uri.starts_with(partial.as_str()))
                .cloned()
                .collect(),
            [command] if ends_with_space && command == "schema" => self.state.tool_names.clone(),
            [command, partial] if command == "schema" && !ends_with_space => self
                .state
                .tool_names
                .iter()
                .filter(|name| name.starts_with(partial.as_str()))
                .cloned()
                .collect(),
            [command] if ends_with_space && self.command_set.contains(command) => Vec::new(),
            _ => Vec::new(),
        }
    }
}

impl Completer for InspectorCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let start = completion_start(line, pos);
        self.suggestions_for(line, pos)
            .into_iter()
            .map(|value| Suggestion {
                value,
                span: Span::new(start, pos),
                append_whitespace: true,
                style: Some(Style::new().fg(Color::Cyan)),
                ..Suggestion::default()
            })
            .collect()
    }
}

fn completion_start(line: &str, pos: usize) -> usize {
    line[..pos.min(line.len())]
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0)
}

fn schema_arg_names(schema: &Value) -> Vec<String> {
    let mut args = BTreeSet::new();
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for name in properties.keys() {
            args.insert(name.clone());
        }
    }
    args.into_iter().collect()
}

fn default_history_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("mcp-cli").join("history"))
}

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ReplCommand, parse_arguments, parse_command};

    #[test]
    fn parses_key_value_arguments() {
        let args = parse_arguments(&[
            "name=foo".to_string(),
            "limit=3".to_string(),
            "nested.flag=true".to_string(),
        ])
        .unwrap();

        assert_eq!(
            args,
            json!({
                "name": "foo",
                "limit": 3,
                "nested": {"flag": true}
            })
        );
    }

    #[test]
    fn parses_json_tool_command() {
        let command = parse_command(r#"tool lookup --json '{"q":"rust"}'"#).unwrap();
        assert_eq!(
            command,
            ReplCommand::Tool {
                name: "lookup".to_string(),
                arguments: json!({"q": "rust"})
            }
        );
    }

    #[test]
    fn parses_raw_command() {
        let command = parse_command(r#"raw tools/list '{"cursor":null}'"#).unwrap();
        assert_eq!(
            command,
            ReplCommand::Raw {
                method: "tools/list".to_string(),
                params: Some(json!({"cursor": null}))
            }
        );
    }
}
