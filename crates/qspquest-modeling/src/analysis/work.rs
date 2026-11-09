//! Bounded ordered dispatch of independent work units on native threads.
//!
//! At most `workers` units run beyond the committed prefix. Results are
//! committed on the calling thread in input order. The first failing unit,
//! in input order, stops dispatch; running units finish before return. A
//! panicking unit fails like any other unit instead of stalling the commit.
use super::common::{AnalysisError, Result};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Condvar, Mutex};

struct State<R> {
    next: usize,
    committed: usize,
    abort: bool,
    stop_at: usize,
    results: Vec<Option<Result<R>>>,
}

pub fn ordered<T, R>(
    items: Vec<T>,
    workers: usize,
    execute: &(dyn Fn(T) -> Result<R> + Sync),
    commit: &mut dyn FnMut(R) -> Result<()>,
) -> Result<()>
where
    T: Send,
    R: Send,
{
    let total = items.len();
    if workers <= 1 || total <= 1 || cfg!(target_arch = "wasm32") {
        for item in items {
            commit(execute(item)?)?;
        }
        return Ok(());
    }
    let items: Mutex<Vec<Option<T>>> = Mutex::new(items.into_iter().map(Some).collect());
    let state = Mutex::new(State {
        next: 0,
        committed: 0,
        abort: false,
        stop_at: total,
        results: (0..total).map(|_| None).collect(),
    });
    let ready = Condvar::new();
    let mut outcome = Ok(());
    std::thread::scope(|scope| {
        for _ in 0..workers.min(total) {
            scope.spawn(|| loop {
                let index = {
                    let mut s = state.lock().expect("work state");
                    loop {
                        if s.abort || s.next >= s.stop_at {
                            return;
                        }
                        if s.next < s.committed + workers {
                            break;
                        }
                        s = ready.wait(s).expect("work state");
                    }
                    s.next += 1;
                    s.next - 1
                };
                let item = items.lock().expect("work items")[index]
                    .take()
                    .expect("each unit is taken once");
                let result =
                    catch_unwind(AssertUnwindSafe(|| execute(item))).unwrap_or_else(|_| {
                        Err(AnalysisError::new(
                            "internal",
                            "an analysis work unit panicked; report this as a bug",
                        ))
                    });
                let mut s = state.lock().expect("work state");
                if result.is_err() {
                    // Stop new dispatch; earlier units are already dispatched
                    // and still commit in order.
                    s.stop_at = s.stop_at.min(index + 1);
                }
                s.results[index] = Some(result);
                ready.notify_all();
            });
        }
        let mut position = 0;
        while position < total {
            let result = {
                let mut s = state.lock().expect("work state");
                loop {
                    if let Some(result) = s.results[position].take() {
                        break result;
                    }
                    s = ready.wait(s).expect("work state");
                }
            };
            let committed = result.and_then(|r| commit(r));
            let mut s = state.lock().expect("work state");
            if let Err(error) = committed {
                s.abort = true;
                ready.notify_all();
                outcome = Err(error);
                break;
            }
            position += 1;
            s.committed = position;
            ready.notify_all();
        }
    });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_unit_fails_the_call_instead_of_hanging() {
        let mut seen = Vec::new();
        let error = ordered(
            (0..12).collect(),
            4,
            &|i: usize| {
                if i == 5 {
                    panic!("unit five");
                }
                Ok(i)
            },
            &mut |r| {
                seen.push(r);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.to_value()["code"], "internal");
        assert_eq!(seen, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn results_commit_in_order_for_any_worker_count() {
        for workers in [1, 2, 5, 8] {
            let mut seen = Vec::new();
            ordered(
                (0..20).collect(),
                workers,
                &|i: usize| Ok(i * i),
                &mut |r| {
                    seen.push(r);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(seen, (0..20).map(|i| i * i).collect::<Vec<_>>());
        }
    }

    #[test]
    fn in_flight_work_is_bounded_beyond_the_committed_prefix() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let started = AtomicUsize::new(0);
        let committed = AtomicUsize::new(0);
        let widest = AtomicUsize::new(0);
        ordered(
            (0..40).collect(),
            3,
            &|i: usize| {
                let ahead =
                    started.fetch_add(1, Ordering::SeqCst) + 1 - committed.load(Ordering::SeqCst);
                widest.fetch_max(ahead, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_micros(200));
                Ok(i)
            },
            &mut |_| {
                committed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        assert!(widest.into_inner() <= 3);
        assert_eq!(committed.into_inner(), 40);
    }

    #[test]
    fn first_failure_in_order_is_returned_after_earlier_commits() {
        let mut seen = Vec::new();
        let error = ordered(
            (0..10).collect(),
            4,
            &|i: usize| {
                if i == 3 {
                    Err(AnalysisError::new("budget", "stop"))
                } else {
                    Ok(i)
                }
            },
            &mut |r| {
                seen.push(r);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "budget");
        assert_eq!(seen, vec![0, 1, 2]);
    }
}
