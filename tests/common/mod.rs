use std::sync::Once;

static INIT: Once = Once::new();

pub fn init_test_logger() {
    INIT.call_once(|| {
        // Initialize logger with a path in /workspace since that's where tests run in Docker
        hive::init_logger_with_path("/workspace/log.txt");
    });
}