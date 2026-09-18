//! Tests for every acceptance criterion in `docs/specs/modulos/text.md`.

use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

// ------------------------------------------------------------------ E0 subset

#[test]
fn new_normalizes_crlf() {
    let buffer = Buffer::new("a\r\nb\rc");
    assert_eq!(buffer.text(), "a\nb\nc");
    assert_eq!(buffer.line_count(), 3);
    assert_eq!(buffer.line_ending(), LineEnding::Crlf);
}

#[test]
fn lines_and_offsets() {
    let buffer = Buffer::new("uno\ndos\ntres\n");
    assert_eq!(buffer.line_count(), 4);
    assert_eq!(buffer.line_text(0), "uno");
    assert_eq!(buffer.line_text(3), "");
    assert_eq!(buffer.line_len(1), 3);
    assert_eq!(buffer.line_start_offset(2), 8);
    assert_eq!(buffer.point_to_offset(Point::new(1, 2)), 6);
    assert_eq!(buffer.offset_to_point(6), Point::new(1, 2));
    assert_eq!(buffer.text_in(4..7), "dos");
}

#[test]
fn clip_point_snaps_to_char_boundary() {
    let buffer = Buffer::new("aá\nb");
    // "á" occupies bytes 1..3.
    assert_eq!(buffer.clip_point(Point::new(0, 2)), Point::new(0, 1));
    assert_eq!(buffer.clip_point(Point::new(0, 9)), Point::new(0, 3));
    assert_eq!(buffer.clip_point(Point::new(99, 0)), Point::new(1, 0));
}

#[test]
fn edits_bump_version() {
    let mut buffer = Buffer::new("hola");
    assert_eq!(buffer.version(), 0);
    buffer.insert(4, " mundo");
    assert_eq!(buffer.text(), "hola mundo");
    assert_eq!(buffer.version(), 1);
    buffer.delete(0..5);
    assert_eq!(buffer.text(), "mundo");
    assert_eq!(buffer.version(), 2);
    buffer.replace(0..5, "adiós");
    assert_eq!(buffer.text(), "adiós");
    assert_eq!(buffer.version(), 3);
}

#[test]
fn edit_across_lines() {
    let mut buffer = Buffer::new("uno\ndos\ntres\n");
    let start = buffer.point_to_offset(Point::new(0, 3));
    let end = buffer.point_to_offset(Point::new(1, 0));
    buffer.delete(start..end);
    assert_eq!(buffer.text(), "unodos\ntres\n");
    assert_eq!(buffer.line_count(), 3);
}

#[test]
fn replace_rows_swaps_whole_lines() {
    let mut buffer = Buffer::new("a\nb\nc\nd\n");
    buffer.replace_rows(1..3, &["X".to_string(), "Y".to_string()]);
    assert_eq!(buffer.text(), "a\nX\nY\nd\n");

    let mut buffer = Buffer::new("a\nb\nc\n");
    buffer.replace_rows(1..2, &[]);
    assert_eq!(buffer.text(), "a\nc\n");

    let mut buffer = Buffer::new("a\nb");
    buffer.replace_rows(1..2, &["z".to_string()]);
    assert_eq!(buffer.text(), "a\nz");
}

#[test]
fn utf16_conversions() {
    let buffer = Buffer::new("aá€\n😀");
    // a=1 byte, á=2, €=3, \n=1, 😀=4
    assert_eq!(buffer.offset_to_utf16(0), 0);
    assert_eq!(buffer.offset_to_utf16(3), 2);
    assert_eq!(buffer.offset_to_utf16(6), 3);
    assert_eq!(buffer.offset_from_utf16(3), 6);
    // The emoji is a surrogate pair in UTF-16.
    assert_eq!(buffer.offset_to_utf16(11), 6);
    assert_eq!(buffer.offset_from_utf16(6), 11);
    assert_eq!(buffer.range_to_utf16(&(0..3)), 0..2);
    assert_eq!(buffer.range_from_utf16(&(0..2)), 0..3);
}

#[test]
fn char_boundaries() {
    let buffer = Buffer::new("aáb");
    assert_eq!(buffer.next_char_boundary(0), 1);
    assert_eq!(buffer.next_char_boundary(1), 3);
    assert_eq!(buffer.previous_char_boundary(3), 1);
    assert_eq!(buffer.previous_char_boundary(1), 0);
    assert_eq!(buffer.previous_char_boundary(0), 0);
    assert_eq!(buffer.next_char_boundary(4), 4);
}

// -------------------------------------------------------------------- anchors

#[test]
fn anchor_bias_at_the_edit_point() {
    let mut buffer = Buffer::new("abcdef");
    let before = buffer.track_anchor(3, Bias::Before);
    let after = buffer.track_anchor(3, Bias::After);

    buffer.insert(3, "XY");
    assert_eq!(buffer.text(), "abcXYdef");
    assert_eq!(buffer.resolve_tracked(before), Some(3));
    assert_eq!(buffer.resolve_tracked(after), Some(5));
}

#[test]
fn anchors_survive_edits_before_inside_and_after() {
    let mut buffer = Buffer::new("0123456789");
    let a = buffer.track_anchor(2, Bias::Before);
    let inside = buffer.track_anchor(5, Bias::Before);
    let inside_after = buffer.track_anchor(5, Bias::After);
    let b = buffer.track_anchor(8, Bias::Before);

    // An insertion strictly before every anchor shifts them all.
    buffer.insert(0, "..");
    assert_eq!(buffer.resolve_tracked(a), Some(4));
    assert_eq!(buffer.resolve_tracked(b), Some(10));

    // A deletion containing `inside` collapses it to the edit edges.
    buffer.delete(6..9); // covers offsets 6..9, i.e. the anchors at 7
    assert_eq!(buffer.resolve_tracked(a), Some(4));
    assert_eq!(buffer.resolve_tracked(inside), Some(6));
    assert_eq!(buffer.resolve_tracked(inside_after), Some(6));
    assert_eq!(buffer.resolve_tracked(b), Some(7));

    // An edit fully after the anchors leaves them alone.
    let before_last = buffer.resolve_tracked(b);
    buffer.insert(buffer.len_bytes(), "tail");
    assert_eq!(buffer.resolve_tracked(b), before_last);
}

#[test]
fn plain_anchors_are_clipped_positions() {
    let buffer = Buffer::new("aáb");
    let anchor = buffer.anchor_before(2); // inside "á"
    assert_eq!(anchor.offset, 1);
    assert_eq!(anchor.bias, Bias::Before);
    assert_eq!(buffer.resolve(&anchor), 1);
    assert_eq!(buffer.anchor_after(999).offset, buffer.len_bytes());
}

/// The acceptance criterion: 1 000 random edits, the [`AnchorSet`] against a
/// naive model that transforms every anchor independently with the reference
/// rule written out here.
#[test]
fn anchor_set_matches_naive_model_over_1000_random_edits() {
    /// Reference transform, written independently of the implementation.
    fn naive(offset: usize, bias: Bias, range: &Range<usize>, new_len: usize) -> usize {
        if offset < range.start {
            offset
        } else if offset > range.end {
            offset - (range.end - range.start) + new_len
        } else if bias == Bias::Before {
            range.start
        } else {
            range.start + new_len
        }
    }

    let mut rng = Xorshift::new(0x5EED_1234_ABCD_0001);
    let mut buffer = Buffer::new(&"abcdefghij\n".repeat(20));

    // 64 anchors spread over the buffer, half of each bias.
    let mut ids = Vec::new();
    let mut naive_anchors: Vec<(usize, Bias)> = Vec::new();
    for i in 0..64 {
        let bias = if i % 2 == 0 {
            Bias::Before
        } else {
            Bias::After
        };
        let offset = (i * buffer.len_bytes() / 64).min(buffer.len_bytes());
        ids.push(buffer.track_anchor(offset, bias));
        naive_anchors.push((offset, bias));
    }

    let inserts = ["", "x", "hola\n", "ñ", "\n\n", "0123456789"];
    for step in 0..1000 {
        let len = buffer.len_bytes();
        let mut start = if len == 0 { 0 } else { rng.next_usize(len + 1) };
        let mut end = if len == 0 {
            0
        } else {
            (start + rng.next_usize(12)).min(len)
        };
        start = buffer.clip_offset(start);
        end = buffer.clip_offset(end.max(start));
        let text = inserts[rng.next_usize(inserts.len())];
        if start == end && text.is_empty() {
            continue;
        }

        buffer.replace(start..end, text);

        for anchor in &mut naive_anchors {
            anchor.0 = naive(anchor.0, anchor.1, &(start..end), text.len());
        }

        for (id, expected) in ids.iter().zip(naive_anchors.iter()) {
            assert_eq!(
                buffer.resolve_tracked(*id),
                Some(expected.0),
                "step {step}: anchor {id:?} with bias {:?} after replacing {start}..{end} with {text:?}",
                expected.1
            );
            assert!(
                expected.0 <= buffer.len_bytes(),
                "step {step}: anchor escaped the buffer"
            );
        }
    }
}

#[test]
fn external_anchor_map_follows_events() {
    let mut buffer = Buffer::new("0123456789");
    let events: Arc<Mutex<Vec<BufferEvent>>> = Arc::default();
    let sink = events.clone();
    buffer.subscribe(Box::new(move |event| {
        sink.lock().unwrap().push(event.clone())
    }));

    // A container owned outside the buffer, as the review crate will do.
    let mut map: AnchorMap<&'static str> = AnchorMap::new();
    let hunk = map.insert(Anchor::before(4), "hunk");
    let tail = map.insert(Anchor::after(9), "tail");

    buffer.insert(0, "ab");
    buffer.delete(6..8);

    for event in events.lock().unwrap().iter() {
        map.apply_event(event);
    }
    assert_eq!(map.resolve(hunk), Some(6));
    assert_eq!(map.resolve(tail), Some(9));
    assert_eq!(map.value(hunk), Some(&"hunk"));
    assert_eq!(map.len(), 2);
    assert_eq!(
        map.remove(tail).map(|(a, v)| (a.offset, v)),
        Some((9, "tail"))
    );
    assert_eq!(map.len(), 1);
}

#[test]
fn anchor_map_iterates_in_offset_order() {
    let mut map = AnchorSet::new();
    map.add(Anchor::before(30));
    map.add(Anchor::before(10));
    map.add(Anchor::after(20));
    let offsets: Vec<usize> = map.iter().map(|(_, anchor, ())| anchor.offset).collect();
    assert_eq!(offsets, vec![10, 20, 30]);
}

// --------------------------------------------------- transactions, undo, redo

#[test]
fn agent_transaction_undo_restores_text_and_source() {
    let mut buffer = Buffer::new("fn main() {}\n");
    let original = buffer.text();

    buffer.start_transaction(EditSource::Agent { turn_id: 7 });
    buffer.replace(3..7, "suma");
    buffer.insert(buffer.len_bytes(), "// nuevo\n");
    assert!(buffer.end_transaction());

    assert_eq!(buffer.text(), "fn suma() {}\n// nuevo\n");
    assert_eq!(buffer.undo(), Some(EditSource::Agent { turn_id: 7 }));
    assert_eq!(buffer.text(), original);
    assert_eq!(buffer.redo(), Some(EditSource::Agent { turn_id: 7 }));
    assert_eq!(buffer.text(), "fn suma() {}\n// nuevo\n");
}

#[test]
fn undo_redo_walks_the_whole_stack() {
    let mut buffer = Buffer::new("");
    buffer.set_group_interval(Duration::ZERO);
    for word in ["a", "b", "c"] {
        buffer.transact(EditSource::User, |b| b.insert(b.len_bytes(), word));
    }
    assert_eq!(buffer.text(), "abc");
    assert_eq!(buffer.undo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "ab");
    assert_eq!(buffer.undo(), Some(EditSource::User));
    assert_eq!(buffer.undo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "");
    assert_eq!(buffer.undo(), None);
    assert!(!buffer.can_undo());
    assert_eq!(buffer.redo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "a");
}

#[test]
fn consecutive_user_transactions_group_within_the_interval() {
    let mut buffer = Buffer::new("");
    // Default 300 ms: three fast keystrokes are one undo unit.
    for ch in ["h", "o", "y"] {
        buffer.transact(EditSource::User, |b| b.insert(b.len_bytes(), ch));
    }
    assert_eq!(buffer.text(), "hoy");
    assert_eq!(buffer.undo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "");

    // With no grouping window each transaction is its own unit.
    let mut buffer = Buffer::new("");
    buffer.set_group_interval(Duration::ZERO);
    for ch in ["h", "o", "y"] {
        buffer.transact(EditSource::User, |b| b.insert(b.len_bytes(), ch));
    }
    buffer.undo();
    assert_eq!(buffer.text(), "ho");
}

#[test]
fn only_user_transactions_group() {
    let mut buffer = Buffer::new("");
    buffer.transact(EditSource::User, |b| b.insert(0, "u"));
    buffer.transact(EditSource::Agent { turn_id: 1 }, |b| b.insert(1, "a"));
    buffer.transact(EditSource::User, |b| b.insert(2, "u2"));
    assert_eq!(buffer.text(), "uau2");
    assert_eq!(buffer.undo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "ua");
    assert_eq!(buffer.undo(), Some(EditSource::Agent { turn_id: 1 }));
    assert_eq!(buffer.text(), "u");
}

#[test]
fn nested_transactions_form_one_undo_unit() {
    let mut buffer = Buffer::new("x");
    buffer.start_transaction(EditSource::Review);
    buffer.start_transaction(EditSource::User);
    buffer.insert(1, "y");
    assert!(!buffer.end_transaction());
    buffer.insert(2, "z");
    assert!(buffer.end_transaction());
    assert_eq!(buffer.text(), "xyz");
    assert_eq!(buffer.undo(), Some(EditSource::Review));
    assert_eq!(buffer.text(), "x");
}

#[test]
fn a_new_edit_clears_the_redo_stack() {
    let mut buffer = Buffer::new("");
    buffer.set_group_interval(Duration::ZERO);
    buffer.transact(EditSource::User, |b| b.insert(0, "a"));
    buffer.undo();
    assert!(buffer.can_redo());
    buffer.transact(EditSource::User, |b| b.insert(0, "z"));
    assert!(!buffer.can_redo());
    assert_eq!(buffer.text(), "z");
}

// ---------------------------------------------------------- multi-range edits

#[test]
fn multi_range_edit_applies_every_range() {
    let mut buffer = Buffer::new("aaa bbb ccc");
    buffer.edit(&[0..3, 8..11, 4..7], "X");
    assert_eq!(buffer.text(), "X X X");
    assert_eq!(buffer.undo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "aaa bbb ccc");
    assert_eq!(buffer.redo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "X X X");
}

#[test]
fn overlapping_ranges_are_merged() {
    let mut buffer = Buffer::new("abcdef");
    buffer.edit(&[1..3, 2..5], "-");
    assert_eq!(buffer.text(), "a-f");
}

// --------------------------------------------------------------------- events

#[test]
fn events_report_source_ranges_and_version() {
    let mut buffer = Buffer::new("hola mundo");
    let events: Arc<Mutex<Vec<BufferEvent>>> = Arc::default();
    let sink = events.clone();
    let id = buffer.subscribe(Box::new(move |event| {
        sink.lock().unwrap().push(event.clone())
    }));

    buffer.transact(EditSource::Agent { turn_id: 3 }, |b| {
        b.replace(0..4, "chau");
    });

    assert_eq!(events.lock().unwrap().len(), 1);
    let BufferEvent::Edited {
        source,
        old_ranges,
        new_ranges,
        version,
    } = events.lock().unwrap()[0].clone();
    assert_eq!(source, EditSource::Agent { turn_id: 3 });
    assert_eq!(old_ranges, vec![0..4]);
    assert_eq!(new_ranges, vec![0..4]);
    assert_eq!(version, buffer.version());

    buffer.unsubscribe(id);
    buffer.insert(0, "!");
    assert_eq!(events.lock().unwrap().len(), 1);
}

#[test]
fn multi_range_event_ranges_are_in_both_coordinate_spaces() {
    let mut buffer = Buffer::new("aaa bbb");
    let events: Arc<Mutex<Vec<BufferEvent>>> = Arc::default();
    let sink = events.clone();
    buffer.subscribe(Box::new(move |event| {
        sink.lock().unwrap().push(event.clone())
    }));

    buffer.edit(&[0..3, 4..7], "XY");
    assert_eq!(buffer.text(), "XY XY");
    let BufferEvent::Edited {
        old_ranges,
        new_ranges,
        ..
    } = events.lock().unwrap()[0].clone();
    assert_eq!(old_ranges, vec![0..3, 4..7]);
    assert_eq!(new_ranges, vec![0..2, 3..5]);
}

#[test]
fn the_event_queue_is_the_pull_alternative_to_subscribing() {
    let mut buffer = Buffer::new("abc");
    assert!(buffer.drain_events().is_empty(), "off by default");

    buffer.record_events(true);
    assert!(buffer.records_events());
    buffer.insert(3, "d");
    buffer.delete(0..1);

    let events = buffer.drain_events();
    assert_eq!(events.len(), 2);
    assert!(buffer.drain_events().is_empty(), "draining empties it");

    buffer.record_events(false);
    buffer.insert(0, "z");
    assert!(buffer.drain_events().is_empty());
}

#[test]
fn undo_emits_an_event_with_the_undone_source() {
    let mut buffer = Buffer::new("abc");
    let events: Arc<Mutex<Vec<BufferEvent>>> = Arc::default();
    let sink = events.clone();
    buffer.subscribe(Box::new(move |event| {
        sink.lock().unwrap().push(event.clone())
    }));

    buffer.transact(EditSource::Review, |b| b.delete(0..1));
    buffer.undo();

    let sources: Vec<EditSource> = events
        .lock()
        .unwrap()
        .iter()
        .map(|BufferEvent::Edited { source, .. }| *source)
        .collect();
    assert_eq!(sources, vec![EditSource::Review, EditSource::Review]);
}

// ------------------------------------------------------- encoding, BOM, CRLF

#[test]
fn crlf_file_roundtrips_byte_identical() {
    let original: &[u8] = b"uno\r\ndos\r\ntres\r\n";
    let buffer = Buffer::from_bytes(original).expect("valid utf-8");
    assert_eq!(buffer.line_ending(), LineEnding::Crlf);
    assert!(!buffer.has_bom());
    assert_eq!(buffer.text(), "uno\ndos\ntres\n");
    assert_eq!(buffer.to_bytes(), original);
}

#[test]
fn crlf_with_bom_roundtrips_byte_identical() {
    let mut original = vec![0xEF, 0xBB, 0xBF];
    original.extend_from_slice(b"uno\r\ndos\r\n");
    let buffer = Buffer::from_bytes(&original).expect("valid utf-8");
    assert!(buffer.has_bom());
    assert_eq!(buffer.line_ending(), LineEnding::Crlf);
    assert_eq!(buffer.text(), "uno\ndos\n");
    assert_eq!(buffer.to_bytes(), original);
}

#[test]
fn lf_with_bom_roundtrips_byte_identical() {
    let mut original = vec![0xEF, 0xBB, 0xBF];
    original.extend_from_slice("ñandú\nsegunda\n".as_bytes());
    let buffer = Buffer::from_bytes(&original).expect("valid utf-8");
    assert!(buffer.has_bom());
    assert_eq!(buffer.line_ending(), LineEnding::Lf);
    assert_eq!(buffer.to_bytes(), original);
}

#[test]
fn invalid_utf8_is_rejected() {
    assert!(matches!(
        Buffer::from_bytes(&[0x66, 0xFF, 0x6F]),
        Err(LoadError::NotUtf8)
    ));
    assert!(matches!(
        Buffer::from_bytes(&[0xEF, 0xBB, 0xBF, 0xC3]),
        Err(LoadError::NotUtf8)
    ));
    assert_eq!(
        LoadError::NotUtf8.to_string(),
        "the file is not valid UTF-8"
    );
}

#[test]
fn edits_do_not_reintroduce_cr() {
    let mut buffer = Buffer::from_bytes(b"a\r\nb\r\n").expect("valid utf-8");
    buffer.insert(2, "x\r\ny");
    assert_eq!(buffer.text(), "a\nx\nyb\n");
    assert_eq!(buffer.to_bytes(), b"a\r\nx\r\nyb\r\n");
}

// -------------------------------------------------------------------- snapshot

#[test]
fn snapshot_is_send_and_independent() {
    fn assert_send<T: Send>() {}
    assert_send::<BufferSnapshot>();
    // The buffer itself is `Send` too, so a `BufferStore` can keep it behind a
    // mutex; that is why subscriber callbacks carry a `Send` bound.
    assert_send::<Buffer>();

    let mut buffer = Buffer::new("hola");
    let snapshot = buffer.snapshot();
    buffer.insert(4, " mundo");
    assert_eq!(snapshot.text(), "hola");
    assert_eq!(snapshot.version(), 0);
    assert_eq!(snapshot.len(), 4);
    assert_eq!(buffer.snapshot().text(), "hola mundo");
    assert_eq!(buffer.snapshot().version(), 1);
    assert_eq!(snapshot.line(0).to_string(), "hola");
}

/// `snapshot()` of a 10 MB buffer must take < 1 ms (it is a rope clone, so the
/// real cost is a handful of refcount bumps). Timing test: if it ever turns
/// flaky on a loaded CI box, mark it `#[ignore]`.
#[test]
fn snapshot_of_10mb_buffer_is_under_a_millisecond() {
    let line = "esta es una línea de texto de relleno para el buffer grande\n";
    let mut text = String::with_capacity(11 * 1024 * 1024);
    while text.len() < 10 * 1024 * 1024 {
        text.push_str(line);
    }
    let buffer = Buffer::new(&text);
    assert!(buffer.len_bytes() >= 10 * 1024 * 1024);

    // Warm up, then measure the best of 20 clones.
    let _ = buffer.snapshot();
    let mut best = Duration::MAX;
    for _ in 0..20 {
        let start = Instant::now();
        let snapshot = buffer.snapshot();
        let elapsed = start.elapsed();
        std::hint::black_box(&snapshot);
        best = best.min(elapsed);
    }
    println!(
        "snapshot() of {} MiB: {best:?}",
        buffer.len_bytes() / (1024 * 1024)
    );
    assert!(
        best < Duration::from_millis(1),
        "snapshot() of 10 MB took {best:?}, expected < 1 ms"
    );
}

// ------------------------------------------------------------------ utilities

/// Tiny deterministic PRNG, so the randomized test is reproducible and the
/// crate stays dependency-free.
struct Xorshift(u64);

impl Xorshift {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_usize(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }
}

// ------------------------------------------------- edit_many (E1-A follow-up)

#[test]
fn edit_many_applies_distinct_replacements_in_one_undo_step() {
    let mut buffer = Buffer::new("aaa bbb ccc");
    // Deliberately out of order.
    let ranges = buffer.edit_many(&[(8..11, "Z"), (0..3, "XYZ"), (4..7, "")]);
    assert_eq!(buffer.text(), "XYZ  Z");
    assert_eq!(ranges, vec![0..3, 4..4, 5..6]);

    assert_eq!(buffer.undo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "aaa bbb ccc");
    assert_eq!(buffer.redo(), Some(EditSource::User));
    assert_eq!(buffer.text(), "XYZ  Z");
}

#[test]
fn edit_many_emits_one_event_per_replacement() {
    let mut buffer = Buffer::new("aaa bbb");
    buffer.record_events(true);
    let start = buffer.version();
    buffer.edit_many(&[(0..3, "X"), (4..7, "YY")]);

    let events = buffer.drain_events();
    assert_eq!(events.len(), 2, "one event per replacement");
    let BufferEvent::Edited {
        old_ranges,
        new_ranges,
        version,
        ..
    } = events[0].clone();
    assert_eq!(old_ranges, vec![0..3]);
    assert_eq!(new_ranges, vec![0..1]);
    assert_eq!(version, start + 1);

    // The second event is expressed in the text the first one produced, which
    // is exactly what a tree-sitter `InputEdit` needs.
    let BufferEvent::Edited {
        old_ranges,
        new_ranges,
        version,
        ..
    } = events[1].clone();
    assert_eq!(old_ranges, vec![2..5]);
    assert_eq!(new_ranges, vec![2..4]);
    assert_eq!(version, start + 2);
    assert_eq!(buffer.text(), "X YY");
}

#[test]
fn edit_many_joins_an_open_transaction() {
    let mut buffer = Buffer::new("uno dos");
    buffer.start_transaction(EditSource::Agent { turn_id: 4 });
    buffer.edit_many(&[(0..3, "1"), (4..7, "2")]);
    buffer.insert(buffer.len_bytes(), "!");
    assert!(buffer.end_transaction());
    assert_eq!(buffer.text(), "1 2!");
    assert_eq!(buffer.undo(), Some(EditSource::Agent { turn_id: 4 }));
    assert_eq!(buffer.text(), "uno dos");
}

#[test]
fn edit_many_moves_anchors_and_drops_overlaps() {
    let mut buffer = Buffer::new("0123456789");
    let tail = buffer.track_anchor(9, Bias::Before);
    // 2..5 and 3..6 overlap: the second one is dropped.
    buffer.edit_many(&[(2..5, "--"), (3..6, "!!")]);
    assert_eq!(buffer.text(), "01--56789");
    assert_eq!(buffer.resolve_tracked(tail), Some(8));
}

#[test]
fn edit_many_on_empty_input_does_nothing() {
    let mut buffer = Buffer::new("hola");
    assert!(buffer.edit_many(&[]).is_empty());
    assert_eq!(buffer.version(), 0);
    assert!(!buffer.can_undo());
}

// ------------------------------------------------------------- minimal diffs

#[test]
fn minimal_edits_matches_the_project_helper() {
    use crate::diff::{apply, minimal_edits};

    assert!(minimal_edits("a\nb\n", "a\nb\n").is_empty());

    let before = "fn suma(a: i32, b: i32) -> i32 {\n    a - b\n}\n";
    let after = "fn suma(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    let edits = minimal_edits(before, after);
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].1, "+");
    assert_eq!(&before[edits[0].0.clone()], "-");
    assert_eq!(apply(before, &edits), after);

    // Nearby character changes merge; distant ones stay apart.
    assert_eq!(
        minimal_edits("say(\"hola\");\n", "say(\"chau\");\n").len(),
        1
    );
    assert_eq!(
        minimal_edits(
            "let alfa = 1; let beta = 2; let gama = 3;\n",
            "let alfa = 9; let beta = 2; let gama = 7;\n"
        )
        .len(),
        2
    );

    // Ascending and non-overlapping.
    let before = "a\nb\nc\nd\ne\nf\ng\n";
    let after = "a\nB\nc\nd\ne\nF\ng\n";
    let edits = minimal_edits(before, after);
    assert_eq!(edits.len(), 2);
    for pair in edits.windows(2) {
        assert!(pair[0].0.end <= pair[1].0.start);
    }
    assert_eq!(apply(before, &edits), after);
}

#[test]
fn set_text_minimal_only_touches_what_changed() {
    let mut buffer = Buffer::new("uno\ndos\ntres\n");
    // An anchor on the last line must survive a rewrite of the second one.
    let tres = buffer.track_anchor(8, Bias::Before);
    buffer.record_events(true);

    let ranges = buffer.set_text_minimal("uno\nDOS\ntres\n", EditSource::Agent { turn_id: 1 });
    assert_eq!(buffer.text(), "uno\nDOS\ntres\n");
    assert_eq!(ranges.len(), 1);
    assert_eq!(buffer.text_in(ranges[0].clone()), "DOS");
    assert_eq!(
        buffer.resolve_tracked(tres),
        Some(8),
        "the anchor did not move"
    );

    // One transaction, one undo.
    assert_eq!(buffer.undo(), Some(EditSource::Agent { turn_id: 1 }));
    assert_eq!(buffer.text(), "uno\ndos\ntres\n");

    // The events are real edits, not a wholesale replacement.
    assert!(!buffer.drain_events().is_empty());
}

#[test]
fn set_text_minimal_on_equal_text_is_a_no_op() {
    let mut buffer = Buffer::new("igual\n");
    assert!(
        buffer
            .set_text_minimal("igual\n", EditSource::Load)
            .is_empty()
    );
    assert_eq!(buffer.version(), 0);
    assert!(!buffer.can_undo());
    // CRLF input is normalized before comparing, so it is still a no-op.
    assert!(
        buffer
            .set_text_minimal("igual\r\n", EditSource::Load)
            .is_empty()
    );
    assert_eq!(buffer.version(), 0);
}

#[test]
fn set_text_minimal_handles_a_full_rewrite() {
    let mut buffer = Buffer::new("una cosa\notra cosa\n");
    buffer.set_text_minimal("nada que ver\ncon lo anterior\n", EditSource::Review);
    assert_eq!(buffer.text(), "nada que ver\ncon lo anterior\n");
    assert_eq!(buffer.undo(), Some(EditSource::Review));
    assert_eq!(buffer.text(), "una cosa\notra cosa\n");
}

// -------------------------------------------------------------- saved state

#[test]
fn saved_state_tracks_edits() {
    let mut buffer = Buffer::new("hola");
    assert_eq!(buffer.saved_version(), 0);
    assert!(!buffer.is_dirty());

    buffer.insert(4, "!");
    assert!(buffer.is_dirty());
    assert_eq!(buffer.saved_version(), 0);

    buffer.mark_saved();
    assert!(!buffer.is_dirty());
    assert_eq!(buffer.saved_version(), buffer.version());

    buffer.undo();
    assert!(buffer.is_dirty(), "undoing past the save point is a change");

    let buffer = Buffer::from_bytes(b"limpio\n").expect("valid utf-8");
    assert!(!buffer.is_dirty());
    assert_eq!(buffer.saved_version(), 0);
}

#[test]
fn undoing_back_to_the_saved_text_cleans_the_buffer() {
    let mut buffer = Buffer::new("hola\n");
    buffer.transact(EditSource::User, |buffer| buffer.insert(4, " mundo"));
    assert!(buffer.is_dirty());
    assert_ne!(buffer.version(), buffer.saved_version());

    buffer.undo();
    assert_eq!(buffer.text(), "hola\n");
    assert!(
        !buffer.is_dirty(),
        "the text is the one on disk again, even though the version moved"
    );
    assert!(buffer.content_matches_saved());
    assert!(
        buffer.version() != buffer.saved_version(),
        "the version cannot come back, which is exactly why dirtiness is by content"
    );

    buffer.redo();
    assert_eq!(buffer.text(), "hola mundo\n");
    assert!(buffer.is_dirty(), "redoing the edit is a change again");
}

#[test]
fn retyping_the_saved_text_by_another_path_is_clean() {
    let mut buffer = Buffer::new("hola\n");
    // Delete everything and type the very same thing back.
    buffer.transact(EditSource::User, |buffer| buffer.delete(0..5));
    assert!(buffer.is_dirty());
    buffer.transact(EditSource::User, |buffer| buffer.insert(0, "hola\n"));

    assert_eq!(buffer.text(), "hola\n");
    assert!(
        !buffer.is_dirty(),
        "same content through a different edit path is still clean"
    );
}

#[test]
fn marking_saved_moves_the_baseline() {
    let mut buffer = Buffer::new("uno\n");
    buffer.transact(EditSource::User, |buffer| buffer.insert(4, "dos\n"));
    buffer.mark_saved();
    assert!(!buffer.is_dirty());
    assert_eq!(buffer.saved_snapshot().text(), "uno\ndos\n");

    // Undoing now goes *away* from disk, so it is a change.
    buffer.undo();
    assert_eq!(buffer.text(), "uno\n");
    assert!(buffer.is_dirty());
}

// ------------------------------------------------- snapshot comparison

#[test]
fn snapshots_compare_cheaply() {
    let mut buffer = Buffer::new("mismo texto\n");
    let a = buffer.snapshot();
    let b = buffer.snapshot();
    assert!(
        a.ptr_eq(&b),
        "two snapshots with no edit in between share the rope"
    );
    assert!(a.same_text_as(&b));
    assert_eq!(a.content_hash(), b.content_hash());

    buffer.insert(0, "x");
    let c = buffer.snapshot();
    assert!(!a.ptr_eq(&c));
    assert!(!a.same_text_as(&c));

    // Same text from a different buffer: no shared rope, but equal content.
    let other = Buffer::new("mismo texto\n").snapshot();
    assert!(!a.ptr_eq(&other));
    assert!(a.same_text_as(&other));
    assert_eq!(a.content_hash(), other.content_hash());

    // Different lengths take the O(1) path.
    assert!(!a.same_text_as(&Buffer::new("otro").snapshot()));

    // The hash is stable across repeated calls and clones.
    let hash = a.content_hash();
    assert_eq!(a.content_hash(), hash);
    assert_eq!(a.clone().content_hash(), hash);
}

#[test]
fn the_content_hash_follows_the_text() {
    let mut buffer = Buffer::new("uno");
    let first = buffer.snapshot().content_hash();
    buffer.insert(3, " dos");
    let second = buffer.snapshot().content_hash();
    assert_ne!(first, second);
    buffer.delete(3..7);
    assert_eq!(buffer.snapshot().content_hash(), first);
}

// ------------------------------------------------- undo/redo with ranges

#[test]
fn undo_and_redo_report_the_affected_ranges() {
    let mut buffer = Buffer::new("uno dos tres");
    buffer.transact(EditSource::Agent { turn_id: 9 }, |b| {
        b.edit_many(&[(0..3, "1"), (8..12, "3333")]);
    });
    assert_eq!(buffer.text(), "1 dos 3333");

    let (source, ranges) = buffer.undo_with_ranges().expect("one undo unit");
    assert_eq!(source, EditSource::Agent { turn_id: 9 });
    assert_eq!(buffer.text(), "uno dos tres");
    assert_eq!(ranges, vec![0..3, 8..12]);
    for range in &ranges {
        assert!(range.end <= buffer.len_bytes());
    }
    assert_eq!(buffer.text_in(ranges[0].clone()), "uno");
    assert_eq!(buffer.text_in(ranges[1].clone()), "tres");

    let (source, ranges) = buffer.redo_with_ranges().expect("one redo unit");
    assert_eq!(source, EditSource::Agent { turn_id: 9 });
    assert_eq!(buffer.text(), "1 dos 3333");
    assert_eq!(ranges, vec![0..1, 6..10]);
    assert_eq!(buffer.text_in(ranges[1].clone()), "3333");

    assert_eq!(
        buffer.undo_with_ranges().map(|(s, _)| s),
        Some(EditSource::Agent { turn_id: 9 })
    );
    assert!(buffer.undo_with_ranges().is_none());
}

#[test]
fn undo_with_ranges_on_a_single_edit() {
    let mut buffer = Buffer::new("hola mundo");
    buffer.replace(5..10, "gente");
    let (_, ranges) = buffer.undo_with_ranges().expect("one unit");
    assert_eq!(ranges, vec![5..10]);
    assert_eq!(buffer.text_in(ranges[0].clone()), "mundo");
}

// ------------------------------------------------------- read-only buffers

#[test]
fn invalid_utf8_loads_lossy_and_read_only() {
    let (buffer, had_invalid) = Buffer::from_bytes_lossy(b"ho\xFFla\n");
    assert!(had_invalid);
    assert!(buffer.is_read_only());
    assert_eq!(buffer.text(), "ho\u{FFFD}la\n");

    let (buffer, had_invalid) = Buffer::from_bytes_lossy("válido\r\n".as_bytes());
    assert!(!had_invalid);
    assert!(!buffer.is_read_only());
    assert_eq!(buffer.line_ending(), LineEnding::Crlf);
    assert_eq!(buffer.to_bytes(), "válido\r\n".as_bytes());

    let mut original = vec![0xEF, 0xBB, 0xBF];
    original.extend_from_slice(b"con bom\n");
    let (buffer, had_invalid) = Buffer::from_bytes_lossy(&original);
    assert!(!had_invalid);
    assert!(buffer.has_bom());
    assert_eq!(buffer.to_bytes(), original);
}

#[test]
fn a_read_only_buffer_refuses_every_edit() {
    let mut buffer = Buffer::new("intocable");
    buffer.set_read_only(true);
    assert!(buffer.is_read_only());

    // The fallible API reports it.
    let one = 0..1;
    assert_eq!(
        buffer.try_edit(std::slice::from_ref(&one), "X"),
        Err(EditError::ReadOnly)
    );
    assert_eq!(
        buffer.try_edit_many(&[(0..1, "X")]),
        Err(EditError::ReadOnly)
    );

    // The infallible one is a warned no-op.
    buffer.insert(0, "X");
    buffer.delete(0..1);
    buffer.replace(0..2, "yy");
    buffer.edit_many(&[(0..1, "z")]);
    buffer.replace_rows(0..1, &["otra".to_string()]);
    buffer.set_text_minimal("distinto", EditSource::Load);
    assert_eq!(buffer.text(), "intocable");
    assert_eq!(buffer.version(), 0);
    assert!(buffer.undo_with_ranges().is_none());
    assert!(buffer.redo_with_ranges().is_none());

    // Clearing the flag lets edits through again.
    buffer.set_read_only(false);
    let empty = 0..0;
    assert!(buffer.try_edit(std::slice::from_ref(&empty), "¡").is_ok());
    assert_eq!(buffer.text(), "¡intocable");
    assert_eq!(EditError::ReadOnly.to_string(), "the buffer is read-only");
}
