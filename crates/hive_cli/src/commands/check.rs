use crate::TuiResult;
use crate::tui::icons;
use hive_actor_loader::dependency_resolver::{DependencyResolver, ResolvedActor};
use hive_config::{ActorSource, Config};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug)]
struct StartupAnalysis {
    starting_actors: Vec<String>,
    auto_spawn_actors: Vec<String>,
    effective_startup_actors: Vec<String>,
    has_valid_startup: bool,
}

pub fn show_status(config_path: Option<PathBuf>) -> TuiResult<()> {
    println!("Hive Configuration Status");
    println!("========================");

    // Load config using provided path or default
    let (config, config_file) = if let Some(path) = config_path {
        if !path.exists() {
            println!("Config: {} (not found)", path.display());
            println!("{} Configuration file not found", icons::FAILED_ICON);
            return Ok(());
        }
        let config = hive_config::load_from_path(path.clone())?;
        (config, path)
    } else {
        let config_file = hive_config::get_config_file_path()?;
        if !config_file.exists() {
            println!("Config: {} (not found)", config_file.display());
            println!("{} No configuration file found", icons::FAILED_ICON);
            return Ok(());
        }
        let config = hive_config::load_default_config()?;
        (config, config_file)
    };
    println!(
        "Config: {} {} Valid",
        config_file.display(),
        icons::SUCCESS_ICON
    );
    println!();

    // Use hive_actor_loader to resolve dependencies
    let resolver = DependencyResolver::new();
    let resolved_actors =
        match resolver.resolve_all(config.actors.clone(), config.actor_overrides.clone()) {
            Ok(actors) => actors,
            Err(e) => {
                println!("{} Dependency resolution failed:", icons::FAILED_ICON);
                println!("  {}", e);
                return Ok(());
            }
        };

    // Analyze startup behavior
    let startup_analysis = analyze_startup_behavior(&config, &resolved_actors);

    // Display comprehensive status
    display_status(&config, &resolved_actors, &startup_analysis);

    Ok(())
}

fn analyze_startup_behavior(
    config: &Config,
    resolved_actors: &HashMap<String, ResolvedActor>,
) -> StartupAnalysis {
    let starting_actors = config.starting_actors.clone();

    let auto_spawn_actors: Vec<String> = resolved_actors
        .values()
        .filter(|actor| actor.auto_spawn)
        .map(|actor| actor.logical_name.clone())
        .collect();

    // Calculate effective startup actors by following dependency chains
    let mut effective_startup_actors = Vec::new();
    let mut visited = std::collections::HashSet::new();

    // Add explicitly configured starting actors and their dependencies
    for actor_name in &starting_actors {
        collect_actor_and_dependencies(
            actor_name,
            resolved_actors,
            &mut effective_startup_actors,
            &mut visited,
        );
    }

    // Add auto-spawn actors and their dependencies
    for actor_name in &auto_spawn_actors {
        collect_actor_and_dependencies(
            actor_name,
            resolved_actors,
            &mut effective_startup_actors,
            &mut visited,
        );
    }

    let has_valid_startup = !effective_startup_actors.is_empty();

    StartupAnalysis {
        starting_actors,
        auto_spawn_actors,
        effective_startup_actors,
        has_valid_startup,
    }
}

fn collect_actor_and_dependencies(
    actor_name: &str,
    resolved_actors: &HashMap<String, ResolvedActor>,
    effective_actors: &mut Vec<String>,
    visited: &mut std::collections::HashSet<String>,
) {
    if visited.contains(actor_name) {
        return;
    }
    visited.insert(actor_name.to_string());

    if let Some(actor) = resolved_actors.get(actor_name) {
        // Add this actor to effective startup
        if !effective_actors.contains(&actor.logical_name) {
            effective_actors.push(actor.logical_name.clone());
        }

        // Recursively add dependencies that will be spawned
        for dep_name in &actor.required_spawn_with {
            collect_actor_and_dependencies(dep_name, resolved_actors, effective_actors, visited);
        }
    }
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

fn format_config_compact(config: &toml::Table) -> String {
    if config.is_empty() {
        return "(empty)".to_string();
    }

    // Use proper TOML format with sections
    format_toml_table_proper(config, String::new(), 0)
}

fn format_toml_table_proper(table: &toml::Table, section_prefix: String, _depth: usize) -> String {
    let mut result = Vec::new();

    // First, handle simple key-value pairs (non-table values)
    for (key, value) in table.iter() {
        if !matches!(value, toml::Value::Table(_)) {
            let formatted_value = format_toml_value_for_display(value);
            result.push(format!("{} = {}", key, formatted_value));
        }
    }

    // Then handle table sections
    for (key, value) in table.iter() {
        if let toml::Value::Table(nested_table) = value {
            if nested_table.is_empty() {
                continue;
            }

            let section_name = if section_prefix.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", section_prefix, key)
            };

            // Add section header
            result.push(format!("[{}]", section_name));

            // Recursively format nested table
            let nested_content = format_toml_table_proper(nested_table, section_name, 0);
            if !nested_content.is_empty() {
                result.push(nested_content);
            }
        }
    }

    result.join("\n")
}

fn format_toml_value_for_display(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => {
            let truncated = truncate_string(s, 100); // Longer limit for analysis tool
            format!("\"{}\"", truncated)
        }
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) => {
            if arr.len() <= 5 {
                let items: Vec<String> = arr.iter().map(format_toml_value_for_display).collect();
                format!("[{}]", items.join(", "))
            } else {
                format!("[{} items...]", arr.len())
            }
        }
        toml::Value::Table(_) => "(table)".to_string(), // Shouldn't happen in this context
        _ => "...".to_string(),
    }
}

fn display_status(
    _config: &Config,
    resolved_actors: &HashMap<String, ResolvedActor>,
    analysis: &StartupAnalysis,
) {
    // Display startup behavior
    println!("Startup Behavior:");
    if !analysis.starting_actors.is_empty() {
        println!(
            "├─ User-configured starting actors: {}",
            analysis.starting_actors.join(", ")
        );
    } else {
        println!("├─ User-configured starting actors: (none)");
    }

    if !analysis.auto_spawn_actors.is_empty() {
        println!(
            "├─ Auto-spawn actors: {}",
            analysis.auto_spawn_actors.join(", ")
        );
    } else {
        println!("├─ Auto-spawn actors: (none)");
    }

    println!(
        "├─ Calculated effective startup actors ({} total): {}",
        analysis.effective_startup_actors.len(),
        analysis.effective_startup_actors.join(", ")
    );

    if analysis.has_valid_startup {
        println!(
            "└─ Validation: {} At least one starting actor configured",
            icons::SUCCESS_ICON
        );
    } else {
        println!(
            "└─ Validation: {} No starting actors configured",
            icons::FAILED_ICON
        );
    }

    println!();

    // Display all actors
    println!("All Actors ({} total):", resolved_actors.len());
    println!();

    // Sort actors: user-defined first, then dependencies
    let mut user_actors: Vec<_> = resolved_actors
        .values()
        .filter(|actor| !actor.is_dependency)
        .collect();
    let mut dependency_actors: Vec<_> = resolved_actors
        .values()
        .filter(|actor| actor.is_dependency)
        .collect();

    user_actors.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));
    dependency_actors.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));

    // Display user-defined actors
    for actor in user_actors {
        display_actor_info(actor, analysis);
    }

    // Display dependency actors
    for actor in dependency_actors {
        display_actor_info(actor, analysis);
    }

    println!();
    println!(
        "System Validation: {} All checks passed",
        icons::SUCCESS_ICON
    );
}

fn display_actor_info(actor: &ResolvedActor, analysis: &StartupAnalysis) {
    let mut tags = Vec::new();
    if !actor.is_dependency {
        tags.push("user-defined");
    } else {
        tags.push("dependency");
    }

    if analysis.starting_actors.contains(&actor.logical_name) {
        tags.push("starting");
    }

    if actor.auto_spawn {
        tags.push("auto-spawn");
    }

    let has_config = actor.config.is_some() && !actor.config.as_ref().unwrap().is_empty();
    if has_config {
        tags.push("overridden");
    }

    let tags_str = if tags.len() > 1 {
        format!(" ({})", tags.join(", "))
    } else if !tags.is_empty() {
        format!(" ({})", tags[0])
    } else {
        String::new()
    };

    println!("{}{}", actor.logical_name, tags_str);
    println!("   ID: {}", actor.actor_id);

    let source_str = match &actor.source {
        ActorSource::Path(path_source) => {
            if let Some(package) = &path_source.package {
                format!("path({}, package={})", path_source.path, package)
            } else {
                format!("path({})", path_source.path)
            }
        }
        ActorSource::Git(git_source) => {
            if let Some(package) = &git_source.package {
                format!("git({}, package={})", git_source.url, package)
            } else {
                format!("git({})", git_source.url)
            }
        }
    };
    println!("   Source: {}", source_str);

    if let Some(config) = &actor.config {
        let config_display = format_config_compact(config);
        println!("   Effective config:");
        println!("   ┌─────────────────────────────");
        for line in config_display.lines() {
            println!("   │ {}", line);
        }
        println!("   └─────────────────────────────");
    } else {
        println!("   Effective config: (empty)");
    }

    println!("   ───────────────────────────────────────────────────────────────────────────");
    println!();
}
