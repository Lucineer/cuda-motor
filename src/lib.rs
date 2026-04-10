/*!
# cuda-motor

Motor primitives — turning thought into action.

Perception reads the world. Cognition thinks about it. Motor ACTS.
This crate bridges the gap between "I want to go there" and actual
actuator commands. Every action has a cost, a duration, and a success
probability.

- Action types (move, rotate, grab, release, speak, wait)
- Action sequences with dependencies
- Effort estimation (energy cost before execution)
- Safety guards (collision avoidance, force limits)
- Action feedback (did it work? how well?)
*/

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// An action the agent can take
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub kind: ActionKind,
    pub params: HashMap<String, f64>,
    pub target: Option<(f64, f64)>,       // position target
    pub effort: f64,                        // estimated energy cost
    pub duration_ms: u64,                   // estimated duration
    pub confidence: f64,                    // probability of success
    pub safety_check: bool,                 // passed safety guard
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    Move,       // translate in space
    Rotate,     // change orientation
    Grab,       // pick up object
    Release,    // put down object
    Speak,      // emit message
    Listen,     // attend to input
    Wait,       // idle
    Scan,       // active perception sweep
    Retreat,    // back away
    Stop,       // emergency halt
}

impl Action {
    pub fn new(kind: ActionKind) -> Self {
        let effort = match kind {
            ActionKind::Move => 0.1, ActionKind::Rotate => 0.05, ActionKind::Grab => 0.15,
            ActionKind::Release => 0.05, ActionKind::Speak => 0.02, ActionKind::Listen => 0.01,
            ActionKind::Wait => 0.0, ActionKind::Scan => 0.08, ActionKind::Retreat => 0.12,
            ActionKind::Stop => 0.01,
        };
        let duration = match kind {
            ActionKind::Move => 1000, ActionKind::Rotate => 500, ActionKind::Grab => 800,
            ActionKind::Release => 300, ActionKind::Speak => 200, ActionKind::Listen => 500,
            ActionKind::Wait => 0, ActionKind::Scan => 2000, ActionKind::Retreat => 800,
            ActionKind::Stop => 0,
        };
        Action { id: String::new(), kind, params: HashMap::new(), target: None, effort, duration_ms: duration, confidence: 0.8, safety_check: false }
    }

    pub fn with_target(mut self, x: f64, y: f64) -> Self { self.target = Some((x, y)); self }
    pub fn with_param(mut self, key: &str, val: f64) -> Self { self.params.insert(key.to_string(), val); self }
    pub fn with_effort(mut self, e: f64) -> Self { self.effort = e; self }
}

/// Result of executing an action
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionResult {
    pub action_id: String,
    pub success: bool,
    pub actual_effort: f64,
    pub actual_duration_ms: u64,
    pub error: Option<String>,
    pub feedback: HashMap<String, f64>,
}

impl ActionResult {
    pub fn ok(action_id: &str) -> Self { ActionResult { action_id: action_id.to_string(), success: true, actual_effort: 0.0, actual_duration_ms: 0, error: None, feedback: HashMap::new() } }
    pub fn fail(action_id: &str, reason: &str) -> Self { ActionResult { action_id: action_id.to_string(), success: false, actual_effort: 0.0, actual_duration_ms: 0, error: Some(reason.to_string()), feedback: HashMap::new() } }
}

/// An action sequence — ordered plan of actions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionSequence {
    pub id: String,
    pub actions: Vec<Action>,
    pub current_index: usize,
    pub total_effort: f64,
    pub total_duration_ms: u64,
    pub status: SequenceStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequenceStatus { Pending, Executing, Paused, Completed, Failed, Cancelled }

impl ActionSequence {
    pub fn new(id: &str) -> Self { ActionSequence { id: id.to_string(), actions: vec![], current_index: 0, total_effort: 0.0, total_duration_ms: 0, status: SequenceStatus::Pending } }

    pub fn add(&mut self, mut action: Action) {
        action.id = format!("{}_{}", self.id, self.actions.len());
        self.total_effort += action.effort;
        self.total_duration_ms += action.duration_ms;
        self.actions.push(action);
    }

    pub fn current_action(&self) -> Option<&Action> { self.actions.get(self.current_index) }

    pub fn advance(&mut self) -> bool {
        self.current_index += 1;
        if self.current_index >= self.actions.len() { self.status = SequenceStatus::Completed; return false; }
        true
    }

    pub fn progress(&self) -> f64 {
        if self.actions.is_empty() { return 0.0; }
        self.current_index as f64 / self.actions.len() as f64
    }

    pub fn remaining_effort(&self) -> f64 {
        self.actions[self.current_index..].iter().map(|a| a.effort).sum()
    }

    pub fn cancel(&mut self) { self.status = SequenceStatus::Cancelled; }
}

/// Safety guard — checks before executing actions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SafetyGuard {
    pub max_force: f64,
    pub max_speed: f64,
    pub collision_radius: f64,
    pub obstacle_positions: Vec<(f64, f64)>,
    pub safe_zones: Vec<(f64, f64, f64)>, // x, y, radius
    pub emergency_stop: bool,
}

impl SafetyGuard {
    pub fn new() -> Self { SafetyGuard { max_force: 10.0, max_speed: 2.0, collision_radius: 0.5, obstacle_positions: vec![], safe_zones: vec![], emergency_stop: false } }

    /// Check if an action is safe to execute
    pub fn check(&self, action: &Action) -> SafetyResult {
        if self.emergency_stop { return SafetyResult { safe: false, reason: "Emergency stop active".into(), blocked_by: None }; }

        // Check speed parameter
        if let Some(&speed) = action.params.get("speed") {
            if speed > self.max_speed { return SafetyResult { safe: false, reason: format!("Speed {} exceeds max {}", speed, self.max_speed), blocked_by: None }; }
        }

        // Check collision for move actions
        if action.kind == ActionKind::Move {
            if let Some((tx, ty)) = action.target {
                for (ox, oy) in &self.obstacle_positions {
                    let dist = ((tx - ox).powi(2) + (ty - oy).powi(2)).sqrt();
                    if dist < self.collision_radius * 2.0 {
                        return SafetyResult { safe: false, reason: format!("Collision risk at ({:.1},{:.1})", tx, ty), blocked_by: Some((*ox, *oy)) };
                    }
                }
            }
        }

        SafetyResult { safe: true, reason: "Safe".into(), blocked_by: None }
    }

    /// Add obstacle
    pub fn add_obstacle(&mut self, x: f64, y: f64) { self.obstacle_positions.push((x, y)); }

    /// Add safe zone
    pub fn add_safe_zone(&mut self, x: f64, y: f64, radius: f64) { self.safe_zones.push((x, y, radius)); }
}

#[derive(Clone, Debug)]
pub struct SafetyResult { pub safe: bool, pub reason: String, pub blocked_by: Option<(f64, f64)> }

/// Motor controller — manages action execution
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MotorController {
    pub sequences: Vec<ActionSequence>,
    pub active_sequence: Option<usize>,
    pub safety: SafetyGuard,
    pub energy_budget: f64,
    pub energy_spent: f64,
    pub actions_executed: u32,
    pub actions_failed: u32,
    pub next_action_id: u64,
}

impl MotorController {
    pub fn new() -> Self { MotorController { sequences: vec![], active_sequence: None, safety: SafetyGuard::new(), energy_budget: 10.0, energy_spent: 0.0, actions_executed: 0, actions_failed: 0, next_action_id: 0 } }

    /// Plan a new sequence
    pub fn plan(&mut self, id: &str) -> usize {
        let seq = ActionSequence::new(id);
        self.sequences.push(seq);
        self.sequences.len() - 1
    }

    /// Add action to a sequence
    pub fn add_to_sequence(&mut self, seq_idx: usize, action: Action) -> bool {
        if let Some(seq) = self.sequences.get_mut(seq_idx) { seq.add(action); true } else { false }
    }

    /// Execute next action in active sequence
    pub fn execute_next(&mut self) -> Option<ActionResult> {
        let seq_idx = self.active_sequence?;
        let seq = &mut self.sequences[seq_idx];

        let action = seq.current_action()?.clone();
        seq.status = SequenceStatus::Executing;

        // Safety check
        let safety = self.safety.check(&action);
        if !safety.safe {
            seq.status = SequenceStatus::Failed;
            self.actions_failed += 1;
            return Some(ActionResult::fail(&action.id, &safety.reason));
        }

        // Energy check
        if self.energy_budget - self.energy_spent < action.effort {
            seq.status = SequenceStatus::Paused;
            return Some(ActionResult::fail(&action.id, "insufficient energy"));
        }

        // Execute
        self.energy_spent += action.effort;
        self.actions_executed += 1;
        let result = ActionResult::ok(&action.id);
        seq.advance();
        Some(result)
    }

    /// Start a sequence
    pub fn start(&mut self, seq_idx: usize) -> bool {
        if seq_idx >= self.sequences.len() { return false; }
        self.sequences[seq_idx].status = SequenceStatus::Executing;
        self.sequences[seq_idx].current_index = 0;
        self.active_sequence = Some(seq_idx);
        true
    }

    /// Emergency stop
    pub fn emergency_stop(&mut self) { self.safety.emergency_stop = true; }

    /// Clear emergency stop
    pub fn clear_emergency(&mut self) { self.safety.emergency_stop = false; }

    /// Remaining energy
    pub fn remaining_energy(&self) -> f64 { self.energy_budget - self.energy_spent }

    /// Success rate
    pub fn success_rate(&self) -> f64 {
        let total = self.actions_executed + self.actions_failed;
        if total == 0 { return 0.0; }
        self.actions_executed as f64 / total as f64
    }

    /// Summary
    pub fn summary(&self) -> String {
        format!("Motor: {} sequences, executed={}, failed={}, success_rate={:.0%}, energy={:.2}/{:.2}",
            self.sequences.len(), self.actions_executed, self.actions_failed, self.success_rate(), self.energy_spent, self.energy_budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_creation() {
        let action = Action::new(ActionKind::Move).with_target(5.0, 3.0);
        assert_eq!(action.kind, ActionKind::Move);
        assert_eq!(action.target, Some((5.0, 3.0)));
    }

    #[test]
    fn test_sequence_planning() {
        let mut mc = MotorController::new();
        let idx = mc.plan("go_to_door");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move).with_target(5.0, 0.0));
        mc.add_to_sequence(idx, Action::new(ActionKind::Rotate));
        assert_eq!(mc.sequences[idx].actions.len(), 2);
    }

    #[test]
    fn test_sequence_execute() {
        let mut mc = MotorController::new();
        let idx = mc.plan("seq1");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move).with_target(1.0, 0.0));
        mc.add_to_sequence(idx, Action::new(ActionKind::Stop));
        mc.start(idx);
        let r = mc.execute_next();
        assert!(r.unwrap().success);
        assert!(mc.sequences[idx].current_index == 1);
    }

    #[test]
    fn test_safety_collision() {
        let mut mc = MotorController::new();
        mc.safety.add_obstacle(5.0, 0.0);
        let idx = mc.plan("crash");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move).with_target(5.0, 0.0));
        mc.start(idx);
        let r = mc.execute_next();
        assert!(!r.unwrap().success);
    }

    #[test]
    fn test_emergency_stop() {
        let mut mc = MotorController::new();
        let idx = mc.plan("seq");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move).with_target(1.0, 0.0));
        mc.start(idx);
        mc.emergency_stop();
        let r = mc.execute_next();
        assert!(!r.unwrap().success);
    }

    #[test]
    fn test_energy_budget() {
        let mut mc = MotorController::new();
        mc.energy_budget = 0.01;
        let idx = mc.plan("seq");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move).with_target(1.0, 0.0));
        mc.start(idx);
        let r = mc.execute_next();
        assert!(!r.unwrap().success);
    }

    #[test]
    fn test_sequence_progress() {
        let mut mc = MotorController::new();
        let idx = mc.plan("seq");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move));
        mc.add_to_sequence(idx, Action::new(ActionKind::Move));
        mc.add_to_sequence(idx, Action::new(ActionKind::Move));
        mc.start(idx);
        mc.execute_next(); mc.execute_next();
        let seq = &mc.sequences[idx];
        assert!((seq.progress() - 0.667).abs() < 0.01);
    }

    #[test]
    fn test_speed_safety() {
        let mut mc = MotorController::new();
        mc.safety.max_speed = 1.0;
        let idx = mc.plan("fast");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move).with_param("speed", 5.0));
        mc.start(idx);
        let r = mc.execute_next();
        assert!(!r.unwrap().success);
    }

    #[test]
    fn test_remaining_effort() {
        let mut seq = ActionSequence::new("s");
        seq.add(Action::new(ActionKind::Move));
        seq.add(Action::new(ActionKind::Move));
        seq.current_index = 1;
        assert!(seq.remaining_effort() < seq.total_effort);
    }

    #[test]
    fn test_cancel_sequence() {
        let mut mc = MotorController::new();
        let idx = mc.plan("cancel_me");
        mc.add_to_sequence(idx, Action::new(ActionKind::Move));
        mc.start(idx);
        mc.sequences[idx].cancel();
        assert_eq!(mc.sequences[idx].status, SequenceStatus::Cancelled);
    }
}
