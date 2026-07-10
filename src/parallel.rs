//! Bounded parallelism for git shell-out probes.
//!
//! The scan and worktree drivers used to spawn one OS thread *per repository*
//! (`std::thread::scope` with one `spawn` per item) — an UNBOUNDED fan-out: a
//! root holding thousands of repos would create thousands of live threads and
//! file descriptors at once, risking exhaustion (#16). This module replaces that
//! with a fixed-size worker pool: at most `concurrency` threads are alive
//! regardless of item count, each pulling the next index from a shared atomic
//! counter. That is work-stealing, not fixed chunking, so one slow item never
//! idles the other workers.
//!
//! The design stays true to the shell-out model (see
//! `docs/architecture/01-discovery.md`): no new dependency, plain `std::thread`,
//! git remains the unit of work — only the *number of concurrent git processes*
//! is now capped.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use crate::error::GitError;

/// Error types used with [`try_map`] must absorb a caught worker panic without
/// aborting the scan (see tolerance rules in `docs/architecture/01-discovery.md`).
pub trait PanicRecover: Send + 'static {
    fn from_worker_panic(msg: &'static str) -> Self;
}

impl PanicRecover for anyhow::Error {
    fn from_worker_panic(msg: &'static str) -> Self {
        anyhow::anyhow!(msg)
    }
}

impl PanicRecover for GitError {
    fn from_worker_panic(msg: &'static str) -> Self {
        GitError::WorkerPanic(msg)
    }
}

/// Default worker cap: the machine's available parallelism (≈ `nproc`), never
/// below 1.
///
/// Git probes spawn a subprocess and mostly wait on it, so a modest amount of
/// oversubscription would keep cores busier — but the whole point of #16 is to
/// *bound* the fan-out, and `nproc` is the ceiling the issue calls for. It caps
/// concurrent git processes at a predictable, machine-proportional number while
/// still saturating a typical multi-core box.
pub fn default_concurrency() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Runs `f` over every item on a bounded pool of at most `concurrency` worker
/// threads (clamped to `[1, items.len()]`), returning the results POSITIONALLY
/// ALIGNED with `items`.
///
/// `f` returns `Result<R>`. A panic inside `f` for one item is caught and mapped
/// to `Err(anyhow!(panic_msg))` for THAT item only — every other item still
/// produces its result. This preserves the driver contract that a single
/// misbehaving repo becomes a warning, never an aborted scan (see the tolerance
/// rules in `docs/architecture/01-discovery.md`).
///
/// Ordering: workers claim indices out of order, but each result is scattered
/// back to its original slot, so `out[i]` is always the result of `f(&items[i])`.
pub fn try_map<T, R, E, F>(
    items: &[T],
    concurrency: usize,
    panic_msg: &'static str,
    f: F,
) -> Vec<std::result::Result<R, E>>
where
    T: Sync,
    R: Send,
    E: PanicRecover,
    F: Fn(&T) -> std::result::Result<R, E> + Sync,
{
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let workers = concurrency.clamp(1, n);
    let next = AtomicUsize::new(0);

    // Each worker returns the (index, result) pairs it happened to claim; we
    // scatter them into a positionally-aligned Vec afterward. Keeping results
    // thread-local until the join avoids any shared results buffer (no locking),
    // and because `catch_unwind` turns a panicking item into an `Err` value the
    // worker loop itself never unwinds — so no claimed result is ever lost.
    let per_worker: Vec<Vec<(usize, std::result::Result<R, E>)>> = thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                s.spawn(|| {
                    let mut claimed: Vec<(usize, std::result::Result<R, E>)> = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= n {
                            break;
                        }
                        let r = match catch_unwind(AssertUnwindSafe(|| f(&items[i]))) {
                            Ok(v) => v,
                            Err(_) => Err(E::from_worker_panic(panic_msg)),
                        };
                        claimed.push((i, r));
                    }
                    claimed
                })
            })
            .collect();
        handles
            .into_iter()
            // A worker only unwinds if the pool's own bookkeeping panics (never
            // `f`, which is caught above); treat that as a fatal bug.
            .map(|h| {
                h.join()
                    .expect("parallel worker panicked outside catch_unwind")
            })
            .collect()
    });

    let mut slots: Vec<Option<std::result::Result<R, E>>> = (0..n).map(|_| None).collect();
    for chunk in per_worker {
        for (i, r) in chunk {
            slots[i] = Some(r);
        }
    }
    slots
        .into_iter()
        .map(|o| o.expect("every index is produced exactly once"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn empty_input_returns_empty() {
        let out = try_map::<u32, u32, anyhow::Error, _>(&[], 4, "x", |_| Ok(0));
        assert!(out.is_empty());
    }

    #[test]
    fn results_are_positionally_aligned() {
        let items: Vec<usize> = (0..100).collect();
        let out = try_map(&items, 8, "panic", |&v| Ok::<_, anyhow::Error>(v * 2));
        assert_eq!(out.len(), 100);
        for (i, r) in out.iter().enumerate() {
            assert_eq!(*r.as_ref().unwrap(), i * 2, "slot {i} misaligned");
        }
    }

    #[test]
    fn never_exceeds_the_concurrency_cap() {
        let cap = 3;
        let items: Vec<usize> = (0..64).collect();
        let live = AtomicUsize::new(0);
        let max_seen = AtomicUsize::new(0);
        let out = try_map(&items, cap, "panic", |_| {
            let now = live.fetch_add(1, Ordering::SeqCst) + 1;
            max_seen.fetch_max(now, Ordering::SeqCst);
            // Hold the slot briefly so overlap is observable.
            std::thread::sleep(Duration::from_millis(2));
            live.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        });
        assert_eq!(out.len(), 64);
        assert!(
            max_seen.load(Ordering::SeqCst) <= cap,
            "observed {} concurrent workers, cap was {cap}",
            max_seen.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn workers_are_clamped_to_item_count() {
        // Fewer items than the cap must not spawn idle workers past the count,
        // and the results must still be complete and aligned.
        let items = [10usize, 20, 30];
        let out = try_map(&items, 64, "panic", |&v| Ok::<_, anyhow::Error>(v + 1));
        assert_eq!(
            out.iter().map(|r| *r.as_ref().unwrap()).collect::<Vec<_>>(),
            vec![11, 21, 31]
        );
    }

    #[test]
    fn a_panicking_item_becomes_err_others_survive() {
        let items: Vec<usize> = (0..10).collect();
        let out = try_map(&items, 4, "boom", |&v| {
            if v == 5 {
                panic!("intentional test panic");
            }
            Ok::<_, anyhow::Error>(v)
        });
        for (i, r) in out.iter().enumerate() {
            if i == 5 {
                let e = r.as_ref().unwrap_err();
                assert!(e.to_string().contains("boom"), "got: {e}");
            } else {
                assert_eq!(*r.as_ref().unwrap(), i);
            }
        }
    }

    #[test]
    fn propagates_ordinary_errors_per_item() {
        let items: Vec<usize> = (0..6).collect();
        let out = try_map(&items, 2, "panic", |&v| {
            if v % 2 == 0 {
                Ok(v)
            } else {
                Err(anyhow::anyhow!("odd {v}"))
            }
        });
        assert!(out[0].is_ok());
        assert!(out[1].is_err());
        assert_eq!(*out[2].as_ref().unwrap(), 2);
        assert!(out[3].is_err());
    }
}
