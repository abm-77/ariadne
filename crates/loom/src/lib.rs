use ariadne::diagnostics::{Diagnostic, Severity};
use std::path::Path;

/// Load a TIR document. The format is chosen by extension: `.pb` (protobuf
/// binary) or `.json` (serde JSON of the IR).
pub fn load_workflow(path: &Path) -> Result<ariadne::ir::Workflow, String> {
    ariadne::proto::load(path)
        .map_err(|e| format!("cannot load '{}': {e}", path.display()))
}

pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

pub fn print_diags(diags: &[Diagnostic]) {
    for d in diags {
        eprintln!("{d}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_workflow_missing_file_is_err() {
        let result = load_workflow(Path::new("/nonexistent/path/workflow.json"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot load"));
    }

    #[test]
    fn load_workflow_bad_json_is_err() {
        let dir = std::env::temp_dir();
        let path = dir.join("ariadne-test-bad.json");
        std::fs::write(&path, b"not json").unwrap();
        let result = load_workflow(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot load"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_workflow_unknown_extension_is_err() {
        let dir = std::env::temp_dir();
        let path = dir.join("ariadne-test.yaml");
        std::fs::write(&path, b"x").unwrap();
        let result = load_workflow(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown TIR extension"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn has_errors_true_when_any_error_present() {
        let diags = vec![
            ariadne::diagnostics::Diagnostic::warning(ariadne::diagnostics::DiagCode::UnusedInput, "w"),
            ariadne::diagnostics::Diagnostic::error(ariadne::diagnostics::DiagCode::CycleDetected, "e"),
        ];
        assert!(has_errors(&diags));
    }

    #[test]
    fn has_errors_false_when_only_warnings() {
        let diags = vec![
            ariadne::diagnostics::Diagnostic::warning(ariadne::diagnostics::DiagCode::FallbackPlacementSelected, "w"),
        ];
        assert!(!has_errors(&diags));
    }
}
