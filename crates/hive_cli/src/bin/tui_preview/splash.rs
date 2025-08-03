use std::{sync::Arc, time::Duration};

use hive::coordinator::HiveCoordinator;
use hive_actor_loader::LoadedActor;
use hive_cli::{TuiResult, tui};

pub async fn run() -> TuiResult<()> {
    let tui_config = hive_cli::config::TuiConfig::default().parse()?;

    let context = Arc::new(hive::context::HiveContext::new::<LoadedActor>(vec![]));
    let coordinator: HiveCoordinator = HiveCoordinator::new(context.clone());

    let tui = tui::Tui::new(tui_config, coordinator.get_sender(), None, context.clone());

    coordinator
        .start_hive(&vec![], "Root Agent".to_string())
        .await?;

    tui.run();

    tokio::time::sleep(Duration::from_secs(10_000)).await;

    Ok(())
}
