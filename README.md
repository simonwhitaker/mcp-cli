# MCP Inspector CLI

`mcp` is an interactive terminal inspector for Model Context Protocol servers.
It connects to an MCP server, lists the advertised tools, and gives you a shell
for calling tools and inspecting schemas, resources and prompts.

## Build

```sh
cargo build
```

Run from the repository with:

```sh
cargo run -- [options] <target>
```

After installing or copying the built binary, use:

```sh
mcp [options] <target>
```

## Connect to a Server

### Stdio

Pass the command that starts a stdio MCP server:

```sh
mcp coral mcp-stdio
```

With `cargo run`, put the server command after `--`:

```sh
cargo run -- coral mcp-stdio
```

If the stdio server command has arguments that start with `-`, put `--` before
the server command so `mcp` does not parse them as its own options:

```sh
mcp -- server --server-flag
```

### Streamable HTTP

Pass an `http://` or `https://` endpoint URL as the target:

```sh
mcp http://localhost:8000/mcp
```

Add headers with repeated `--header NAME=VALUE` options:

```sh
mcp http://localhost:8000/mcp --header X-Tenant=dev
```

For bearer auth, avoid putting the token itself on the command line. Use one of
these token sources instead.

Read from an environment variable:

```sh
mcp https://example.com/mcp --bearer-token-env EXAMPLE_MCP_TOKEN
```

The variable name is safe to keep in shell history. The variable value still has
the normal exposure tradeoffs of environment variables.

Read from a file:

```sh
mcp https://example.com/mcp --bearer-token-file ~/.config/mcp-cli/example.token
```

The file should contain only the token. Trailing whitespace is trimmed. Set file
permissions appropriately, for example with `chmod 600`.

Prompt without echoing input:

```sh
mcp https://example.com/mcp --bearer-token-prompt
```

The first target argument determines the transport. If it starts with
`http://` or `https://`, `mcp` uses Streamable HTTP. Otherwise, it treats the
target as a stdio command and arguments.

## Options

```text
--debug                 Print protocol and diagnostic detail
--json                  Emit command output as JSON where possible
--history <PATH>        Path to the shell history file
--no-color              Disable ANSI colors
--header <NAME=VALUE>   Send an HTTP header with every request
--bearer-token-env VAR  Read bearer token from an environment variable
--bearer-token-file PATH
                        Read bearer token from a file
--bearer-token-prompt   Prompt for bearer token without echoing input
```

`--header` and bearer token options are only valid with an HTTP(S) target. Use
only one bearer token source at a time.

## Shell Commands

Once connected, the prompt is `mcp:<server-name>`.

```text
help                          Show command help
info                          Show server metadata and capabilities
tools                         List available tools
schema TOOL                   Pretty-print a tool input schema
tool TOOL key=value ...       Call a tool
tool TOOL --json '{...}'      Call a tool with raw JSON arguments
tool TOOL @args.json          Call a tool with JSON arguments from a file
resources                     List available resources
resource URI                  Read a resource
prompts                       List available prompts
prompt NAME key=value ...     Render a prompt
raw METHOD [JSON]             Send a raw MCP request
reload                        Refresh tool metadata and completions
quit | exit                   Close the session
```

Ctrl-C and Ctrl-D also exit.

## Calling Tools

For simple arguments, use `key=value`:

```text
tool search query=rust limit=5
```

Values are converted to JSON scalars when possible:

```text
tool example enabled=true count=3 missing=null
```

Use dotted keys for nested objects:

```text
tool lookup user.name=simon user.active=true
```

Use `--json` for complex inputs:

```text
tool create --json '{"name":"demo","tags":["mcp","debug"]}'
```

Use `@file` to load arguments from a JSON file:

```text
tool create @args.json
```

The file must contain a JSON object.

## Rendering Prompts

`prompts` lists the prompts a server advertises, with their arguments. A `?`
marks an optional argument:

```text
args-prompt   A prompt with two arguments (city, state?)
```

`prompt NAME` calls `prompts/get` and prints the messages the server returns:

```text
prompt args-prompt city=Bristol
```

MCP prompt arguments are strings, so values are passed through as written
rather than being converted to JSON scalars the way tool arguments are. To send
anything else, use `--json` or `@file`:

```text
prompt args-prompt --json '{"city":"Bristol"}'
prompt args-prompt @args.json
```

Argument names complete on tab, and the server validates required arguments.

## Completion and History

Tab completion is enabled for:

- Shell commands
- tool names
- tool argument names derived from each tool input schema

Run `reload` after a server changes its advertised tools.

History is stored in the platform data directory by default. Use `--history` to
choose a specific file:

```sh
mcp --history ./mcp-history coral mcp-stdio
```

## Output

Tool results are rendered for terminal reading:

- text content is printed directly
- JSON-looking text is pretty-printed
- structured tool output is printed as formatted JSON
- binary image/audio/blob content is summarized with metadata

Use `--json` when you want machine-readable command output where supported.

## Troubleshooting

If startup fails, first verify that the MCP server command works by itself or
that the HTTP URL is reachable.

Use `--debug` for more protocol context:

```sh
mcp --debug coral mcp-stdio
```

For HTTP servers, check that you are using the MCP endpoint path, commonly
`/mcp`, not just the service root.

If tool completion looks stale, run:

```text
reload
```
