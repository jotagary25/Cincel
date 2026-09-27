//! Property tests over random texts and random operation sequences.

mod common;

use cincel_review::TurnId;
use common::{Harness, Rng, mutate, p, random_range, random_text};

/// Seeds per property; `REVIEW_PROP_SEEDS` raises it for a longer soak.
fn seeds() -> u64 {
    std::env::var("REVIEW_PROP_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

fn agent_writes(h: &mut Harness, rng: &mut Rng, path: &str) {
    for _ in 0..1 + rng.below(3) {
        let next = mutate(rng, &h.text(path));
        if rng.chance(50) {
            h.agent_write(path, &next);
        } else {
            h.agent_write_batched(path, &next);
        }
        h.check(path);
        if rng.chance(50) {
            h.settle(path);
            h.check(path);
        }
    }
}

fn user_edits(h: &mut Harness, rng: &mut Rng, path: &str) {
    for _ in 0..rng.below(4) {
        let text = h.text(path);
        let range = random_range(rng, &text);
        let insert = if rng.chance(30) {
            "\n".to_owned()
        } else if rng.chance(50) {
            "typed".to_owned()
        } else {
            String::new()
        };
        if range.is_empty() && insert.is_empty() {
            continue;
        }
        h.user_edit(path, range, &insert);
        h.check(path);
    }
}

#[test]
fn accept_all_makes_base_equal_buffer() {
    for seed in 0..seeds() {
        let mut rng = Rng::new(seed);
        let mut h = Harness::new();
        let base = random_text(&mut rng);
        h.start("f.txt", &base);
        agent_writes(&mut h, &mut rng, "f.txt");
        user_edits(&mut h, &mut rng, "f.txt");
        h.store.end_turn(TurnId(1));
        let buffer = h.text("f.txt");
        h.store.accept_all();
        match h.store.file(&p("f.txt")) {
            None => {}
            Some(file) => {
                assert_eq!(file.base.to_string(), buffer, "seed {seed}");
                assert!(file.hunks.is_empty(), "seed {seed}");
            }
        }
        assert_eq!(h.store.pending_count(), 0, "seed {seed}");
        assert_eq!(h.text("f.txt"), buffer, "accept never touches the buffer");
    }
}

#[test]
fn reject_all_restores_the_original_base() {
    for seed in 0..seeds() {
        let mut rng = Rng::new(seed + 10_000);
        let mut h = Harness::new();
        let base = random_text(&mut rng);
        h.start("f.txt", &base);
        agent_writes(&mut h, &mut rng, "f.txt");
        h.store.end_turn(TurnId(1));
        let reverts = h.store.reject_all();
        h.apply_all(reverts);
        assert_eq!(h.text("f.txt"), base, "seed {seed}");
        h.settle_all();
        assert_eq!(h.store.pending_count(), 0, "seed {seed}");
    }
}

#[test]
fn reject_all_after_user_edits_restores_the_rebased_base() {
    for seed in 0..seeds() {
        let mut rng = Rng::new(seed + 20_000);
        let mut h = Harness::new();
        let base = random_text(&mut rng);
        h.start("f.txt", &base);
        agent_writes(&mut h, &mut rng, "f.txt");
        user_edits(&mut h, &mut rng, "f.txt");
        h.store.end_turn(TurnId(1));
        let expected = h
            .store
            .file(&p("f.txt"))
            .map(|f| f.base.to_string())
            .unwrap_or_else(|| h.text("f.txt"));
        let reverts = h.store.reject_all();
        h.apply_all(reverts);
        assert_eq!(h.text("f.txt"), expected, "seed {seed}");
    }
}

#[test]
fn random_operations_keep_the_invariants() {
    for seed in 0..seeds() {
        let mut rng = Rng::new(seed + 30_000);
        let mut h = Harness::new();
        let base = random_text(&mut rng);
        h.start("f.txt", &base);
        agent_writes(&mut h, &mut rng, "f.txt");
        for _ in 0..12 {
            let hunks = h.hunks("f.txt");
            let op = rng.below(9);
            match op {
                0 => agent_writes(&mut h, &mut rng, "f.txt"),
                1 => user_edits(&mut h, &mut rng, "f.txt"),
                2 if !hunks.is_empty() => {
                    let id = hunks[rng.below(hunks.len())].id;
                    h.store.accept_hunk(id).unwrap();
                }
                3 if !hunks.is_empty() => {
                    let id = hunks[rng.below(hunks.len())].id;
                    let revert = h.store.reject_hunk(id).unwrap();
                    h.apply(revert);
                }
                4 | 5 if !hunks.is_empty() => {
                    let hunk = &hunks[rng.below(hunks.len())];
                    if hunk.lines.is_empty() {
                        continue;
                    }
                    let pair = hunk.lines[rng.below(hunk.lines.len())];
                    if rng.chance(50) {
                        h.store.accept_line(&p("f.txt"), &pair).unwrap();
                    } else {
                        let revert = h.store.reject_line(&p("f.txt"), &pair).unwrap();
                        h.apply(revert);
                    }
                }
                6 => {
                    let reverts = h.store.undo_last_reject();
                    h.apply_all(reverts);
                }
                7 => h.settle("f.txt"),
                _ => h.undo("f.txt"),
            }
            h.check("f.txt");
            // A recompute from scratch agrees with the incremental state on
            // what is pending.
            if let Some(file) = h.store.file(&p("f.txt")) {
                let base_text = file.base.to_string();
                let pending = !h.hunks("f.txt").is_empty();
                let in_flight = file.hunks.iter().any(|x| !x.is_pending());
                if !in_flight {
                    assert_eq!(
                        pending,
                        base_text != h.text("f.txt"),
                        "seed {seed}: pending hunks must mean base != buffer"
                    );
                }
            }
        }
        h.store.end_turn(TurnId(1));
        let expected = h
            .store
            .file(&p("f.txt"))
            .map(|f| f.base.to_string())
            .unwrap_or_else(|| h.text("f.txt"));
        let reverts = h.store.reject_all();
        h.apply_all(reverts);
        assert_eq!(h.text("f.txt"), expected, "seed {seed}");
    }
}

#[test]
fn fast_path_and_batched_events_agree() {
    for seed in 0..seeds() / 2 {
        let mut rng = Rng::new(seed + 40_000);
        let base = random_text(&mut rng);
        let first = mutate(&mut rng, &base);
        let second = mutate(&mut rng, &first);
        let mut results = Vec::new();
        for batched in [false, true] {
            let mut h = Harness::new();
            h.start("f.txt", &base);
            for text in [&first, &second] {
                if batched {
                    h.agent_write_batched("f.txt", text);
                } else {
                    h.agent_write("f.txt", text);
                }
            }
            h.settle("f.txt");
            h.check("f.txt");
            let hunks: Vec<_> = h
                .hunks("f.txt")
                .iter()
                .map(|x| (x.base_rows.clone(), x.buffer_byte_range()))
                .collect();
            results.push(hunks);
        }
        assert_eq!(results[0], results[1], "seed {seed}");
    }
}

#[test]
fn per_line_decisions_add_up_to_the_hunk_decision() {
    for seed in 0..seeds() {
        let mut rng = Rng::new(seed + 50_000);
        let base = random_text(&mut rng);
        let mut h = Harness::new();
        h.start("f.txt", &base);
        let written = mutate(&mut rng, &base);
        h.agent_write("f.txt", &written);
        h.settle("f.txt");
        let accept = rng.chance(50);
        // Decide every line, one at a time, always the first pending one.
        for _ in 0..200 {
            let hunks = h.hunks("f.txt");
            let Some(hunk) = hunks.first() else { break };
            let pair = hunk.lines[0];
            if accept {
                h.store.accept_line(&p("f.txt"), &pair).unwrap();
            } else {
                let revert = h.store.reject_line(&p("f.txt"), &pair).unwrap();
                h.apply(revert);
            }
            h.check("f.txt");
        }
        assert!(h.hunks("f.txt").is_empty(), "seed {seed}");
        if accept {
            assert_eq!(h.text("f.txt"), written, "seed {seed}");
            if let Some(file) = h.store.file(&p("f.txt")) {
                assert_eq!(file.base.to_string(), written, "seed {seed}");
            }
        } else {
            assert_eq!(h.text("f.txt"), base, "seed {seed}");
        }
    }
}
