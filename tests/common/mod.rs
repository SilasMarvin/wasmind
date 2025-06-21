use hive::config::{Config, ParsedConfig};
use std::sync::Once;

static INIT: Once = Once::new();

pub fn init_test_logger() {
    INIT.call_once(|| {
        // Initialize logger with a path in /workspace since that's where tests run in Docker
        hive::init_logger_with_path("/workspace/log.txt");
    });
}

pub fn create_test_config_with_mock_endpoint(mock_endpoint: String) -> ParsedConfig {
    let mut config = Config::default().unwrap();

    // Use gpt-4o for all models
    config.hive.main_manager_model.name = "gpt-4o".to_string();
    config.hive.sub_manager_model.name = "gpt-4o".to_string();
    config.hive.worker_model.name = "gpt-4o".to_string();

    // Set mock endpoints (needs /v1/ suffix for genai client)
    let endpoint_with_v1 = format!("{}/v1/", mock_endpoint);
    config.hive.main_manager_model.endpoint = Some(endpoint_with_v1.clone());
    config.hive.sub_manager_model.endpoint = Some(endpoint_with_v1.clone());
    config.hive.worker_model.endpoint = Some(endpoint_with_v1);

    config.try_into().unwrap()
}
