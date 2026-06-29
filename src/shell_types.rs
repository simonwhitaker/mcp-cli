pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage_hint: Option<&'static str>,
    pub description: &'static str,
}
