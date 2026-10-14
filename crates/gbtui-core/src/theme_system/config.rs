use std::error::Error;
use std::fmt;
use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};
use toml::Value;
use toml::map::Map;

use super::{BackgroundPolicy, ThemeColor, ThemeDefinition, ThemeKind, ThemeRole, ThemeState};

/// A strict theme configuration error with source-line context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeConfigError {
    line: usize,
    message: String,
}

impl ThemeConfigError {
    fn new(source: &str, needle: &str, message: impl Into<String>) -> Self {
        let line = source
            .lines()
            .position(|line| line.contains(needle))
            .map_or(1, |line| line + 1);
        Self {
            line,
            message: message.into(),
        }
    }

    fn root(message: impl Into<String>) -> Self {
        Self {
            line: 1,
            message: message.into(),
        }
    }
}

impl fmt::Display for ThemeConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

impl Error for ThemeConfigError {}

impl ThemeDefinition {
    /// Parses the strict, in-memory TOML theme schema (`name`, `kind`,
    /// optional `background`, `[colors]`, and `[styles.role.state]`). Core
    /// performs no file IO.
    pub fn from_toml(source: &str) -> Result<Self, ThemeConfigError> {
        let value = source
            .parse::<Value>()
            .map_err(|error| ThemeConfigError::root(format!("invalid TOML: {error}")))?;
        let root = value
            .as_table()
            .ok_or_else(|| ThemeConfigError::root("theme document must be a TOML table"))?;
        reject_unknown(
            source,
            root,
            &["name", "kind", "background", "colors", "styles"],
            "top-level",
        )?;

        let name = required_string(source, root, "name")?;
        if name.is_empty() {
            return Err(ThemeConfigError::new(
                source,
                "name",
                "`name` must not be empty",
            ));
        }
        let kind = parse_kind(required_string(source, root, "kind")?, source)?;
        let background = match root.get("background") {
            Some(value) => parse_background(string_value(source, "background", value)?, source)?,
            None => BackgroundPolicy::Solid,
        };

        let mut definition = ThemeDefinition::new(name, kind).with_background(background);
        if let Some(colors) = root.get("colors") {
            let colors = table_value(source, "colors", colors)?;
            for (name, value) in colors {
                let color = ThemeColor::from_name(name).ok_or_else(|| {
                    ThemeConfigError::new(source, name, format!("unknown palette color `{name}`"))
                })?;
                let value = string_value(source, name, value)?;
                definition = definition.with_color(color, parse_color(value, source, name)?);
            }
        }

        if let Some(styles) = root.get("styles") {
            let styles = table_value(source, "styles", styles)?;
            for (role_name, states) in styles {
                let role = ThemeRole::from_name(role_name).ok_or_else(|| {
                    ThemeConfigError::new(
                        source,
                        role_name,
                        format!("unknown style role `{role_name}`"),
                    )
                })?;
                let states = table_value(source, role_name, states)?;
                for (state_name, style) in states {
                    let state = ThemeState::from_name(state_name).ok_or_else(|| {
                        ThemeConfigError::new(
                            source,
                            state_name,
                            format!("unknown style state `{state_name}`"),
                        )
                    })?;
                    definition =
                        definition.with_style(role, state, parse_style(source, state_name, style)?);
                }
            }
        }

        Ok(definition)
    }
}

fn reject_unknown(
    source: &str,
    table: &Map<String, Value>,
    allowed: &[&str],
    scope: &str,
) -> Result<(), ThemeConfigError> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ThemeConfigError::new(
                source,
                key,
                format!("unknown {scope} key `{key}`"),
            ));
        }
    }
    Ok(())
}

fn required_string<'a>(
    source: &str,
    table: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, ThemeConfigError> {
    let value = table
        .get(key)
        .ok_or_else(|| ThemeConfigError::new(source, key, format!("missing required `{key}`")))?;
    string_value(source, key, value)
}

fn string_value<'a>(
    source: &str,
    key: &str,
    value: &'a Value,
) -> Result<&'a str, ThemeConfigError> {
    value
        .as_str()
        .ok_or_else(|| ThemeConfigError::new(source, key, format!("`{key}` must be a string")))
}

fn table_value<'a>(
    source: &str,
    key: &str,
    value: &'a Value,
) -> Result<&'a Map<String, Value>, ThemeConfigError> {
    value
        .as_table()
        .ok_or_else(|| ThemeConfigError::new(source, key, format!("`{key}` must be a table")))
}

fn parse_kind(value: &str, source: &str) -> Result<ThemeKind, ThemeConfigError> {
    match value {
        "night" => Ok(ThemeKind::Night),
        "day" => Ok(ThemeKind::Day),
        "auto" => Ok(ThemeKind::Auto),
        _ => Err(ThemeConfigError::new(
            source,
            "kind",
            "`kind` must be `auto`, `night`, or `day`",
        )),
    }
}

fn parse_background(value: &str, source: &str) -> Result<BackgroundPolicy, ThemeConfigError> {
    match value {
        "solid" => Ok(BackgroundPolicy::Solid),
        "terminal" => Ok(BackgroundPolicy::TerminalDefault),
        "transparent" => Ok(BackgroundPolicy::Transparent),
        _ => Err(ThemeConfigError::new(
            source,
            "background",
            "`background` must be `solid`, `terminal`, or `transparent`",
        )),
    }
}

fn parse_style(source: &str, state_name: &str, value: &Value) -> Result<Style, ThemeConfigError> {
    let style = table_value(source, state_name, value)?;
    reject_unknown(source, style, &["fg", "bg", "modifiers"], "style")?;

    let mut resolved = Style::default();
    if let Some(value) = style.get("fg") {
        resolved = resolved.fg(parse_color(
            string_value(source, "fg", value)?,
            source,
            "fg",
        )?);
    }
    if let Some(value) = style.get("bg") {
        resolved = resolved.bg(parse_color(
            string_value(source, "bg", value)?,
            source,
            "bg",
        )?);
    }
    if let Some(value) = style.get("modifiers") {
        let modifiers = value.as_array().ok_or_else(|| {
            ThemeConfigError::new(
                source,
                "modifiers",
                "`modifiers` must be an array of strings",
            )
        })?;
        let mut parsed = Modifier::empty();
        for value in modifiers {
            let modifier = string_value(source, "modifiers", value)?;
            parsed |= parse_modifier(modifier).ok_or_else(|| {
                ThemeConfigError::new(
                    source,
                    modifier,
                    format!("unknown style modifier `{modifier}`"),
                )
            })?;
        }
        resolved = resolved.add_modifier(parsed);
    }
    Ok(resolved)
}

fn parse_color(value: &str, source: &str, key: &str) -> Result<Color, ThemeConfigError> {
    let value = value.trim();
    let parsed = if let Some(index) = value.strip_prefix("index:") {
        index.parse::<u8>().ok().map(Color::Indexed)
    } else if let Some(ansi) = value
        .strip_prefix("ansi:")
        .or_else(|| value.strip_prefix("ansi16:"))
    {
        Color::from_str(ansi).ok()
    } else {
        Color::from_str(value).ok()
    };
    parsed.ok_or_else(|| {
        ThemeConfigError::new(
            source,
            key,
            format!("`{key}` must be a hex, ANSI, reset, or indexed color"),
        )
    })
}

fn parse_modifier(value: &str) -> Option<Modifier> {
    match value {
        "bold" => Some(Modifier::BOLD),
        "dim" => Some(Modifier::DIM),
        "italic" => Some(Modifier::ITALIC),
        "underlined" | "underline" => Some(Modifier::UNDERLINED),
        "slow_blink" => Some(Modifier::SLOW_BLINK),
        "rapid_blink" => Some(Modifier::RAPID_BLINK),
        "reversed" | "reverse" => Some(Modifier::REVERSED),
        "hidden" => Some(Modifier::HIDDEN),
        "crossed_out" | "strikethrough" => Some(Modifier::CROSSED_OUT),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier};

    use super::*;
    use crate::{ThemeContext, ThemeResolver};

    #[test]
    fn parser_applies_color_and_state_style_overrides() {
        let definition = ThemeDefinition::from_toml(
            r##"
name = "configurable"
kind = "night"
background = "transparent"

[colors]
accent = "#112233"

[styles.button.focused]
fg = "lightgreen"
bg = "index:17"
modifiers = ["bold", "underlined"]
"##,
        )
        .expect("valid theme configuration");
        let resolved = ThemeResolver::new(definition, ThemeContext::default());
        let style = resolved
            .resolved()
            .style(ThemeRole::Button, ThemeState::Focused);

        assert_eq!(
            resolved.resolved().background(),
            BackgroundPolicy::Transparent
        );
        assert_eq!(
            resolved.resolved().palette().accent,
            Color::Rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(style.fg, Some(Color::LightGreen));
        assert_eq!(style.bg, Some(Color::Indexed(17)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn parser_rejects_unknown_keys_with_line_context() {
        let error =
            ThemeDefinition::from_toml("name = \"strict\"\nkind = \"night\"\nunexpected = true\n")
                .expect_err("unknown keys are rejected");

        assert!(error.to_string().contains("line 3"));
        assert!(
            error
                .to_string()
                .contains("unknown top-level key `unexpected`")
        );
    }

    #[test]
    fn parser_rejects_unknown_color_role_and_state_names() {
        let cases = [
            (
                "name = \"strict\"\nkind = \"night\"\n[colors]\nnot_a_color = \"#112233\"\n",
                "unknown palette color `not_a_color`",
            ),
            (
                "name = \"strict\"\nkind = \"night\"\n[styles.not_a_role.normal]\nfg = \"red\"\n",
                "unknown style role `not_a_role`",
            ),
            (
                "name = \"strict\"\nkind = \"night\"\n[styles.text.not_a_state]\nfg = \"red\"\n",
                "unknown style state `not_a_state`",
            ),
        ];

        for (source, expected) in cases {
            let error = ThemeDefinition::from_toml(source).expect_err("schema name is invalid");
            assert!(error.to_string().contains(expected));
        }
    }
}
