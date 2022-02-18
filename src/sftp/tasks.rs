use super::{flush, Error, ReadEnd, SharedData};

use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::task::{spawn, JoinHandle};
use tokio::time;

pub(super) fn create_flush_task(
    shared_data: SharedData,
    flush_interval: Duration,
) -> JoinHandle<Result<(), Error>> {
    spawn(async move {
        let mut interval = time::interval(flush_interval);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

        let auxiliary = shared_data.get_auxiliary();
        let flush_end_notify = &auxiliary.flush_end_notify;
        let pending_requests = &auxiliary.pending_requests;
        let shutdown_requested = &auxiliary.shutdown_requested;
        let max_pending_requests = auxiliary.max_pending_requests();

        let cancel_guard = auxiliary.cancel_token.clone().drop_guard();

        // The loop can only return `Err`
        loop {
            flush_end_notify.notified().await;

            tokio::select! {
                _ = interval.tick() => (),
                // tokio::sync::Notify is cancel safe, however
                // cancelling it would lose the place in the queue.
                //
                // However, since flush_task is the only one who
                // calls `flush_immediately.notified()`, it
                // is totally fine to cancel here.
                _ = auxiliary.flush_immediately.notified() => (),
            };

            let mut prev_pending_requests = pending_requests.load(Ordering::Relaxed);

            loop {
                // Wait until another thread is done or cancelled flushing
                // and try flush it again just in case the flushing is cancelled
                flush(&shared_data).await?;

                prev_pending_requests =
                    pending_requests.fetch_sub(prev_pending_requests, Ordering::Relaxed);

                if prev_pending_requests < max_pending_requests {
                    break;
                }
            }

            if shutdown_requested.load(Ordering::Relaxed) {
                // Once shutdown_requested is sent, there will be no
                // new requests.
                //
                // Flushing here will ensure all pending requests is sent.
                flush(&shared_data).await?;

                cancel_guard.disarm();

                break Ok(());
            }
        }
    })
}

pub(super) fn create_read_task(mut read_end: ReadEnd) -> JoinHandle<Result<(), Error>> {
    spawn(async move {
        let cancel_guard = read_end
            .get_shared_data()
            .get_auxiliary()
            .cancel_token
            .clone()
            .drop_guard();

        loop {
            let new_requests_submit = read_end.wait_for_new_request().await;
            if new_requests_submit == 0 {
                // All responses is read in and there is no
                // write_end/shared_data left.
                cancel_guard.disarm();
                break Ok::<_, Error>(());
            }

            // If attempt to read in more than new_requests_submit, then
            // `read_in_one_packet` might block forever.
            for _ in 0..new_requests_submit {
                read_end.read_in_one_packet().await?;
            }
        }
    })
}
