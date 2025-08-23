use std::{sync::Arc, time::Duration};

use wasmind::coordinator::WasmindCoordinator;
use wasmind::wasmind_actor_loader::LoadedActor;
use wasmind_cli::{TuiResult, tui};

pub async fn run() -> TuiResult<()> {
    let tui_config = wasmind_cli::config::TuiConfig::default().parse()?;

    let context = Arc::new(wasmind::context::WasmindContext::new::<LoadedActor>(vec![]));
    let coordinator: WasmindCoordinator = WasmindCoordinator::new(context.clone());

    let tui = tui::Tui::new(tui_config, coordinator.get_sender(), None, context.clone());

    coordinator
        .start_wasmind(&[], "Root Agent".to_string())
        .await?;

    tui.run();

    tokio::time::sleep(Duration::from_secs(10_000)).await;

    Ok(())
}
