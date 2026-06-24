use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CODEX_USAGE_MODULE: &str = "custom/codex-usage";
const WAYBAR_FORMAT: &str = "<span font_family=\"bootstrap-icons\" rise=\"1200\" color=\"#5f78ff\">{icon_plain}</span> {5h_pct}% <span color=\"#5f78ff\">󰥔</span> {5h_reset} <span font_family=\"bootstrap-icons\" rise=\"1200\" color=\"#5f78ff\">{icon_plain}</span> {7d_pct} <span color=\"#5f78ff\">{time_icon_plain}</span> {7d_reset}";

fn waybar_exec() -> String {
    format!(
        "{} --waybar --format '{}'",
        shell_quote(&codex_switch_command()),
        WAYBAR_FORMAT
    )
}

fn codex_switch_command() -> String {
    let Ok(path) = std::env::current_exe() else {
        return "codex-switch".to_string();
    };
    if path.file_name().and_then(|name| name.to_str()) == Some("codex-switch") {
        return path.display().to_string();
    }
    "codex-switch".to_string()
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '+'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

pub fn install_waybar_config() {
    let home = std::env::var("HOME").unwrap_or_else(|_| crate::data::die("HOME not set"));
    let waybar_dir = PathBuf::from(home).join(".config").join("waybar");
    let common_path = waybar_dir.join("common.jsonc");
    let config_path = waybar_dir.join("config.jsonc");

    let mut changed = false;
    if common_path.exists() {
        changed |= update_file(&common_path, ensure_common_module);
        changed |= update_file(&config_path, ensure_module_in_layouts);
    } else {
        changed |= update_file(&config_path, ensure_inline_waybar_config);
    }

    if changed {
        println!("Installed codex-switch Waybar config");
    } else {
        println!("codex-switch Waybar config already installed");
    }
    println!("  module: {}", CODEX_USAGE_MODULE);
    println!("  common: {}", common_path.display());
    println!("  config: {}", config_path.display());
}

fn update_file(path: &Path, update: fn(&str) -> String) -> bool {
    let original = std::fs::read_to_string(path)
        .unwrap_or_else(|e| crate::data::die(&format!("failed to read {}: {}", path.display(), e)));
    let updated = update(&original);
    if updated == original {
        return false;
    }

    backup_file(path, &original);
    std::fs::write(path, updated).unwrap_or_else(|e| {
        crate::data::die(&format!("failed to write {}: {}", path.display(), e))
    });
    true
}

fn backup_file(path: &Path, content: &str) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let backup = path.with_extension(format!("jsonc.bak.{}", timestamp));
    std::fs::write(&backup, content).unwrap_or_else(|e| {
        crate::data::die(&format!(
            "failed to write backup {}: {}",
            backup.display(),
            e
        ))
    });
}

fn ensure_common_module(input: &str) -> String {
    if let Some((start, end)) = find_named_object(input, CODEX_USAGE_MODULE) {
        let block = &input[start..end];
        let updated_block = ensure_module_properties(block);
        if updated_block == block {
            return input.to_string();
        }
        return format!("{}{}{}", &input[..start], updated_block, &input[end..]);
    }

    let module = format!(
        "  \"{}\": {{\n    \"exec\": {},\n    \"return-type\": \"json\",\n    \"on-click\": \"/usr/bin/pkill -RTMIN+11 waybar\",\n    \"signal\": 11,\n    \"interval\": 120\n  }}",
        CODEX_USAGE_MODULE,
        json_string(&waybar_exec())
    );

    if let Some(pos) = input.rfind('}') {
        let separator = if input[..pos].trim_end().ends_with('{') {
            "\n"
        } else {
            ",\n"
        };
        return format!("{}{}{}{}", &input[..pos], separator, module, &input[pos..]);
    }

    input.to_string()
}

fn ensure_module_properties(block: &str) -> String {
    let mut updated = set_or_insert_property(block, "exec", &json_string(&waybar_exec()));
    updated = set_or_insert_property(&updated, "return-type", &json_string("json"));
    updated
}

fn set_or_insert_property(block: &str, key: &str, value: &str) -> String {
    let pattern = format!("\"{}\"", key);
    let lines: Vec<&str> = block.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with(&pattern) {
            let indent_len = line.len() - line.trim_start().len();
            let indent = &line[..indent_len];
            let comma = if line.trim_end().ends_with(',') {
                ","
            } else {
                ""
            };
            let mut new_lines: Vec<String> = lines.iter().map(|line| (*line).to_string()).collect();
            new_lines[index] = format!("{}\"{}\": {}{}", indent, key, value, comma);
            return new_lines.join("\n");
        }
    }

    let Some(close_pos) = block.rfind('}') else {
        return block.to_string();
    };
    let before = block[..close_pos].trim_end();
    let separator = if before.ends_with('{') { "\n" } else { ",\n" };
    format!(
        "{}{}    \"{}\": {}\n{}",
        before,
        separator,
        key,
        value,
        &block[close_pos..]
    )
}

fn ensure_inline_waybar_config(input: &str) -> String {
    let with_layouts = ensure_module_in_layouts(input);
    if with_layouts.trim_start().starts_with('[') {
        ensure_module_in_top_level_array_objects(&with_layouts)
    } else {
        ensure_common_module(&with_layouts)
    }
}

fn ensure_module_in_top_level_array_objects(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 512);
    let mut rest_start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut array_depth = 0usize;

    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '[' if !in_string => array_depth += 1,
            ']' if !in_string => array_depth = array_depth.saturating_sub(1),
            '{' if !in_string && array_depth == 1 => {
                output.push_str(&input[rest_start..index]);
                if let Some(end_rel) = find_matching_brace(&input[index + 1..]) {
                    let end = index + end_rel + 2;
                    output.push_str(&ensure_common_module(&input[index..end]));
                    rest_start = end;
                }
            }
            _ => {}
        }
    }

    output.push_str(&input[rest_start..]);
    output
}

fn ensure_module_in_layouts(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 64);
    let mut rest = input;

    while let Some(pos) = rest.find("\"modules-right\"") {
        output.push_str(&rest[..pos]);
        let after_key = &rest[pos..];
        let Some(array_start_rel) = after_key.find('[') else {
            output.push_str(after_key);
            return output;
        };
        let array_start = pos + array_start_rel;
        output.push_str(&rest[pos..=array_start]);

        let after_array = &rest[array_start + 1..];
        let Some(array_end_rel) = find_matching_bracket(after_array) else {
            output.push_str(after_array);
            return output;
        };
        let array_body = &after_array[..array_end_rel];
        output.push_str(&ensure_module_in_array(array_body));
        output.push(']');
        rest = &after_array[array_end_rel + 1..];
    }

    output.push_str(rest);
    output
}

fn ensure_module_in_array(array_body: &str) -> String {
    if array_body.contains(&format!("\"{}\"", CODEX_USAGE_MODULE)) {
        return array_body.to_string();
    }

    if !array_body.contains('\n') {
        let trimmed = array_body.trim();
        if trimmed.is_empty() {
            return format!("\n      \"{}\"\n    ", CODEX_USAGE_MODULE);
        }
        let comma = if trimmed.ends_with(',') { "" } else { "," };
        return format!("{}{} \"{}\"", array_body, comma, CODEX_USAGE_MODULE);
    }

    let lines: Vec<&str> = array_body.lines().collect();
    let insert_line = "      \"custom/codex-usage\",";
    let mut output = Vec::with_capacity(lines.len() + 1);
    let mut inserted = false;

    for line in lines {
        output.push(line.to_string());
        if !inserted && line.contains("\"pulseaudio\"") {
            output.push(insert_line.to_string());
            inserted = true;
        }
    }

    if !inserted {
        if output.is_empty() {
            output.push(String::new());
            output.push(insert_line.trim_end_matches(',').to_string());
        } else {
            output.insert(1.min(output.len()), insert_line.to_string());
        }
    }

    output.join("\n")
}

fn find_named_object(input: &str, key: &str) -> Option<(usize, usize)> {
    let key_pos = input.find(&format!("\"{}\"", key))?;
    let object_start = input[key_pos..].find('{')? + key_pos;
    let object_end = find_matching_brace(&input[object_start + 1..])? + object_start + 2;
    Some((key_pos, object_end))
}

fn find_matching_brace(input: &str) -> Option<usize> {
    find_matching_delimiter(input, '{', '}')
}

fn find_matching_bracket(input: &str) -> Option<usize> {
    find_matching_delimiter(input, '[', ']')
}

fn find_matching_delimiter(input: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 1usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            value if !in_string && value == open => depth += 1,
            value if !in_string && value == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }

    None
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::{ensure_common_module, ensure_inline_waybar_config, ensure_module_in_layouts};

    #[test]
    fn updates_existing_common_module_exec_without_dropping_user_keys() {
        let input = r##"{
  "custom/codex-usage": {
    "exec": "old",
    "return-type": "json",
    "interval": 999
  }
}
"##;

        let updated = ensure_common_module(input);
        assert!(updated.contains("codex-switch --waybar"));
        assert!(updated.contains("\"interval\": 999"));
    }

    #[test]
    fn adds_common_module_when_missing() {
        let input = "{\n  \"clock\": {\"tooltip\": false}\n}\n";
        let updated = ensure_common_module(input);
        assert!(updated.contains("\"custom/codex-usage\""));
        assert!(updated.contains("\"return-type\": \"json\""));
        assert!(updated.contains("\"clock\""));
    }

    #[test]
    fn inserts_module_after_pulseaudio_in_each_modules_right_array() {
        let input = r#"[
  {
    "modules-right": [
      "network",
      "pulseaudio",
      "cpu"
    ]
  },
  {
    "modules-right": [
      "pulseaudio"
    ]
  }
]
"#;

        let updated = ensure_module_in_layouts(input);
        assert_eq!(updated.matches("custom/codex-usage").count(), 2);
        assert!(updated.contains("\"pulseaudio\",\n      \"custom/codex-usage\","));
    }

    #[test]
    fn inline_install_supports_standard_single_object_config() {
        let input = r#"{
  "modules-right": [
    "pulseaudio",
    "cpu"
  ]
}
"#;

        let updated = ensure_inline_waybar_config(input);
        assert!(updated.contains("\"custom/codex-usage\""));
        assert!(updated.contains("codex-switch --waybar"));
        assert!(updated.contains("\"pulseaudio\",\n      \"custom/codex-usage\","));
    }

    #[test]
    fn inline_install_supports_standard_multi_bar_array_config() {
        let input = r#"[
  {
    "modules-right": ["pulseaudio"]
  },
  {
    "modules-right": ["network"]
  }
]
"#;

        let updated = ensure_inline_waybar_config(input);
        assert_eq!(updated.matches("codex-switch --waybar").count(), 2);
        assert_eq!(updated.matches("custom/codex-usage").count(), 4);
    }
}
