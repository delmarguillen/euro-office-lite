// Timestamps of one document open, one log line per phase:
//
//   [OPEN] phase=<name> open=<id> t=<ms> dt=<ms>
//
// t counts from the first mark of the open and dt from the previous mark, both
// in whole milliseconds of a monotonic clock. The id comes from the page,
// which logs its own phases under it with its own clock (bridge.js,
// _eoOpenPhases). The first line of each side also carries
// wall=<ms since the Unix epoch>: the difference between the two places one
// clock on the other (scripts/measure-open.mjs).
//
// The phases of the backend, in order:
//   command  the command starts
//   start    the previous document is gone and the format is known
//   copy     the private copy x2t reads is written
//   convert  x2t has finished
//   encode   the result is read and encoded as base64
//   respond  the command is about to answer; `failed` in its place on an error
//
// A PDF is not converted: it marks command, copy (the file is read and its
// copy kept), encode and respond.
//
// The lines only add to the log: `[OPEN] start`, `[OPEN] success` and
// `[OPEN] failed` are written as before.

use crate::file_ops::{log_event, AppState};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const ID_MAX_LEN: usize = 32;

// The id goes into the log as the page sent it, so only letters, digits and
// hyphens are taken: nothing that could start another line or another field.
fn accepted_id(open_id: Option<String>) -> Option<String> {
    let id = open_id?;
    if id.is_empty() || id.len() > ID_MAX_LEN {
        return None;
    }
    let plain = id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    plain.then_some(id)
}

// The lines of one open, given the time of each mark. Apart from the clock so
// the lines can be checked with chosen times.
struct PhaseLines {
    id: String,
    previous_ms: u128,
}

impl PhaseLines {
    fn new(id: String) -> Self {
        PhaseLines { id, previous_ms: 0 }
    }

    fn line(&mut self, phase: &str, elapsed_ms: u128) -> String {
        let dt = elapsed_ms.saturating_sub(self.previous_ms);
        self.previous_ms = elapsed_ms;
        format!("[OPEN] phase={} open={} t={} dt={}", phase, self.id, elapsed_ms, dt)
    }
}

pub struct OpenPhases {
    started: Instant,
    // None when the page gave no usable id: nothing is logged.
    lines: Option<PhaseLines>,
}

impl OpenPhases {
    fn new(open_id: Option<String>) -> Self {
        OpenPhases {
            started: Instant::now(),
            lines: accepted_id(open_id).map(PhaseLines::new),
        }
    }

    // A conversion that is not an open (a blank document): no lines.
    pub fn untracked() -> Self {
        OpenPhases::new(None)
    }

    pub fn begin(state: &AppState, open_id: Option<String>) -> Self {
        let mut phases = OpenPhases::new(open_id);
        if let Some(lines) = &mut phases.lines {
            let line = lines.line("command", 0);
            log_event(state, &format!("{} wall={}", line, wall_ms()));
        }
        phases
    }

    pub fn mark(&mut self, state: &AppState, phase: &str) {
        let Some(lines) = &mut self.lines else {
            return;
        };
        let elapsed_ms = self.started.elapsed().as_millis();
        log_event(state, &lines.line(phase, elapsed_ms));
    }

    // The last mark of a command, taken from what it is about to answer.
    pub fn finish<T>(&mut self, state: &AppState, answer: &Result<T, String>) {
        let phase = if answer.is_ok() { "respond" } else { "failed" };
        self.mark(state, phase);
    }
}

fn wall_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(id: &str) -> PhaseLines {
        PhaseLines::new(id.to_string())
    }

    #[test]
    fn a_line_has_the_phase_the_open_and_both_times() {
        let mut open = lines("m3k9x2ab-1");
        assert_eq!(
            open.line("copy", 12),
            "[OPEN] phase=copy open=m3k9x2ab-1 t=12 dt=12"
        );
    }

    #[test]
    fn dt_is_the_time_since_the_previous_phase_of_the_same_open() {
        let mut first = lines("a-1");
        let mut second = lines("a-2");

        assert_eq!(first.line("command", 0), "[OPEN] phase=command open=a-1 t=0 dt=0");
        assert_eq!(first.line("copy", 40), "[OPEN] phase=copy open=a-1 t=40 dt=40");
        // Another open in between does not move the first one's previous phase.
        assert_eq!(second.line("command", 0), "[OPEN] phase=command open=a-2 t=0 dt=0");
        assert_eq!(second.line("copy", 7), "[OPEN] phase=copy open=a-2 t=7 dt=7");
        assert_eq!(first.line("convert", 2540), "[OPEN] phase=convert open=a-1 t=2540 dt=2500");
        assert_eq!(first.line("encode", 2575), "[OPEN] phase=encode open=a-1 t=2575 dt=35");
    }

    #[test]
    fn the_dts_of_an_open_add_up_to_its_last_t() {
        let mut open = lines("a-1");
        let times = [0u128, 3, 3, 250, 1900, 1907];
        let mut sum = 0;
        for (index, t) in times.iter().enumerate() {
            let line = open.line(&format!("p{index}"), *t);
            let dt = line.rsplit("dt=").next().unwrap().parse::<u128>().unwrap();
            sum += dt;
        }
        assert_eq!(sum, 1907);
    }

    // The test tools find a failed or finished open by these texts.
    #[test]
    fn a_phase_line_is_not_taken_for_a_start_success_or_failed_line() {
        let mut open = lines("a-1");
        for phase in ["start", "failed", "respond"] {
            let line = open.line(phase, 5);
            assert!(line.starts_with("[OPEN] phase="), "{line}");
            assert!(!line.contains("[OPEN] start"), "{line}");
            assert!(!line.contains("[OPEN] success"), "{line}");
            assert!(!line.contains("[OPEN] failed"), "{line}");
        }
    }

    #[test]
    fn only_a_plain_id_is_logged() {
        let accepted = |id: &str| accepted_id(Some(id.to_string()));
        assert_eq!(accepted("m3k9x2ab-1"), Some("m3k9x2ab-1".to_string()));
        assert_eq!(accepted(""), None);
        assert_eq!(accepted("a b"), None);
        assert_eq!(accepted("a\n[OPEN] failed"), None);
        assert_eq!(accepted("a=b"), None);
        assert_eq!(accepted(&"a".repeat(ID_MAX_LEN)), Some("a".repeat(ID_MAX_LEN)));
        assert_eq!(accepted(&"a".repeat(ID_MAX_LEN + 1)), None);
        assert_eq!(accepted_id(None), None);
    }

    #[test]
    fn an_open_without_an_id_logs_nothing() {
        assert!(OpenPhases::untracked().lines.is_none());
        assert!(OpenPhases::new(Some("a b".to_string())).lines.is_none());
        assert!(OpenPhases::new(Some("a-1".to_string())).lines.is_some());
    }
}
