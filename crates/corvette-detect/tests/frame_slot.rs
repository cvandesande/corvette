use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use corvette_detect::{Frame, SlotEvent, frame_slot};

const fn frame(index: u64) -> Frame {
    Frame {
        index,
        time: Duration::ZERO,
        data: Vec::new(),
    }
}

#[test]
fn the_newest_frame_wins_and_each_replaced_frame_counts_as_dropped() {
    let (sender, receiver) = frame_slot();
    sender.put(frame(1));
    sender.put(frame(2));
    sender.put(frame(3));
    let event = receiver.take(Duration::from_millis(50));
    assert!(
        matches!(&event, SlotEvent::Frame(f) if f.index == 3),
        "{event:?}"
    );
    assert_eq!(receiver.dropped(), 2);
    assert_eq!(receiver.arrived(), 3);
}

#[test]
fn a_take_on_an_empty_slot_times_out_within_a_small_bound() {
    let (_sender, receiver) = frame_slot();
    let start = Instant::now();
    assert!(matches!(
        receiver.take(Duration::from_millis(50)),
        SlotEvent::Timeout
    ));
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(40),
        "returned after {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(2), "overran to {elapsed:?}");
}

#[test]
fn a_blocked_take_wakes_when_a_frame_arrives() {
    let (sender, receiver) = frame_slot();
    let start = Instant::now();
    let taker = thread::spawn(move || receiver.take(Duration::from_secs(10)));
    thread::sleep(Duration::from_millis(50));
    sender.put(frame(7));
    let event = taker.join().unwrap();
    assert!(
        matches!(&event, SlotEvent::Frame(f) if f.index == 7),
        "{event:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "woke only at {:?}",
        start.elapsed()
    );
}

#[test]
fn a_blocked_take_wakes_when_the_producer_reports_a_gap() {
    let (sender, receiver) = frame_slot();
    let (reply, replies) = mpsc::channel();
    let taker = thread::spawn(move || {
        let start = Instant::now();
        let event = receiver.take(Duration::from_secs(10));
        reply.send((event, start.elapsed())).unwrap();
    });
    thread::sleep(Duration::from_millis(50));
    sender.gap();
    let (event, elapsed) = replies.recv_timeout(Duration::from_secs(5)).unwrap();
    taker.join().unwrap();
    assert!(matches!(event, SlotEvent::Gap), "{event:?}");
    assert!(elapsed < Duration::from_secs(5), "woke only at {elapsed:?}");
}

#[test]
fn a_blocked_take_wakes_closed_when_a_closer_closes_the_slot() {
    let (_sender, receiver) = frame_slot();
    let closer = receiver.closer();
    let start = Instant::now();
    let stopper = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        closer.close();
    });
    let event = receiver.take(Duration::from_secs(10));
    stopper.join().unwrap();
    assert!(matches!(event, SlotEvent::Closed), "{event:?}");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "woke only at {:?}",
        start.elapsed()
    );
}

#[test]
fn a_frame_precedes_queued_gaps_and_each_gap_is_returned_once() {
    let (sender, receiver) = frame_slot();
    sender.gap();
    sender.put(frame(1));
    sender.gap();
    assert!(
        matches!(
            receiver.take(Duration::from_millis(50)),
            SlotEvent::Frame(f) if f.index == 1
        ),
        "frame should come before the queued gaps"
    );
    assert!(matches!(
        receiver.take(Duration::from_millis(50)),
        SlotEvent::Gap
    ));
    assert!(matches!(
        receiver.take(Duration::from_millis(50)),
        SlotEvent::Gap
    ));
    assert!(
        matches!(receiver.take(Duration::from_millis(50)), SlotEvent::Timeout),
        "a delivered gap must not be delivered again"
    );
}

#[test]
fn closure_wins_over_a_queued_frame() {
    let (sender, receiver) = frame_slot();
    sender.put(frame(1));
    sender.close();
    assert!(matches!(
        receiver.take(Duration::from_millis(50)),
        SlotEvent::Closed
    ));
}

#[test]
fn dropping_the_sender_closes_the_slot() {
    let (sender, receiver) = frame_slot();
    drop(sender);
    assert!(matches!(
        receiver.take(Duration::from_millis(50)),
        SlotEvent::Closed
    ));
}

#[test]
fn a_receiver_clone_takes_and_closes_the_same_slot() {
    let (sender, receiver) = frame_slot();
    let clone = receiver.clone();
    sender.put(frame(5));
    let taken = receiver.take(Duration::from_millis(50));
    assert!(
        matches!(&taken, SlotEvent::Frame(f) if f.index == 5),
        "{taken:?}"
    );
    assert!(
        matches!(clone.take(Duration::from_millis(50)), SlotEvent::Timeout),
        "the clone found a frame that had already been taken"
    );
    receiver.close();
    assert!(
        matches!(clone.take(Duration::from_millis(50)), SlotEvent::Closed),
        "a close on one handle must be seen by the clone"
    );
}
