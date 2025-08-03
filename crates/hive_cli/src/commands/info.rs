use crate::TuiResult;

pub fn show_info() -> TuiResult<()> {
    println!("Hive Configuration and Cache Information");
    println!("======================================");
    
    // Show config directory
    let config_dir = hive_config::get_config_dir()?;
    println!("Config directory: {}", config_dir.display());
    
    let config_file = hive_config::get_config_file_path()?;
    if config_file.exists() {
        println!("Config file: {} (exists)", config_file.display());
    } else {
        println!("Config file: {} (not found)", config_file.display());
    }
    
    // Show cache directory  
    let cache_dir = hive_config::get_cache_dir()?;
    println!("Cache directory: {}", cache_dir.display());
    
    let actors_cache_dir = hive_config::get_actors_cache_dir()?;
    let cached_count = hive_config::count_cached_actors(&actors_cache_dir)?;
    
    if cached_count > 0 {
        println!("Actor cache: {} (contains {} cached actors)", actors_cache_dir.display(), cached_count);
    } else {
        println!("Actor cache: {} (empty)", actors_cache_dir.display());
    }
    
    Ok(())
}