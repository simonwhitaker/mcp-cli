use std::fmt::Write;

use nu_ansi_term::{Color, Style};
use rmcp::model::{
    CallToolResult, Content, RawContent, RawResource, Resource, ResourceContents, ServerInfo, Tool,
};
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy)]
pub struct Formatter {
    color: bool,
    json: bool,
}

impl Formatter {
    pub const fn new(color: bool, json: bool) -> Self {
        Self { color, json }
    }

    pub const fn json_mode(&self) -> bool {
        self.json
    }

    pub fn server_intro(&self, info: &ServerInfo, tool_count: usize) -> String {
        if self.json {
            return self.json_value(&serde_json::json!({
                "server": info.server_info,
                "protocolVersion": info.protocol_version,
                "capabilities": info.capabilities,
                "toolCount": tool_count,
            }));
        }

        let name = &info.server_info.name;
        let version = &info.server_info.version;
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{} {} {}",
            self.accent("connected"),
            name,
            self.dim(&format!("v{version}"))
        );
        let _ = writeln!(
            output,
            "{} {}",
            self.label("protocol"),
            self.json_value(&info.protocol_version)
        );
        let _ = writeln!(output, "{} {tool_count}", self.label("tools"));
        output
    }

    pub fn server_info(&self, info: &ServerInfo) -> String {
        self.json_value(info)
    }

    pub fn tools(&self, tools: &[Tool]) -> String {
        if self.json {
            return self.json_value(tools);
        }
        if tools.is_empty() {
            return "No tools advertised by this server.\n".to_string();
        }

        let width = self.width(tools, |tool| tool.name.len(), None, None);

        let mut output = String::new();
        for tool in tools {
            let description = tool.description.as_deref().unwrap_or("");
            let schema = schema_summary(&Value::Object((*tool.input_schema).clone()));
            let _ = writeln!(
                output,
                "{:<width$}  {} {}",
                self.accent(tool.name.as_ref()),
                description,
                self.dim(&schema),
                width = width + ansi_extra(&self.accent(tool.name.as_ref()), tool.name.len())
            );
        }
        output
    }

    pub fn resources(&self, resources: &[Resource]) -> String {
        if self.json {
            return self.json_value(resources);
        }
        if resources.is_empty() {
            return "No resources advertised by this server.\n".to_string();
        }

        let width = self.width(resources, |resource| resource.uri.len(), None, None);
        let mut output = String::new();
        for resource in resources {
            let description = resource.description.as_deref().unwrap_or("");
            let _ = writeln!(
                output,
                "{:<width$}  {}",
                self.accent(resource.uri.as_ref()),
                description,
                width = width + ansi_extra(&self.accent(resource.uri.as_ref()), resource.uri.len())
            );
        }
        output
    }

    fn width<T, F>(
        &self,
        items: &[T],
        item_width_fn: F,
        min: Option<usize>,
        max: Option<usize>,
    ) -> usize
    where
        F: Fn(&T) -> usize,
    {
        let min = min.unwrap_or(4);
        let max = max.unwrap_or(36);
        items
            .iter()
            .map(item_width_fn)
            .max()
            .unwrap_or(min)
            .clamp(min, max)
    }

    pub fn schema(&self, tool: &Tool) -> String {
        self.json_value(&Value::Object((*tool.input_schema).clone()))
    }

    pub fn tool_result(&self, result: &CallToolResult) -> String {
        if self.json {
            return self.json_value(result);
        }

        let mut output = String::new();
        if result.is_error == Some(true) {
            let _ = writeln!(output, "{}", self.error("tool returned an error result"));
        }

        if let Some(structured) = &result.structured_content {
            let _ = writeln!(output, "{}", self.label("structured"));
            let _ = writeln!(output, "{}", self.json_value(structured));
        }

        if result.content.is_empty() {
            if result.structured_content.is_none() {
                output.push_str("(empty result)\n");
            }
            return output;
        }

        for (index, content) in result.content.iter().enumerate() {
            if result.content.len() > 1 {
                let _ = writeln!(output, "{}", self.label(&format!("content[{index}]")));
            }
            output.push_str(&self.content(content));
            if !output.ends_with('\n') {
                output.push('\n');
            }
        }

        output
    }

    pub fn json_value<T: Serialize + ?Sized>(&self, value: &T) -> String {
        serde_json::to_string_pretty(value)
            .unwrap_or_else(|error| format!("failed to serialize value: {error}"))
    }

    pub fn error(&self, message: &str) -> String {
        if self.color {
            Style::new()
                .fg(Color::Red)
                .bold()
                .paint(message)
                .to_string()
        } else {
            message.to_string()
        }
    }

    pub fn accent(&self, message: &str) -> String {
        if self.color {
            Style::new()
                .fg(Color::Cyan)
                .bold()
                .paint(message)
                .to_string()
        } else {
            message.to_string()
        }
    }

    pub fn label(&self, message: &str) -> String {
        if self.color {
            Style::new()
                .fg(Color::Purple)
                .bold()
                .paint(message)
                .to_string()
        } else {
            message.to_string()
        }
    }

    pub fn dim(&self, message: &str) -> String {
        if self.color {
            Style::new().dimmed().paint(message).to_string()
        } else {
            message.to_string()
        }
    }

    fn content(&self, content: &Content) -> String {
        match &content.raw {
            RawContent::Text(text) => format_text_or_json(&text.text, self),
            RawContent::Image(image) => format!(
                "{} {} bytes base64\n",
                self.label("image"),
                image.data.len()
            ),
            RawContent::Audio(audio) => format!(
                "{} {} bytes base64\n",
                self.label("audio"),
                audio.data.len()
            ),
            RawContent::Resource(resource) => self.resource(&resource.resource),
            RawContent::ResourceLink(resource) => self.resource_link(resource),
        }
    }

    pub fn resource(&self, resource: &ResourceContents) -> String {
        match resource {
            ResourceContents::TextResourceContents {
                uri,
                mime_type,
                text,
                ..
            } => {
                let mut output = format!(
                    "{} {} {}\n",
                    self.label("resource"),
                    uri,
                    self.dim(mime_type.as_deref().unwrap_or("text"))
                );
                output.push_str(&format_text_or_json(text, self));
                output
            }
            ResourceContents::BlobResourceContents {
                uri,
                mime_type,
                blob,
                ..
            } => format!(
                "{} {} {} {} bytes base64\n",
                self.label("resource"),
                uri,
                self.dim(mime_type.as_deref().unwrap_or("blob")),
                blob.len()
            ),
        }
    }

    fn resource_link(&self, resource: &RawResource) -> String {
        format!(
            "{} {} {}\n",
            self.label("resource link"),
            resource.uri,
            self.dim(resource.mime_type.as_deref().unwrap_or(""))
        )
    }
}

pub fn schema_summary(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return String::new();
    };
    let required = object
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .map(schema_properties)
        .unwrap_or_default();

    if properties.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    for (name, ty) in properties {
        let required_marker = if required.contains(&name.as_str()) {
            ""
        } else {
            "?"
        };
        parts.push(format!("{name}{required_marker}:{ty}"));
    }
    format!("({})", parts.join(", "))
}

fn schema_properties(properties: &Map<String, Value>) -> Vec<(String, String)> {
    let mut entries = properties
        .iter()
        .map(|(name, schema)| {
            let ty = schema
                .get("type")
                .and_then(Value::as_str)
                .or_else(|| {
                    schema
                        .get("anyOf")
                        .and_then(Value::as_array)
                        .and_then(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.get("type")?.as_str())
                                .next()
                        })
                })
                .unwrap_or("value");
            (name.clone(), ty.to_string())
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn format_text_or_json(text: &str, formatter: &Formatter) -> String {
    let trimmed = text.trim();
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Ok(value) = serde_json::from_str::<Value>(trimmed)
    {
        return format!("{}\n", formatter.json_value(&value));
    }
    format!("{text}\n")
}

fn ansi_extra(styled: &str, plain_len: usize) -> usize {
    styled.len().saturating_sub(plain_len)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::schema_summary;

    #[test]
    fn summarizes_schema_properties() {
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string"},
                "limit": {"type": "integer"}
            }
        });

        assert_eq!(schema_summary(&schema), "(limit?:integer, name:string)");
    }
}
