use crate::{TuiResult, utils};

pub fn show_info() -> TuiResult<()> {
    println!("Hive Configuration and Cache Information");
    println!("======================================");

    // Show config directory
    let config_dir = hive::hive_config::get_config_dir()?;
    println!("Config directory: {}", config_dir.display());

    let config_file = hive::hive_config::get_config_file_path()?;
    if config_file.exists() {
        println!("Config file: {} (exists)", config_file.display());
    } else {
        println!("Config file: {} (not found)", config_file.display());
    }

    // Show cache directory
    let cache_dir = hive::hive_config::get_cache_dir()?;
    println!("\nCache directory: {}", cache_dir.display());

    let actors_cache_dir = hive::hive_config::get_actors_cache_dir()?;
    let cached_count = utils::count_cached_actors(&actors_cache_dir)?;

    if cached_count > 0 {
        println!(
            "Actor cache: {} (contains {} cached actors)",
            actors_cache_dir.display(),
            cached_count
        );
    } else {
        println!("Actor cache: {} (empty)", actors_cache_dir.display());
    }

    // Show log file location
    println!();
    let log_file = hive::hive_config::get_log_file_path()?;
    if log_file.exists() {
        println!("Log file: {} (exists)", log_file.display());
        if let Ok(metadata) = std::fs::metadata(&log_file) {
            println!("Log size: {} bytes", metadata.len());
        }
    } else {
        println!("Log file: {} (not created yet)", log_file.display());
    }
    println!("Note: Use --log-file <PATH> to specify a custom log location");

    Ok(())
}
