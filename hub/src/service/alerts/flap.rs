//! Flap suppression (M3-02, proposal 0005 §2.2, D-63): a check's alert state
//! changes only once enough results in a row say so, so one lost ping or one
//! CPU spike alerts nobody, and a flapping link doesn't send a "down" and a
//! "recovered" e-mail every other minute.
//!
//! A pure state machine: [`next`] takes the stored state, one result and the
//! check's [`AlertAfter`], and says what the state becomes and whether that
//! is news. Storage and e-mail are the engine's business.

use crate::model::AlertAfter;

/// A check's alert state: what was confirmed, and how many results in a row
/// since then said otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlapState {
    /// Confirmed failing (Down, or over its threshold); `false`: confirmed fine.
    pub failing: bool,
    /// Results in a row that disagree with `failing`.
    pub streak: u32,
}

/// What one result changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Nothing people need to hear about (perhaps a streak started).
    None,
    /// Enough failed results in a row: it is failing.
    Failed,
    /// Enough good results in a row: it recovered.
    Recovered,
}

/// The state after one more result (`failing`: this result failed). A check
/// never seen before counts as fine, so a check that is down from the start
/// alerts after [`AlertAfter::failures`] results like any other.
pub fn next(
    state: Option<FlapState>,
    failing: bool,
    policy: AlertAfter,
) -> (FlapState, Transition) {
    let state = state.unwrap_or_default();
    if failing == state.failing {
        // Agrees with what was confirmed: any streak against it is over.
        return (FlapState { failing, streak: 0 }, Transition::None);
    }
    let streak = state.streak + 1;
    let needed = match failing {
        true => policy.failures(),
        false => policy.recoveries(),
    };
    if streak < needed {
        return (
            FlapState {
                failing: state.failing,
                streak,
            },
            Transition::None,
        );
    }
    let transition = match failing {
        true => Transition::Failed,
        false => Transition::Recovered,
    };
    (FlapState { failing, streak: 0 }, transition)
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

    /// Run `results` (`'D'` failed, `'U'` fine) from no state; the transitions, as a string.
    fn run(results: &str, policy: AlertAfter) -> String {
        let mut state = None;
        let mut out = String::new();
        for result in results.chars() {
            let (after, transition) = next(state, result == 'D', policy);
            out.push(match transition {
                Transition::None => '.',
                Transition::Failed => 'F',
                Transition::Recovered => 'R',
            });
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
            "..F..R.",
            "an outage: one each way"
        );
        assert_eq!(run("DD", default), ".F", "down from the start alerts too");
        assert_eq!(
            run("UDDUDUU", default),
            "..F...R",
            "a blip back up isn't recovery"
        );
    }

    #[test]
    fn one_means_at_once_as_before() {
        assert_eq!(run("UDUDU", policy(1, 1)), ".FRFR");
        assert_eq!(run("D", policy(1, 1)), "F");
    }

    #[test]
    fn failing_and_recovering_count_separately() {
        assert_eq!(run("DDDUDUUUU", policy(3, 4)), "..F.....R");
        assert_eq!(run("DUUD", policy(1, 3)), "F...");
        assert_eq!(run("DUUU", policy(1, 3)), "F..R");
    }

    #[test]
    fn a_streak_is_counted_and_reset() {
        let p = AlertAfter::default();
        let (state, _) = next(None, true, p);
        assert_eq!(
            state,
            FlapState {
                failing: false,
                streak: 1
            }
        );
        let (state, _) = next(Some(state), false, p);
        assert_eq!(
            state,
            FlapState {
                failing: false,
                streak: 0
            },
            "agreeing resets it"
        );
        let (state, t) = next(
            Some(FlapState {
                failing: false,
                streak: 1,
            }),
            true,
            p,
        );
        assert_eq!(
            (state, t),
            (
                FlapState {
                    failing: true,
                    streak: 0
                },
                Transition::Failed
            )
        );
    }

    /// Every sequence of up to 8 results, for every policy up to 4: a
    /// transition comes exactly when the last `n` results agree and differ
    /// from the state confirmed before them, and the transitions alternate.
    #[test]
    fn exhaustively_matches_its_definition() {
        for fail_after in 1..=4 {
            for recover_after in 1..=4 {
                let p = policy(fail_after, recover_after);
                for len in 0..=8u32 {
                    for bits in 0..(1u32 << len) {
                        let results: Vec<bool> = (0..len).map(|i| bits >> i & 1 == 1).collect();
                        let (mut state, mut confirmed, mut last) = (None, false, None);
                        for (i, &failing) in results.iter().enumerate() {
                            let (after, t) = next(state, failing, p);
                            let needed = if failing { fail_after } else { recover_after } as usize;
                            let run_len = results[..=i]
                                .iter()
                                .rev()
                                .take_while(|&&r| r == failing)
                                .count();
                            let expected = failing != confirmed && run_len >= needed;
                            assert_eq!(
                                t != Transition::None,
                                expected,
                                "{results:?} at {i}, {p:?}"
                            );
                            if expected {
                                confirmed = failing;
                                assert_ne!(last, Some(t), "transitions alternate");
                                last = Some(t);
                            }
                            assert_eq!(after.failing, confirmed);
                            state = Some(after);
                        }
                    }
                }
            }
        }
    }
}
