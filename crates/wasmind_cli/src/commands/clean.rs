use crate::{TuiResult, utils};

pub fn clean_cache() -> TuiResult<()> {
    let actors_cache_dir = wasmind::wasmind_config::get_actors_cache_dir()?;

    if !actors_cache_dir.exists() {
        println!("No actor cache found at {}", actors_cache_dir.display());
        return Ok(());
    }

    println!("Cleaning actor cache at {}...", actors_cache_dir.display());

    // Remove the entire actors cache directory
    utils::remove_actors_cache(&actors_cache_dir)?;

    println!("✓ Actor cache cleaned successfully");
    Ok(())
}
