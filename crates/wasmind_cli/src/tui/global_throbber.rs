use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use throbber_widgets_tui::ThrobberState;

/// Minimum time between throbber animation updates (shared across all throbber implementations)
pub const MIN_THROBBER_UPDATE_GAP: Duration = Duration::from_millis(300);

/// Global throbber state and timing
struct GlobalThrobberData {
    state: ThrobberState,
    last_update: Instant,
}

impl Default for GlobalThrobberData {
    fn default() -> Self {
        Self {
            state: ThrobberState::default(),
            last_update: Instant::now(),
        }
    }
}

/// Global static throbber state
static GLOBAL_THROBBER: LazyLock<Mutex<GlobalThrobberData>> =
    LazyLock::new(|| Mutex::new(GlobalThrobberData::default()));

/// Get a clone of the current global throbber state
pub fn get_current_state() -> ThrobberState {
    GLOBAL_THROBBER.lock().unwrap().state.clone()
}

/// Tick the global throbber state if enough time has elapsed
/// This should only be called from the dashboard's Event::Tick handler
pub fn tick() {
    let mut data = GLOBAL_THROBBER.lock().unwrap();
    if data.last_update.elapsed() >= MIN_THROBBER_UPDATE_GAP {
        data.state.calc_next();
        data.last_update = Instant::now();
    }
}
