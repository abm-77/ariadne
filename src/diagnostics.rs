use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagCode {
    MissingProducer,
    TypeMismatch,
    CycleDetected,
    UnknownArtifact,
    UnknownConsequence,
    UnusedInput,
    ConsequenceRequiresApproval,
    BackendCapabilityUnavailable,
    FallbackPlacementSelected,
    IndexOutOfBounds,
    UnknownAction,
    ActionPortMismatch,
    UnknownSemanticOp,
    NoCompatibleImplementation,
    AmbiguousImplementation,
    UnsatisfiableResources,
}

impl std::fmt::Display for DiagCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::MissingProducer => "MissingProducer",
            Self::TypeMismatch => "TypeMismatch",
            Self::CycleDetected => "CycleDetected",
            Self::UnknownArtifact => "UnknownArtifact",
            Self::UnknownConsequence => "UnknownConsequence",
            Self::UnusedInput => "UnusedInput",
            Self::ConsequenceRequiresApproval => "ConsequenceRequiresApproval",
            Self::BackendCapabilityUnavailable => "BackendCapabilityUnavailable",
            Self::FallbackPlacementSelected => "FallbackPlacementSelected",
            Self::IndexOutOfBounds => "IndexOutOfBounds",
            Self::UnknownAction => "UnknownAction",
            Self::ActionPortMismatch => "ActionPortMismatch",
            Self::UnknownSemanticOp => "UnknownSemanticOp",
            Self::NoCompatibleImplementation => "NoCompatibleImplementation",
            Self::AmbiguousImplementation => "AmbiguousImplementation",
            Self::UnsatisfiableResources => "UnsatisfiableResources",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub source: String,
    pub offset: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagCode,
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    pub suggested_fix: Option<String>,
}

impl Diagnostic {
    pub fn error(code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            span: None,
            suggested_fix: None,
        }
    }

    pub fn warning(code: DiagCode, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            message: message.into(),
            span: None,
            suggested_fix: None,
        }
    }

    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.suggested_fix = Some(fix.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        write!(f, "[{sev}][{}] {}", self.code, self.message)?;
        if let Some(ref fix) = self.suggested_fix {
            write!(f, " (fix: {fix})")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_has_error_severity() {
        let d = Diagnostic::error(DiagCode::CycleDetected, "cycle");
        assert_eq!(d.severity, Severity::Error);
        assert!(d.is_error());
    }

    #[test]
    fn warning_has_warning_severity() {
        let d = Diagnostic::warning(DiagCode::FallbackPlacementSelected, "fallback");
        assert_eq!(d.severity, Severity::Warning);
        assert!(!d.is_error());
    }

    #[test]
    fn with_fix_attaches_suggestion() {
        let d = Diagnostic::error(DiagCode::MissingProducer, "missing").with_fix("add an action");
        assert_eq!(d.suggested_fix.as_deref(), Some("add an action"));
    }

    #[test]
    fn display_includes_severity_code_and_message() {
        let s = Diagnostic::error(DiagCode::CycleDetected, "cycle detected").to_string();
        assert!(s.contains("error"));
        assert!(s.contains("CycleDetected"));
        assert!(s.contains("cycle detected"));
    }

    #[test]
    fn display_appends_fix_when_present() {
        let s = Diagnostic::warning(DiagCode::UnusedInput, "unused")
            .with_fix("remove it")
            .to_string();
        assert!(s.contains("remove it"));
    }
}
