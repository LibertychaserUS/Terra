//! Execution Plan
//!
//! SpecIR describes WHAT to do. ExecutionPlan describes HOW.
//! ExecutionPlan = SpecIR + PlanSteps + version + progress.
//!
//! Mutable: steps can be added, removed, reordered mid-conversation.
//! Versioned: every mutation increments version, old versions in audit.
//! Serializable: JSON/MessagePack, no closures.

use serde::{Deserialize, Serialize};

use crate::spec_ir::SpecIR;

/// Status of a plan step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

/// A single step in the execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub name: String,
    pub description: String,
    /// CommandOp names this step needs (e.g., "FileRead", "ProviderCall")
    pub required_ops: Vec<String>,
    /// Reference to a previous step's output field (e.g., "read.content")
    pub input_ref: Option<String>,
    /// Steps that must complete before this one starts.
    pub depends_on: Vec<String>,
    pub status: StepStatus,
    /// Output data from execution (filled at runtime).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
}

/// The full execution plan — evolves through conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub spec: SpecIR,
    pub steps: Vec<PlanStep>,
    pub version: u32,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Draft,
    Confirmed,
    Running,
    Completed,
    Failed,
}

impl ExecutionPlan {
    /// Create a new plan from a SpecIR with no steps yet.
    pub fn from_spec(spec: SpecIR) -> Self {
        Self {
            spec,
            steps: Vec::new(),
            version: 1,
            status: PlanStatus::Draft,
        }
    }

    /// Add a step. Increments version.
    pub fn add_step(&mut self, step: PlanStep) {
        self.steps.push(step);
        self.version += 1;
    }

    /// Remove a step by name. Increments version.
    pub fn remove_step(&mut self, name: &str) -> bool {
        let before = self.steps.len();
        self.steps.retain(|s| s.name != name);
        if self.steps.len() != before {
            self.version += 1;
            true
        } else {
            false
        }
    }

    /// Update the spec (target, constraints, etc). Increments version.
    pub fn update_spec(&mut self, spec: SpecIR) {
        self.spec = spec;
        self.version += 1;
    }

    /// Mark a step as running.
    pub fn start_step(&mut self, name: &str) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.name == name) {
            step.status = StepStatus::Running;
        }
    }

    /// Mark a step as done with output.
    pub fn complete_step(&mut self, name: &str, output: serde_json::Value) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.name == name) {
            step.status = StepStatus::Done;
            step.output = Some(output);
        }
    }

    /// Mark a step as failed.
    pub fn fail_step(&mut self, name: &str) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.name == name) {
            step.status = StepStatus::Failed;
        }
    }

    /// Get the next pending step whose dependencies are all done.
    pub fn next_step(&self) -> Option<&PlanStep> {
        self.steps.iter().find(|step| {
            step.status == StepStatus::Pending
                && step.depends_on.iter().all(|dep| {
                    self.steps.iter().any(|s| s.name == *dep && s.status == StepStatus::Done)
                })
        })
    }

    /// Check if all steps are done.
    pub fn is_complete(&self) -> bool {
        !self.steps.is_empty()
            && self.steps.iter().all(|s| {
                s.status == StepStatus::Done || s.status == StepStatus::Skipped
            })
    }

    /// Check if any step failed.
    pub fn has_failure(&self) -> bool {
        self.steps.iter().any(|s| s.status == StepStatus::Failed)
    }

    /// Resolve an input reference like "read.content" from accumulated step outputs.
    pub fn resolve_ref(&self, ref_path: &str) -> Option<serde_json::Value> {
        let parts: Vec<&str> = ref_path.splitn(2, '.').collect();
        if parts.len() != 2 {
            return None;
        }
        let step_name = parts[0];
        let field = parts[1];

        self.steps
            .iter()
            .find(|s| s.name == step_name)
            .and_then(|s| s.output.as_ref())
            .and_then(|out| out.get(field))
            .cloned()
    }

    /// Count completed steps.
    pub fn completed_count(&self) -> usize {
        self.steps.iter().filter(|s| s.status == StepStatus::Done).count()
    }

    /// Render as human-readable text for /plan command.
    pub fn display(&self) -> String {
        let mut lines = vec![
            format!("Plan v{} [{}]", self.version, format!("{:?}", self.status).to_lowercase()),
            format!("Goal: {}", self.spec.intent),
            String::new(),
        ];

        for (i, step) in self.steps.iter().enumerate() {
            let icon = match step.status {
                StepStatus::Pending => "○",
                StepStatus::Running => "◉",
                StepStatus::Done => "✓",
                StepStatus::Failed => "✗",
                StepStatus::Skipped => "−",
            };
            let deps = if step.depends_on.is_empty() {
                String::new()
            } else {
                format!(" (after: {})", step.depends_on.join(", "))
            };
            lines.push(format!("  {} {}. {}{}", icon, i + 1, step.description, deps));
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec_ir::SpecIR;

    fn test_plan() -> ExecutionPlan {
        let spec = SpecIR::from_task_description("add logging to auth.rs");
        let mut plan = ExecutionPlan::from_spec(spec);

        plan.add_step(PlanStep {
            name: "read".into(),
            description: "Read auth.rs".into(),
            required_ops: vec!["FileRead".into()],
            input_ref: None,
            depends_on: vec![],
            status: StepStatus::Pending,
            output: None,
        });
        plan.add_step(PlanStep {
            name: "modify".into(),
            description: "Add logging via LLM".into(),
            required_ops: vec!["ProviderCall".into()],
            input_ref: Some("read.content".into()),
            depends_on: vec!["read".into()],
            status: StepStatus::Pending,
            output: None,
        });
        plan.add_step(PlanStep {
            name: "write".into(),
            description: "Write modified file".into(),
            required_ops: vec!["FileWrite".into()],
            input_ref: Some("modify.content".into()),
            depends_on: vec!["modify".into()],
            status: StepStatus::Pending,
            output: None,
        });

        plan
    }

    // T-EP-001: plan has steps
    #[test]
    fn plan_has_steps() {
        let plan = test_plan();
        assert_eq!(plan.steps.len(), 3);
        assert!(plan.steps.iter().all(|s| s.status == StepStatus::Pending));
    }

    // T-EP-002: step dependencies
    #[test]
    fn plan_step_dependencies() {
        let plan = test_plan();
        // First step (read) has no deps → is next
        assert_eq!(plan.next_step().unwrap().name, "read");

        // modify depends on read → not next until read is done
        let modify = &plan.steps[1];
        assert_eq!(modify.depends_on, vec!["read"]);
    }

    // T-EP-003: plan mutable
    #[test]
    fn plan_mutable() {
        let mut plan = test_plan();
        let v1 = plan.version;

        plan.add_step(PlanStep {
            name: "test".into(),
            description: "Run tests".into(),
            required_ops: vec!["ShellExec".into()],
            input_ref: None,
            depends_on: vec!["write".into()],
            status: StepStatus::Pending,
            output: None,
        });
        assert_eq!(plan.steps.len(), 4);
        assert!(plan.version > v1);

        let v2 = plan.version;
        plan.remove_step("test");
        assert_eq!(plan.steps.len(), 3);
        assert!(plan.version > v2);
    }

    // T-EP-004: plan versions (version increments on mutation)
    #[test]
    fn plan_versions() {
        let mut plan = test_plan();
        // Initial version after 3 add_step calls = 4 (1 base + 3 adds)
        let v = plan.version;

        plan.update_spec(SpecIR::from_task_description("new goal"));
        assert_eq!(plan.version, v + 1);

        plan.remove_step("write");
        assert_eq!(plan.version, v + 2);
    }

    // T-EP-005: plan display
    #[test]
    fn plan_display() {
        let plan = test_plan();
        let display = plan.display();
        assert!(display.contains("Read auth.rs"));
        assert!(display.contains("Add logging"));
        assert!(display.contains("Write modified"));
        assert!(display.contains("○")); // pending icon
    }

    // T-EP-006: next step follows dependency order
    #[test]
    fn next_step_follows_deps() {
        let mut plan = test_plan();

        // read is first (no deps)
        assert_eq!(plan.next_step().unwrap().name, "read");

        // Complete read → modify becomes next
        plan.complete_step("read", serde_json::json!({"content": "file data"}));
        assert_eq!(plan.next_step().unwrap().name, "modify");

        // Complete modify → write becomes next
        plan.complete_step("modify", serde_json::json!({"content": "modified data"}));
        assert_eq!(plan.next_step().unwrap().name, "write");

        // Complete write → no more steps
        plan.complete_step("write", serde_json::json!({"written": true}));
        assert!(plan.next_step().is_none());
        assert!(plan.is_complete());
    }

    // T-EP-007: plan serializable
    #[test]
    fn plan_serializable() {
        let plan = test_plan();
        let json = serde_json::to_string(&plan).unwrap();
        let restored: ExecutionPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.steps.len(), 3);
        assert_eq!(restored.version, plan.version);
        assert_eq!(restored.spec.intent, "add logging to auth.rs");
    }

    // T-SR-001: resolve input reference
    #[test]
    fn resolve_input_ref() {
        let mut plan = test_plan();
        plan.complete_step("read", serde_json::json!({"content": "fn main() {}"}));

        let resolved = plan.resolve_ref("read.content");
        assert_eq!(resolved, Some(serde_json::json!("fn main() {}")));
    }

    // T-SR-004: invalid reference
    #[test]
    fn invalid_ref_errors() {
        let plan = test_plan();
        assert!(plan.resolve_ref("nonexistent.field").is_none());
        assert!(plan.resolve_ref("badformat").is_none());
    }

    // Completion tracking
    #[test]
    fn completion_tracking() {
        let mut plan = test_plan();
        assert!(!plan.is_complete());
        assert_eq!(plan.completed_count(), 0);

        plan.complete_step("read", serde_json::json!({}));
        assert_eq!(plan.completed_count(), 1);

        plan.fail_step("modify");
        assert!(plan.has_failure());
        assert!(!plan.is_complete());
    }
}
