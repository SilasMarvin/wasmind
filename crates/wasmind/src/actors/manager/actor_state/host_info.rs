use crate::actors::manager::wasmind::actor::host_info;

use super::ActorState;

impl host_info::Host for ActorState {
    async fn get_host_working_directory(&mut self) -> String {
        match std::env::current_dir() {
            Ok(cwd) => cwd.display().to_string(),
            Err(_) => "/".to_string(),
        }
    }

    async fn get_host_os_info(&mut self) -> host_info::OsInfo {
        host_info::OsInfo {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}
