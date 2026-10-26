//! Stand-in for basisu's `job_pool` (`basisu_enc.h`). C++ queues closures and waits for all of
//! them; every use in the encoder writes disjoint outputs, so results never depend on the order
//! jobs run in. Here jobs run on scoped threads pulling indices from a shared counter, and the
//! results come back in job order for the caller to apply serially.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Runs `job(0..num_jobs)` on up to `total_threads` threads (the calling thread included, as
/// `job_pool::get_total_threads`) and returns the results in job order. `None` if a job
/// panicked (never expected; kept so a worker failure is not a process abort).
pub fn map_jobs<R: Send>(
    total_threads: u32,
    num_jobs: usize,
    job: impl Fn(usize) -> R + Sync,
) -> Option<Vec<R>> {
    let threads = (total_threads.max(1) as usize).min(num_jobs);
    if threads <= 1 {
        return Some((0..num_jobs).map(job).collect());
    }
    let next = AtomicUsize::new(0);
    let worker = || {
        let mut done = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= num_jobs {
                break;
            }
            done.push((i, job(i)));
        }
        done
    };
    let parts = std::thread::scope(|s| {
        let handles: Vec<_> = (1..threads).map(|_| s.spawn(worker)).collect();
        let mut parts = vec![Some(worker())];
        for h in handles {
            parts.push(h.join().ok());
        }
        parts
    });
    let mut slots: Vec<Option<R>> = (0..num_jobs).map(|_| None).collect();
    for part in parts {
        for (i, r) in part? {
            if let Some(slot) = slots.get_mut(i) {
                *slot = Some(r);
            }
        }
    }
    slots.into_iter().collect()
}

/// Runs `job(first_index, chunk)` over `data.chunks_mut(chunk_len)` on up to `total_threads`
/// threads; `first_index` is the chunk's offset into `data`. `false` if a job panicked.
pub fn for_each_chunk_mut<T: Send>(
    total_threads: u32,
    data: &mut [T],
    chunk_len: usize,
    job: impl Fn(usize, &mut [T]) + Sync,
) -> bool {
    let chunk_len = chunk_len.max(1);
    let num_jobs = data.len().div_ceil(chunk_len);
    let threads = (total_threads.max(1) as usize).min(num_jobs);
    if threads <= 1 {
        for (j, chunk) in data.chunks_mut(chunk_len).enumerate() {
            job(j * chunk_len, chunk);
        }
        return true;
    }
    let queue = std::sync::Mutex::new(data.chunks_mut(chunk_len).enumerate());
    // The guard is dropped before the job runs, so workers only contend on taking a chunk.
    let take = || queue.lock().ok().and_then(|mut q| q.next());
    let worker = || {
        while let Some((j, chunk)) = take() {
            job(j * chunk_len, chunk);
        }
    };
    std::thread::scope(|s| {
        let handles: Vec<_> = (1..threads).map(|_| s.spawn(worker)).collect();
        worker();
        handles.into_iter().all(|h| h.join().is_ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_jobs_keeps_job_order() {
        for threads in [1, 3, 16] {
            let r = map_jobs(threads, 1000, |i| i * 7);
            assert_eq!(r, Some((0..1000).map(|i| i * 7).collect::<Vec<_>>()));
        }
        assert_eq!(map_jobs(8, 0, |i| i), Some(vec![]));
    }

    #[test]
    fn for_each_chunk_mut_covers_every_item() {
        for threads in [1, 4, 32] {
            let mut v = vec![0usize; 1001];
            assert!(for_each_chunk_mut(threads, &mut v, 64, |first, c| {
                for (k, x) in c.iter_mut().enumerate() {
                    *x = first + k;
                }
            }));
            assert!(v.iter().enumerate().all(|(i, &x)| i == x));
        }
    }
}
