//! Flap suppression (M3-02, proposal 0005 §2.2, D-63): a check's alert state
//! changes only once enough results in a row say so, so one lost ping or one
//! CPU spike alerts nobody, and a flapping link doesn't send a "down" and a
//! "recovered" e-mail every other minute.
//!
//! A pure state machine: [`next`] takes the stored state, one result's
//! status (as the hub judged it, D-64) and the check's [`AlertAfter`], and
//! says what the state becomes and whether that is news. Storage and e-mail
//! are the engine's business.

use crate::model::{AlertAfter, Status};

/// A check's alert state: the status people were told (or would have been),
/// and how many results in a row since then said otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlapState {
    /// Up, Warning or Down.
    pub confirmed: Status,
    /// Results in a row that disagree with `confirmed`.
    pub streak: u32,
}

impl Default for FlapState {
    /// A check never seen counts as Up.
    fn default() -> Self {
        Self {
            confirmed: Status::Up,
            streak: 0,
        }
    }
}

/// The state after one more result with `status`; `Some(new)` when the
/// confirmed status changed (the news to tell). Enough results in a row
/// that disagree with what was confirmed change it to the latest one: back
/// to Up takes [`AlertAfter::recoveries`] of them, to Warning or Down
/// [`AlertAfter::failures`]. A check down from the start alerts after
/// `failures` results like any other.
pub fn next(
    state: Option<FlapState>,
    status: Status,
    policy: AlertAfter,
) -> (FlapState, Option<Status>) {
    let state = state.unwrap_or_default();
    if status == state.confirmed {
        // Agrees with what was confirmed: any streak against it is over.
        return (
            FlapState {
                confirmed: status,
                streak: 0,
            },
            None,
        );
    }
    let streak = state.streak + 1;
    let needed = match status {
        Status::Up => policy.recoveries(),
        _ => policy.failures(),
    };
    if streak < needed {
        return (
            FlapState {
                confirmed: state.confirmed,
                streak,
            },
            None,
        );
    }
    (
        FlapState {
            confirmed: status,
            streak: 0,
        },
        Some(status),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(fail_after: u32, recover_after: u32) -> AlertAfter {
        AlertAfter {
            fail_after: Some(fail_after),
            recover_after: Some(recover_after),
        }
    }

    fn status(c: char) -> Status {
        match c {
            'U' => Status::Up,
            'W' => Status::Warning,
            _ => Status::Down,
        }
    }

    /// Run `results` (`U`, `W`, `D`) from no state; the news, as a string
    /// (`.` for none, else the new status's letter).
    fn run(results: &str, policy: AlertAfter) -> String {
        let mut state = None;
        let mut out = String::new();
        for c in results.chars() {
            let (after, news) = next(state, status(c), policy);
            out.push(news.map_or('.', |s| s.as_str().chars().next().unwrap()));
            state = Some(after);
        }
        out
    }

    #[test]
    fn by_default_two_in_a_row_change_it() {
        let default = AlertAfter::default();
        assert_eq!(run("UUUU", default), "....", "healthy: nothing");
        assert_eq!(run("UDUDUDU", default), ".......", "flapping: nothing");
        assert_eq!(
            run("UDDDUUU", default),
            "..D..U.",
            "an outage: one each way"
        );
        assert_eq!(run("DD", default), ".D", "down from the start alerts too");
        assert_eq!(
            run("UDDUDUU", default),
            "..D...U",
            "a blip back up isn't recovery"
        );
    }

    #[test]
    fn warnings_count_the_same_way() {
        let default = AlertAfter::default();
        assert_eq!(run("UWUWU", default), ".....", "a spike: nothing");
        assert_eq!(run("UWWWUU", default), "..W..U");
        assert_eq!(run("UWDD", default), "..D.", "worse and worse: the latest");
        assert_eq!(run("DDWW", default), ".D.W", "better, but not up");
        assert_eq!(run("WWDWD", default), ".W...", "a down blip while warning");
    }

    #[test]
    fn one_means_at_once_as_before() {
        assert_eq!(run("UDUDU", policy(1, 1)), ".DUDU");
        assert_eq!(run("D", policy(1, 1)), "D");
        assert_eq!(run("WD", policy(1, 1)), "WD");
    }

    #[test]
    fn failing_and_recovering_count_separately() {
        assert_eq!(run("DDDUDUUUU", policy(3, 4)), "..D.....U");
        assert_eq!(run("DUUD", policy(1, 3)), "D...");
        assert_eq!(run("DUUU", policy(1, 3)), "D..U");
    }

    /// Every Up/Down sequence of up to 8 results, for every policy up to 4:
    /// news comes exactly when the last `n` results agree and differ from
    /// the status confirmed before them, and alternates.
    #[test]
    fn exhaustively_matches_its_definition_for_up_and_down() {
        for fail_after in 1..=4 {
            for recover_after in 1..=4 {
                let p = policy(fail_after, recover_after);
                for len in 0..=8u32 {
                    for bits in 0..(1u32 << len) {
                        let results: Vec<bool> = (0..len).map(|i| bits >> i & 1 == 1).collect();
                        let (mut state, mut confirmed, mut last) = (None, false, None);
                        for (i, &down) in results.iter().enumerate() {
                            let s = if down { Status::Down } else { Status::Up };
                            let (after, news) = next(state, s, p);
                            let needed = if down { fail_after } else { recover_after } as usize;
                            let run_len = results[..=i]
                                .iter()
                                .rev()
                                .take_while(|&&r| r == down)
                                .count();
                            let expected = down != confirmed && run_len >= needed;
                            assert_eq!(news.is_some(), expected, "{results:?} at {i}, {p:?}");
                            if expected {
                                confirmed = down;
                                assert_ne!(last, news, "news alternates");
                                last = news;
                            }
                            assert_eq!(after.confirmed == Status::Down, confirmed);
                            state = Some(after);
                        }
                    }
                }
            }
        }
    }

    /// Every Up/Warning/Down sequence of up to 6 results, for every policy up
    /// to 3: news is always the result that brought it, never a repeat of
    /// what was confirmed, and a run of `max(fail, recover)` equal results
    /// always ends confirmed as that status.
    #[test]
    fn three_statuses_keep_their_promises() {
        const ALL: [Status; 3] = [Status::Up, Status::Warning, Status::Down];
        for fail_after in 1..=3 {
            for recover_after in 1..=3 {
                let p = policy(fail_after, recover_after);
                for len in 0..=6u32 {
                    for mut code in 0..3u32.pow(len) {
                        let mut results = Vec::new();
                        for _ in 0..len {
                            results.push(ALL[(code % 3) as usize]);
                            code /= 3;
                        }
                        let mut state: Option<FlapState> = None;
                        for (i, &s) in results.iter().enumerate() {
                            let before = state.unwrap_or_default().confirmed;
                            let (after, news) = next(state, s, p);
                            if let Some(new) = news {
                                assert_eq!(new, s, "{results:?} at {i}");
                                assert_ne!(new, before, "{results:?} at {i}");
                            }
                            if s == before {
                                assert_eq!(news, None);
                            }
                            let run = results[..=i].iter().rev().take_while(|&&r| r == s).count();
                            if run >= fail_after.max(recover_after) as usize {
                                assert_eq!(after.confirmed, s, "{results:?} at {i}");
                            }
                            state = Some(after);
                        }
                    }
                }
            }
        }
    }
}
