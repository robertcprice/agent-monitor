//! Analytics module for intelligent session monitoring.
//! Inspired by Ralph (exit detection, circuit breaker) and Auto-Claude (memory persistence).

use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::models::{SessionEvent, EventType};

// ============================================================================
// Exit Detection System (Ralph-inspired)
// ============================================================================

/// Reasons for graceful exit detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    /// Task list is 100% complete
    TaskListComplete,
    /// Multiple "done" signals detected
    CompletionSignals,
    /// Strong completion signal (e.g., "ready for review")
    StrongCompletion,
    /// Project marked as complete
    ProjectComplete,
    /// Test saturation - only running tests, no progress
    TestSaturation,
    /// User requested stop
    UserRequested,
    /// Circuit breaker triggered
    CircuitBreakerOpen,
    /// Rate limit hit
    RateLimitExceeded,
    /// API limit (e.g., Claude 5-hour limit)
    ApiLimitReached,
}

/// Completion signal patterns to detect.
const DONE_PATTERNS: &[&str] = &[
    "all tasks completed",
    "all tasks complete",
    "implementation complete",
    "feature complete",
    "work complete",
    "all done",
    "everything is done",
    "finished implementing",
    "successfully completed",
    "task completed successfully",
    "no more tasks",
    "nothing left to do",
];

/// Strong completion indicators (higher confidence).
const STRONG_COMPLETION_PATTERNS: &[&str] = &[
    "all requirements have been met",
    "the implementation is complete",
    "all features are working",
    "tests are passing",
    "ready for review",
    "ready to merge",
    "pr ready",
    "pull request ready",
];

/// Test-only activity patterns.
const TEST_ONLY_PATTERNS: &[&str] = &[
    "running tests",
    "test passed",
    "tests passed",
    "all tests pass",
    "pytest",
    "cargo test",
    "npm test",
    "jest",
    "vitest",
];

/// Exit detector for session completion analysis.
#[derive(Debug, Clone)]
pub struct ExitDetector {
    /// Consecutive "done" signal count
    done_signal_count: u32,
    /// Consecutive test-only loop count
    test_only_count: u32,
    /// Strong completion indicator count
    completion_indicator_count: u32,
    /// History of recent content for pattern matching
    recent_content: Vec<String>,
    /// Maximum recent content entries to keep
    max_recent: usize,
    /// Threshold for done signals before exit
    done_threshold: u32,
    /// Threshold for test saturation
    test_saturation_threshold: u32,
    /// Threshold for completion indicators
    completion_threshold: u32,
}

impl Default for ExitDetector {
    fn default() -> Self {
        Self {
            done_signal_count: 0,
            test_only_count: 0,
            completion_indicator_count: 0,
            recent_content: Vec::new(),
            max_recent: 20,
            done_threshold: 2,
            test_saturation_threshold: 3,
            completion_threshold: 2,
        }
    }
}

impl ExitDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyze an event and update detection state.
    /// Returns Some(ExitReason) if exit condition is met.
    pub fn analyze_event(&mut self, event: &SessionEvent) -> Option<ExitReason> {
        let content = event.content.as_deref().unwrap_or("");
        let content_lower = content.to_lowercase();

        // Store recent content for pattern analysis
        if !content.is_empty() {
            self.recent_content.push(content_lower.clone());
            if self.recent_content.len() > self.max_recent {
                self.recent_content.remove(0);
            }
        }

        // Check for done patterns
        let has_done_signal = DONE_PATTERNS.iter().any(|p| content_lower.contains(p));
        if has_done_signal {
            self.done_signal_count += 1;
            debug!("Done signal detected (count: {})", self.done_signal_count);
        } else {
            // Reset if no done signal in this message
            self.done_signal_count = 0;
        }

        // Check for strong completion indicators (immediate exit)
        let has_strong_completion = STRONG_COMPLETION_PATTERNS
            .iter()
            .any(|p| content_lower.contains(p));
        if has_strong_completion {
            self.completion_indicator_count += 1;
            debug!(
                "Strong completion indicator (count: {})",
                self.completion_indicator_count
            );
            // Strong completion triggers immediately
            return Some(ExitReason::StrongCompletion);
        }

        // Check for test-only activity
        let is_test_only = TEST_ONLY_PATTERNS.iter().any(|p| content_lower.contains(p))
            && !content_lower.contains("implement")
            && !content_lower.contains("fix")
            && !content_lower.contains("add")
            && !content_lower.contains("create");

        if is_test_only {
            self.test_only_count += 1;
            debug!("Test-only activity (count: {})", self.test_only_count);
        } else if !content.is_empty() {
            self.test_only_count = 0;
        }

        // Check exit conditions
        if self.done_signal_count >= self.done_threshold {
            return Some(ExitReason::CompletionSignals);
        }

        if self.completion_indicator_count >= self.completion_threshold {
            return Some(ExitReason::ProjectComplete);
        }

        if self.test_only_count >= self.test_saturation_threshold {
            return Some(ExitReason::TestSaturation);
        }

        None
    }

    /// Check if a fix plan / task list is complete.
    /// Returns true if all checkboxes are checked.
    pub fn check_task_list_complete(&self, content: &str) -> bool {
        let lines: Vec<&str> = content.lines().collect();
        let mut has_checkboxes = false;
        let mut all_checked = true;

        for line in lines {
            let trimmed = line.trim();
            if trimmed.starts_with("- [ ]") || trimmed.starts_with("* [ ]") {
                has_checkboxes = true;
                all_checked = false;
                break;
            } else if trimmed.starts_with("- [x]")
                || trimmed.starts_with("- [X]")
                || trimmed.starts_with("* [x]")
                || trimmed.starts_with("* [X]")
            {
                has_checkboxes = true;
            }
        }

        has_checkboxes && all_checked
    }

    /// Reset the detector state.
    pub fn reset(&mut self) {
        self.done_signal_count = 0;
        self.test_only_count = 0;
        self.completion_indicator_count = 0;
        self.recent_content.clear();
    }

    /// Get current detection state as a summary.
    pub fn get_state(&self) -> ExitDetectorState {
        ExitDetectorState {
            done_signal_count: self.done_signal_count,
            test_only_count: self.test_only_count,
            completion_indicator_count: self.completion_indicator_count,
            recent_content_count: self.recent_content.len(),
        }
    }
}

/// Serializable exit detector state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitDetectorState {
    pub done_signal_count: u32,
    pub test_only_count: u32,
    pub completion_indicator_count: u32,
    pub recent_content_count: usize,
}

// ============================================================================
// Circuit Breaker (Ralph-inspired)
// ============================================================================

/// Error patterns to detect in output.
const ERROR_PATTERNS: &[&str] = &[
    "error:",
    "error!",
    "exception:",
    "exception!",
    "fatal:",
    "fatal!",
    "panic:",
    "failed:",
    "failure:",
    "traceback",
    "stack trace",
];

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // Tripped, blocking execution
    HalfOpen, // Testing if issue resolved
}

/// Result of a single loop/iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopResult {
    pub timestamp: DateTime<Utc>,
    pub files_changed: u32,
    pub errors_detected: u32,
    pub output_length: usize,
    pub tokens_used: i64,
    pub had_progress: bool,
}

/// Circuit breaker for detecting stagnation and repeated errors.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    state: CircuitState,
    /// Recent loop results for pattern analysis
    loop_history: Vec<LoopResult>,
    /// Max history entries
    max_history: usize,
    /// Consecutive no-progress loops before opening
    no_progress_threshold: u32,
    /// Consecutive identical error loops before opening
    repeated_error_threshold: u32,
    /// Current no-progress count
    no_progress_count: u32,
    /// Current repeated error count
    repeated_error_count: u32,
    /// Last error signature for deduplication
    last_error_signature: Option<String>,
    /// Time circuit was opened
    opened_at: Option<DateTime<Utc>>,
    /// Reason circuit was opened
    open_reason: Option<String>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            loop_history: Vec::new(),
            max_history: 10,
            no_progress_threshold: 3,
            repeated_error_threshold: 5,
            no_progress_count: 0,
            repeated_error_count: 0,
            last_error_signature: None,
            opened_at: None,
            open_reason: None,
        }
    }
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if execution is allowed.
    pub fn is_closed(&self) -> bool {
        self.state == CircuitState::Closed
    }

    /// Check if circuit is open (blocking).
    pub fn is_open(&self) -> bool {
        self.state == CircuitState::Open
    }

    /// Get current state.
    pub fn state(&self) -> CircuitState {
        self.state
    }

    /// Record the result of a loop/iteration.
    /// Returns true if circuit should open.
    pub fn record_result(&mut self, content: &str, files_changed: u32, tokens_used: i64) -> bool {
        let content_lower = content.to_lowercase();

        // Count errors in output
        let errors_detected = ERROR_PATTERNS
            .iter()
            .filter(|p| content_lower.contains(*p))
            .count() as u32;

        // Create error signature for deduplication
        let error_signature = if errors_detected > 0 {
            // Extract first error line as signature
            content_lower
                .lines()
                .find(|line| ERROR_PATTERNS.iter().any(|p| line.contains(*p)))
                .map(|s| s.to_string())
        } else {
            None
        };

        // Check for progress
        let had_progress = files_changed > 0 || tokens_used > 1000;

        let result = LoopResult {
            timestamp: Utc::now(),
            files_changed,
            errors_detected,
            output_length: content.len(),
            tokens_used,
            had_progress,
        };

        // Add to history
        self.loop_history.push(result);
        if self.loop_history.len() > self.max_history {
            self.loop_history.remove(0);
        }

        // Check no-progress condition
        if !had_progress {
            self.no_progress_count += 1;
            debug!("No progress detected (count: {})", self.no_progress_count);
        } else {
            self.no_progress_count = 0;
        }

        // Check repeated error condition
        if let Some(ref sig) = error_signature {
            if Some(sig.clone()) == self.last_error_signature {
                self.repeated_error_count += 1;
                debug!("Repeated error detected (count: {})", self.repeated_error_count);
            } else {
                self.repeated_error_count = 1;
            }
            self.last_error_signature = Some(sig.clone());
        } else {
            self.repeated_error_count = 0;
            self.last_error_signature = None;
        }

        // Check if should open
        if self.no_progress_count >= self.no_progress_threshold {
            self.open("No progress detected for {} consecutive loops".to_string());
            return true;
        }

        if self.repeated_error_count >= self.repeated_error_threshold {
            self.open("Same error repeated {} times".to_string());
            return true;
        }

        false
    }

    /// Open the circuit breaker.
    fn open(&mut self, reason: String) {
        self.state = CircuitState::Open;
        self.opened_at = Some(Utc::now());
        self.open_reason = Some(reason.clone());
        warn!("Circuit breaker opened: {}", reason);
    }

    /// Reset/close the circuit breaker.
    pub fn reset(&mut self) {
        self.state = CircuitState::Closed;
        self.no_progress_count = 0;
        self.repeated_error_count = 0;
        self.last_error_signature = None;
        self.opened_at = None;
        self.open_reason = None;
        info!("Circuit breaker reset");
    }

    /// Get circuit breaker state summary.
    pub fn get_state(&self) -> CircuitBreakerState {
        CircuitBreakerState {
            state: self.state,
            no_progress_count: self.no_progress_count,
            repeated_error_count: self.repeated_error_count,
            opened_at: self.opened_at,
            open_reason: self.open_reason.clone(),
            loop_history_count: self.loop_history.len(),
        }
    }
}

/// Serializable circuit breaker state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerState {
    pub state: CircuitState,
    pub no_progress_count: u32,
    pub repeated_error_count: u32,
    pub opened_at: Option<DateTime<Utc>>,
    pub open_reason: Option<String>,
    pub loop_history_count: usize,
}

// ============================================================================
// Rate Limiting & API Usage Tracking (Ralph-inspired)
// ============================================================================

/// Rate limiter for API call management.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    /// Whether rate limiting is disabled (unlimited mode)
    pub disabled: bool,
    /// Calls made in current hour
    calls_this_hour: u32,
    /// Maximum calls per hour
    max_calls_per_hour: u32,
    /// Hour when counter was last reset (YYYYMMDDHH format)
    last_reset_hour: String,
    /// Total calls made
    total_calls: u64,
    /// Tokens used this hour
    tokens_this_hour: i64,
    /// Maximum tokens per hour (if any)
    max_tokens_per_hour: Option<i64>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self {
            disabled: false,
            calls_this_hour: 0,
            max_calls_per_hour: 100, // Ralph default
            last_reset_hour: Self::current_hour_string(),
            total_calls: 0,
            tokens_this_hour: 0,
            max_tokens_per_hour: None,
        }
    }
}

impl RateLimiter {
    pub fn new(max_calls_per_hour: u32) -> Self {
        Self {
            max_calls_per_hour,
            ..Default::default()
        }
    }

    /// Create a rate limiter with unlimited/disabled mode.
    pub fn unlimited() -> Self {
        Self {
            disabled: true,
            max_calls_per_hour: u32::MAX,
            ..Default::default()
        }
    }

    fn current_hour_string() -> String {
        Utc::now().format("%Y%m%d%H").to_string()
    }

    /// Check if current hour has changed and reset if needed.
    fn maybe_reset_hour(&mut self) {
        let current = Self::current_hour_string();
        if current != self.last_reset_hour {
            debug!(
                "Hour changed from {} to {}, resetting counters",
                self.last_reset_hour, current
            );
            self.calls_this_hour = 0;
            self.tokens_this_hour = 0;
            self.last_reset_hour = current;
        }
    }

    /// Check if a call can be made.
    pub fn can_make_call(&mut self) -> bool {
        if self.disabled {
            return true; // Unlimited mode - always allow
        }
        self.maybe_reset_hour();
        self.calls_this_hour < self.max_calls_per_hour
    }

    /// Enable or disable rate limiting.
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        if disabled {
            info!("Rate limiting DISABLED - unlimited mode active");
        } else {
            info!("Rate limiting ENABLED - max {} calls/hour", self.max_calls_per_hour);
        }
    }

    /// Check if rate limiting is disabled.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Record a call being made.
    pub fn record_call(&mut self, tokens: i64) {
        self.maybe_reset_hour();
        self.calls_this_hour += 1;
        self.total_calls += 1;
        self.tokens_this_hour += tokens;
    }

    /// Get remaining calls this hour.
    pub fn remaining_calls(&mut self) -> u32 {
        self.maybe_reset_hour();
        self.max_calls_per_hour.saturating_sub(self.calls_this_hour)
    }

    /// Get seconds until next hour reset.
    pub fn seconds_until_reset(&self) -> i64 {
        let now = Utc::now();
        // Calculate seconds remaining in current hour
        let minutes_remaining = 59 - now.minute();
        let seconds_remaining = 60 - now.second();
        (minutes_remaining * 60 + seconds_remaining) as i64
    }

    /// Get rate limiter state.
    pub fn get_state(&self) -> RateLimiterState {
        RateLimiterState {
            disabled: self.disabled,
            calls_this_hour: self.calls_this_hour,
            max_calls_per_hour: self.max_calls_per_hour,
            remaining_calls: self.max_calls_per_hour.saturating_sub(self.calls_this_hour),
            total_calls: self.total_calls,
            tokens_this_hour: self.tokens_this_hour,
            seconds_until_reset: self.seconds_until_reset(),
        }
    }
}

/// Serializable rate limiter state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimiterState {
    pub disabled: bool,
    pub calls_this_hour: u32,
    pub max_calls_per_hour: u32,
    pub remaining_calls: u32,
    pub total_calls: u64,
    pub tokens_this_hour: i64,
    pub seconds_until_reset: i64,
}

// ============================================================================
// Session Analytics Manager
// ============================================================================

/// Per-session analytics tracking.
#[derive(Debug)]
pub struct SessionAnalytics {
    pub session_id: String,
    pub exit_detector: ExitDetector,
    pub circuit_breaker: CircuitBreaker,
    pub loop_count: u64,
    pub files_changed_total: u32,
    pub errors_total: u32,
    pub last_activity: DateTime<Utc>,
}

impl SessionAnalytics {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            exit_detector: ExitDetector::new(),
            circuit_breaker: CircuitBreaker::new(),
            loop_count: 0,
            files_changed_total: 0,
            errors_total: 0,
            last_activity: Utc::now(),
        }
    }

    /// Increment loop counter and update stats.
    pub fn increment_loop(&mut self, files_changed: u32, errors: u32) {
        self.loop_count += 1;
        self.files_changed_total += files_changed;
        self.errors_total += errors;
        self.last_activity = Utc::now();
    }

    /// Record the result of a loop iteration.
    pub fn record_loop_result(&mut self, output: &str, files_changed: u32, tokens_used: i64) -> bool {
        self.increment_loop(files_changed, 0);
        self.circuit_breaker.record_result(output, files_changed, tokens_used)
    }
}

/// Analytics manager for all sessions.
pub struct AnalyticsManager {
    sessions: Arc<RwLock<HashMap<String, SessionAnalytics>>>,
    rate_limiter: Arc<RwLock<RateLimiter>>,
    status_file: Option<PathBuf>,
}

impl AnalyticsManager {
    pub fn new(max_calls_per_hour: u32) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            rate_limiter: Arc::new(RwLock::new(RateLimiter::new(max_calls_per_hour))),
            status_file: None,
        }
    }

    /// Set the status file path for JSON output.
    pub fn set_status_file(&mut self, path: PathBuf) {
        self.status_file = Some(path);
    }

    /// Get or create analytics for a session.
    pub async fn get_session(&self, session_id: &str) -> SessionAnalytics {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .cloned()
            .unwrap_or_else(|| SessionAnalytics::new(session_id))
    }

    /// Process an event and update analytics.
    pub async fn process_event(&self, event: &SessionEvent) -> Option<ExitReason> {
        let mut sessions = self.sessions.write().await;
        let analytics = sessions
            .entry(event.session_id.clone())
            .or_insert_with(|| SessionAnalytics::new(&event.session_id));

        analytics.last_activity = Utc::now();

        // Update rate limiter
        if let Some(tokens) = event.tokens_input {
            let mut limiter = self.rate_limiter.write().await;
            limiter.record_call(tokens + event.tokens_output.unwrap_or(0));
        }

        // Run exit detection
        let exit_reason = analytics.exit_detector.analyze_event(event);

        // Update circuit breaker for file-related events
        if event.event_type == EventType::FileModified {
            analytics.files_changed_total += 1;
        }

        if event.event_type == EventType::Error {
            analytics.errors_total += 1;
        }

        exit_reason
    }

    /// Record a loop result for circuit breaker analysis.
    pub async fn record_loop(&self, session_id: &str, content: &str, files_changed: u32, tokens: i64) -> bool {
        let mut sessions = self.sessions.write().await;
        let analytics = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionAnalytics::new(session_id));

        analytics.loop_count += 1;
        analytics.circuit_breaker.record_result(content, files_changed, tokens)
    }

    /// Check if rate limit allows execution.
    pub async fn can_execute(&self) -> bool {
        let mut limiter = self.rate_limiter.write().await;
        limiter.can_make_call()
    }

    /// Get the overall status for JSON export.
    pub async fn get_status(&self) -> AnalyticsStatus {
        let sessions = self.sessions.read().await;
        let limiter = self.rate_limiter.read().await;

        let session_states: HashMap<String, SessionAnalyticsState> = sessions
            .iter()
            .map(|(id, a)| {
                (
                    id.clone(),
                    SessionAnalyticsState {
                        loop_count: a.loop_count,
                        files_changed_total: a.files_changed_total,
                        errors_total: a.errors_total,
                        exit_detector: a.exit_detector.get_state(),
                        circuit_breaker: a.circuit_breaker.get_state(),
                        last_activity: a.last_activity,
                    },
                )
            })
            .collect();

        AnalyticsStatus {
            timestamp: Utc::now(),
            rate_limiter: limiter.get_state(),
            sessions: session_states,
            active_session_count: sessions.len(),
        }
    }

    /// Write status to JSON file.
    pub async fn write_status_file(&self) -> anyhow::Result<()> {
        if let Some(ref path) = self.status_file {
            let status = self.get_status().await;
            let json = serde_json::to_string_pretty(&status)?;
            tokio::fs::write(path, json).await?;
        }
        Ok(())
    }

    /// Reset circuit breaker for a session.
    pub async fn reset_circuit_breaker(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(analytics) = sessions.get_mut(session_id) {
            analytics.circuit_breaker.reset();
        }
    }
}

impl Clone for SessionAnalytics {
    fn clone(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            exit_detector: self.exit_detector.clone(),
            circuit_breaker: self.circuit_breaker.clone(),
            loop_count: self.loop_count,
            files_changed_total: self.files_changed_total,
            errors_total: self.errors_total,
            last_activity: self.last_activity,
        }
    }
}

/// Per-session analytics state for JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAnalyticsState {
    pub loop_count: u64,
    pub files_changed_total: u32,
    pub errors_total: u32,
    pub exit_detector: ExitDetectorState,
    pub circuit_breaker: CircuitBreakerState,
    pub last_activity: DateTime<Utc>,
}

/// Overall analytics status for JSON export (compatible with Ralph's status.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsStatus {
    pub timestamp: DateTime<Utc>,
    pub rate_limiter: RateLimiterState,
    pub sessions: HashMap<String, SessionAnalyticsState>,
    pub active_session_count: usize,
}

// ============================================================================
// Memory Persistence (Auto-Claude inspired)
// ============================================================================

/// Memory entry for cross-session persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub session_id: Option<String>,
    pub tags: Vec<String>,
}

/// Memory store for persistent insights across sessions.
#[derive(Debug)]
pub struct MemoryStore {
    entries: Arc<RwLock<HashMap<String, MemoryEntry>>>,
    storage_path: Option<PathBuf>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            storage_path: None,
        }
    }

    /// Set persistent storage path.
    pub fn set_storage_path(&mut self, path: PathBuf) {
        self.storage_path = Some(path);
    }

    /// Write a memory entry.
    pub async fn write(&self, key: &str, value: serde_json::Value, session_id: Option<&str>, tags: Vec<String>) {
        let mut entries = self.entries.write().await;
        let now = Utc::now();

        let entry = entries.entry(key.to_string()).or_insert_with(|| MemoryEntry {
            key: key.to_string(),
            value: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
            session_id: session_id.map(|s| s.to_string()),
            tags: vec![],
        });

        entry.value = value;
        entry.updated_at = now;
        entry.tags = tags;
    }

    /// Read a memory entry.
    pub async fn read(&self, key: &str) -> Option<MemoryEntry> {
        let entries = self.entries.read().await;
        entries.get(key).cloned()
    }

    /// List all memory entries.
    pub async fn list(&self) -> Vec<MemoryEntry> {
        let entries = self.entries.read().await;
        entries.values().cloned().collect()
    }

    /// Delete a memory entry.
    pub async fn delete(&self, key: &str) -> bool {
        let mut entries = self.entries.write().await;
        entries.remove(key).is_some()
    }

    /// Save to persistent storage.
    pub async fn persist(&self) -> anyhow::Result<()> {
        if let Some(ref path) = self.storage_path {
            let entries = self.entries.read().await;
            let json = serde_json::to_string_pretty(&*entries)?;
            tokio::fs::write(path, json).await?;
        }
        Ok(())
    }

    /// Load from persistent storage.
    pub async fn load(&self) -> anyhow::Result<()> {
        if let Some(ref path) = self.storage_path {
            if path.exists() {
                let json = tokio::fs::read_to_string(path).await?;
                let loaded: HashMap<String, MemoryEntry> = serde_json::from_str(&json)?;
                let mut entries = self.entries.write().await;
                *entries = loaded;
            }
        }
        Ok(())
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_detector_done_signals() {
        let mut detector = ExitDetector::new();

        // First done signal
        let mut event = SessionEvent::new("test", EventType::ResponseGenerated, crate::models::AgentType::ClaudeCode);
        event.content = Some("All tasks completed successfully!".to_string());
        assert!(detector.analyze_event(&event).is_none());

        // Second done signal should trigger exit
        event.content = Some("All done, everything is working!".to_string());
        let result = detector.analyze_event(&event);
        assert_eq!(result, Some(ExitReason::CompletionSignals));
    }

    #[test]
    fn test_circuit_breaker_no_progress() {
        let mut cb = CircuitBreaker::new();

        // Three loops with no progress
        assert!(!cb.record_result("nothing changed", 0, 100));
        assert!(!cb.record_result("still nothing", 0, 100));
        assert!(cb.record_result("no changes again", 0, 100));

        assert!(cb.is_open());
    }

    #[test]
    fn test_rate_limiter() {
        let mut limiter = RateLimiter::new(10);

        for _ in 0..10 {
            assert!(limiter.can_make_call());
            limiter.record_call(1000);
        }

        assert!(!limiter.can_make_call());
        assert_eq!(limiter.remaining_calls(), 0);
    }

    #[test]
    fn test_task_list_completion() {
        let detector = ExitDetector::new();

        let incomplete = r#"
        - [x] Task 1
        - [ ] Task 2
        - [x] Task 3
        "#;
        assert!(!detector.check_task_list_complete(incomplete));

        let complete = r#"
        - [x] Task 1
        - [x] Task 2
        - [X] Task 3
        "#;
        assert!(detector.check_task_list_complete(complete));
    }

    #[test]
    fn test_rate_limiter_unlimited_mode() {
        let mut limiter = RateLimiter::unlimited();

        // Should always allow calls in unlimited mode
        for _ in 0..1000 {
            assert!(limiter.can_make_call());
            limiter.record_call(10000);
        }

        // Still unlimited after many calls
        assert!(limiter.can_make_call());
        assert!(limiter.is_disabled());
    }

    #[test]
    fn test_rate_limiter_toggle_disabled() {
        let mut limiter = RateLimiter::new(5);

        // Use up all calls
        for _ in 0..5 {
            limiter.record_call(100);
        }
        assert!(!limiter.can_make_call());

        // Disable rate limiting
        limiter.set_disabled(true);
        assert!(limiter.can_make_call()); // Now unlimited

        // Re-enable
        limiter.set_disabled(false);
        assert!(!limiter.can_make_call()); // Back to limited
    }

    #[test]
    fn test_circuit_breaker_repeated_errors() {
        let mut cb = CircuitBreaker::new();

        // Same error 5 times
        let same_error = "Error: Connection refused";
        for i in 0..4 {
            assert!(!cb.record_result(same_error, 1, 100), "Should not trigger on attempt {}", i);
        }
        assert!(cb.record_result(same_error, 1, 100), "Should trigger on 5th identical error");
        assert!(cb.is_open());
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let mut cb = CircuitBreaker::new();

        // Trigger circuit breaker
        for _ in 0..3 {
            cb.record_result("no progress", 0, 100);
        }
        assert!(cb.is_open());

        // Reset
        cb.reset();
        assert!(!cb.is_open());
        assert_eq!(cb.no_progress_count, 0);
    }

    #[test]
    fn test_exit_detector_strong_completion() {
        let mut detector = ExitDetector::new();

        // Strong completion signal should trigger immediately
        let mut event = SessionEvent::new("test", EventType::ResponseGenerated, crate::models::AgentType::ClaudeCode);
        event.content = Some("Implementation complete. Ready for review!".to_string());
        let result = detector.analyze_event(&event);
        assert_eq!(result, Some(ExitReason::StrongCompletion));
    }

    #[test]
    fn test_exit_detector_test_saturation() {
        let mut detector = ExitDetector::new();

        let mut event = SessionEvent::new("test", EventType::ToolExecuted, crate::models::AgentType::ClaudeCode);

        // Three loops of only running tests
        for _ in 0..2 {
            event.content = Some("Running cargo test...".to_string());
            assert!(detector.analyze_event(&event).is_none());
        }

        event.content = Some("Running pytest...".to_string());
        let result = detector.analyze_event(&event);
        assert_eq!(result, Some(ExitReason::TestSaturation));
    }

    #[tokio::test]
    async fn test_memory_store_basic() {
        let store = MemoryStore::new();

        store.write("key1", serde_json::json!("value1"), None, vec!["tag1".to_string()]).await;
        store.write("key2", serde_json::json!("value2"), Some("session1"), vec![]).await;

        let entry = store.read("key1").await;
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().value, serde_json::json!("value1"));

        let list = store.list().await;
        assert_eq!(list.len(), 2);

        assert!(store.delete("key1").await);
        assert!(store.read("key1").await.is_none());
    }

    #[test]
    fn test_session_analytics_new() {
        let analytics = SessionAnalytics::new("test-session");
        assert_eq!(analytics.session_id, "test-session");
        assert_eq!(analytics.loop_count, 0);
        assert_eq!(analytics.files_changed_total, 0);
    }

    #[test]
    fn test_session_analytics_increment_loop() {
        let mut analytics = SessionAnalytics::new("test");
        analytics.increment_loop(5, 2);
        assert_eq!(analytics.loop_count, 1);
        assert_eq!(analytics.files_changed_total, 5);
        assert_eq!(analytics.errors_total, 2);
    }
}
