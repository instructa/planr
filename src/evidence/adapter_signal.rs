use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterBoundarySignal {
    PermissionDenied,
    SandboxBlocked,
}

impl AdapterBoundarySignal {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::SandboxBlocked => "sandbox_blocked",
        }
    }
}

pub(crate) fn adapter_boundary_signal_from_process_output(
    stdout_excerpt: Option<&str>,
    stderr_excerpt: Option<&str>,
) -> Option<AdapterBoundarySignal> {
    stdout_excerpt
        .and_then(adapter_boundary_signal_from_text)
        .or_else(|| stderr_excerpt.and_then(adapter_boundary_signal_from_text))
}

fn adapter_boundary_signal_from_text(text: &str) -> Option<AdapterBoundarySignal> {
    text.lines().find_map(|line| {
        let value = serde_json::from_str::<Value>(line.trim()).ok()?;
        let object = value.as_object()?;
        if object.keys().any(|key| key != "planr_adapter_boundary") {
            return None;
        }
        match object.get("planr_adapter_boundary")?.as_str()? {
            "permission_denied" => Some(AdapterBoundarySignal::PermissionDenied),
            "sandbox_blocked" => Some(AdapterBoundarySignal::SandboxBlocked),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{AdapterBoundarySignal, adapter_boundary_signal_from_process_output};

    #[test]
    fn adapter_boundary_signal_requires_exact_single_field_json_line() {
        assert_eq!(
            adapter_boundary_signal_from_process_output(
                None,
                Some("{\"planr_adapter_boundary\":\"sandbox_blocked\"}\n")
            ),
            Some(AdapterBoundarySignal::SandboxBlocked)
        );
        assert_eq!(
            adapter_boundary_signal_from_process_output(
                Some("{\"planr_adapter_boundary\":\"permission_denied\"}\n"),
                None
            ),
            Some(AdapterBoundarySignal::PermissionDenied)
        );
        assert_eq!(
            adapter_boundary_signal_from_process_output(
                None,
                Some("{\"planr_adapter_boundary\":\"sandbox_blocked\",\"extra\":true}\n")
            ),
            None
        );
        assert_eq!(
            adapter_boundary_signal_from_process_output(None, Some("sandbox_blocked\n")),
            None
        );
    }
}
