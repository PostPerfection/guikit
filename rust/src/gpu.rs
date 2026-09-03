use std::sync::Mutex;

pub struct AcceleratorStatus {
    pub requested: bool,
    pub active: bool,
    pub error: Option<String>,
}

impl std::fmt::Display for AcceleratorStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.requested, self.active, self.error.as_deref()) {
            (false, _, _) => write!(f, "off"),
            (true, true, _) => write!(f, "requested, active"),
            (true, false, Some(error)) => write!(f, "requested, inactive: {error}"),
            (true, false, None) => write!(f, "requested, inactive"),
        }
    }
}

struct AcceleratorRequest {
    requested: bool,
    error: Option<String>,
}

static LAST_REQUEST: Mutex<AcceleratorRequest> = Mutex::new(AcceleratorRequest {
    requested: false,
    error: None,
});

fn record_request(requested: bool, outcome: &Result<(), String>) {
    let mut last = LAST_REQUEST.lock().expect("accelerator request lock");
    last.requested = requested;
    last.error = outcome.as_ref().err().cloned();
}

/// Switch grok's accelerator plugin on or off for the whole process. The
/// `initialize` call loads the plugin and may run here first.
#[tauri::command]
pub fn set_gpu(enabled: bool) -> Result<bool, String> {
    postkit::grok_encoder::initialize(0);
    let outcome = if enabled {
        postkit::grok_encoder::use_gpu()
    } else {
        postkit::grok_encoder::use_cpu();
        Ok(())
    };
    record_request(enabled, &outcome);
    outcome?;
    Ok(postkit::grok_encoder::gpu_active())
}

pub fn accelerator_status() -> AcceleratorStatus {
    let last = LAST_REQUEST.lock().expect("accelerator request lock");
    AcceleratorStatus {
        requested: last.requested,
        active: postkit::grok_encoder::gpu_active(),
        error: last.error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // one test for both requests, since the recorded outcome is process wide
    #[test]
    fn the_status_says_what_was_asked_for_and_why_it_failed() {
        let failure = "the device is unavailable".to_string();
        record_request(true, &Err(failure.clone()));

        let status = accelerator_status();
        assert!(status.requested);
        assert!(!status.active);
        assert_eq!(status.error, Some(failure));
        assert_eq!(
            status.to_string(),
            "requested, inactive: the device is unavailable"
        );

        record_request(false, &Ok(()));
        assert_eq!(accelerator_status().to_string(), "off");
    }
}
