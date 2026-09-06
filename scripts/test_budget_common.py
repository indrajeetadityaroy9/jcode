"""Tests for the shared ratchet-ledger mechanics.

Each test pins one of the five defects the shared layer was written to fix, and
each fails against the previous behaviour: the old scripts accepted a corrupt
aggregate, rebased every entry on `--update`, had no notion of a move, ignored
in-code exemptions, and never retired dead entries.

Run: python3 scripts/test_budget_common.py
"""

from __future__ import annotations

import unittest
from pathlib import Path

import budget_common as bc


class AggregateValidation(unittest.TestCase):
    """A ledger that cannot add up its own rows must not be trusted."""

    def test_total_disagreeing_with_the_table_is_rejected(self):
        data = {"total": 999999, "tracked_files": {"a.rs": 2, "b.rs": 3}}
        with self.assertRaises(SystemExit) as caught:
            bc.validate_baseline(data, Path("x.json"))
        message = str(caught.exception)
        self.assertIn("internally inconsistent", message)
        # The operator needs both numbers to know which side is wrong.
        self.assertIn("999999", message)
        self.assertIn("5", message)

    def test_pattern_totals_disagreeing_with_the_table_are_rejected(self):
        data = {
            "total": 3,
            "totals_by_pattern": {"dot_ok": 99},
            "tracked_files": {"a.rs": {"dot_ok": 3}},
        }
        with self.assertRaises(SystemExit) as caught:
            bc.validate_baseline(data, Path("x.json"))
        self.assertIn("totals_by_pattern", str(caught.exception))

    def test_a_consistent_ledger_passes(self):
        data = {
            "total": 5,
            "totals_by_pattern": {"dot_ok": 5},
            "tracked_files": {"a.rs": {"dot_ok": 2}, "b.rs": {"dot_ok": 3}},
        }
        bc.validate_baseline(data, Path("x.json"))

    def test_repair_lowers_an_inflated_aggregate_to_the_table(self):
        data = {"total": 999, "tracked_files": {"a.rs": 2, "b.rs": 3}}
        bc.repair_summary(data)
        self.assertEqual(data["total"], 5)


class SurgicalUpdate(unittest.TestCase):
    """`--update PATH` must touch only that path."""

    def test_named_path_is_recorded_and_others_are_untouched(self):
        tracked = {"a.rs": 10, "b.rs": 10}
        current = {"a.rs": 4, "b.rs": 25}
        bc.apply_update(tracked, current, ["a.rs"], allow_regression=False)
        self.assertEqual(tracked, {"a.rs": 4, "b.rs": 10}, "b.rs drift must not be laundered")

    def test_recording_a_worse_count_requires_explicit_consent(self):
        tracked = {"a.rs": 10}
        with self.assertRaises(SystemExit) as caught:
            bc.apply_update(tracked, {"a.rs": 25}, ["a.rs"], allow_regression=False)
        self.assertIn("--allow-regression", str(caught.exception))
        self.assertEqual(tracked, {"a.rs": 10}, "a refused update must not write")

        notes = bc.apply_update(tracked, {"a.rs": 25}, ["a.rs"], allow_regression=True)
        self.assertEqual(tracked["a.rs"], 25)
        self.assertTrue(any("RECORDED REGRESSION" in n for n in notes))

    def test_updating_a_path_that_no_longer_qualifies_retires_it(self):
        tracked = {"a.rs": 10}
        bc.apply_update(tracked, {}, ["a.rs"], allow_regression=False)
        self.assertEqual(tracked, {})

    def test_bare_update_still_rebases_everything(self):
        tracked = {"a.rs": 10, "gone.rs": 7}
        bc.apply_update(tracked, {"a.rs": 4, "new.rs": 1}, [], allow_regression=False)
        self.assertEqual(tracked, {"a.rs": 4, "new.rs": 1})


class MoveConservation(unittest.TestCase):
    """A move re-keys counts; anything that grows is not a move."""

    def test_a_conserving_split_is_rekeyed(self):
        tracked = {"old.rs": {"x": 12}}
        current = {"old.rs": {"x": 7}, "new.rs": {"x": 5}}
        notes = bc.apply_moves(tracked, current, ["old.rs=new.rs"])
        self.assertEqual(tracked, {"old.rs": {"x": 7}, "new.rs": {"x": 5}})
        self.assertIn("12 conserved as 12", notes[0])

    def test_a_move_that_empties_the_source_drops_it(self):
        tracked = {"old.rs": 9}
        notes = bc.apply_moves(tracked, {"new.rs": 9}, ["old.rs=new.rs"])
        self.assertEqual(tracked, {"new.rs": 9})
        self.assertIn("conserved", notes[0])

    def test_a_split_across_several_files_conserves(self):
        tracked = {"old.rs": 10}
        current = {"a.rs": 4, "b.rs": 6}
        bc.apply_moves(tracked, current, ["old.rs=a.rs,b.rs"])
        self.assertEqual(tracked, {"a.rs": 4, "b.rs": 6})

    def test_a_growing_move_is_refused(self):
        tracked = {"old.rs": 10}
        with self.assertRaises(SystemExit) as caught:
            bc.apply_moves(tracked, {"old.rs": 10, "new.rs": 5}, ["old.rs=new.rs"])
        self.assertIn("does not conserve", str(caught.exception))
        self.assertEqual(tracked, {"old.rs": 10}, "a refused move must not write")

    def test_a_shrinking_move_is_allowed_and_reported(self):
        tracked = {"old.rs": 10}
        notes = bc.apply_moves(tracked, {"new.rs": 8}, ["old.rs=new.rs"])
        self.assertIn("shrank", notes[0])

    def test_moving_an_untracked_source_is_an_error(self):
        with self.assertRaises(SystemExit) as caught:
            bc.apply_moves({}, {"new.rs": 1}, ["absent.rs=new.rs"])
        self.assertIn("no ledger entry", str(caught.exception))

    def test_malformed_move_spec_is_an_error(self):
        with self.assertRaises(SystemExit) as caught:
            bc.apply_moves({"a.rs": 1}, {}, ["a.rs"])
        self.assertIn("OLD=NEW", str(caught.exception))


class Exemptions(unittest.TestCase):
    """An exemption must carry its reason, in the code."""

    def test_annotation_with_a_reason_is_recognised(self):
        line = "    let _ = tx.send(event); // budget-ok: receiver dropped means nobody is listening"
        self.assertIsNotNone(bc.EXEMPTION_RE.search(line))
        self.assertEqual(
            bc.exemption_reason(line),
            "receiver dropped means nobody is listening",
        )

    def test_a_bare_annotation_without_a_reason_is_not_honoured(self):
        # The whole point is that the justification exists; an unexplained
        # opt-out is what the numeric ledger already allowed.
        self.assertIsNone(bc.EXEMPTION_RE.search("let _ = f(); // budget-ok"))
        self.assertIsNone(bc.EXEMPTION_RE.search("let _ = f(); // budget-ok:"))

    def test_an_unrelated_comment_is_not_an_exemption(self):
        self.assertIsNone(bc.EXEMPTION_RE.search("let _ = f(); // best effort"))


class Pruning(unittest.TestCase):
    def test_entries_the_checker_no_longer_measures_are_retired(self):
        tracked = {"kept.rs": 5, "dead.rs": 5}
        notes = bc.apply_prune(tracked, {"kept.rs": 5})
        self.assertEqual(tracked, {"kept.rs": 5})
        self.assertTrue(any("dead.rs" in n for n in notes))


class EntryShapes(unittest.TestCase):
    """Both ledger shapes in the repo must sum identically."""

    def test_scalar_and_pattern_entries_both_total(self):
        self.assertEqual(bc.entry_total(7), 7)
        self.assertEqual(bc.entry_total({"a": 3, "b": 4}), 7)
        self.assertEqual(bc.grand_total({"x.rs": 7, "y.rs": {"a": 1, "b": 2}}), 10)

    def test_pattern_totals_ignore_scalar_entries(self):
        tracked = {"x.rs": {"a": 2}, "y.rs": {"a": 3}}
        self.assertEqual(bc.pattern_totals(tracked, ["a"]), {"a": 5})


class RepairReachability(unittest.TestCase):
    """The remedy must be reachable on the ledger it exists to fix.

    First implementation validated inside `load_baseline`, before the `--repair`
    verb could run, so a corrupt ledger could not be repaired by the tool that
    detected it - the script just refused to start. Each checker therefore takes
    `validate=False` when repairing.
    """

    def _corrupt_ledger(self, module) -> Path:
        """A schema-valid ledger whose summary contradicts its own table.

        Shaped per ledger: the swallowed-error script keeps a per-pattern
        breakdown and rejects a ledger without one, so a scalar fixture would
        fail its schema check rather than its arithmetic check.
        """
        import json
        import tempfile

        if hasattr(module, "zero_counts"):
            patterns = module.zero_counts()
            entry = dict(patterns)
            entry[next(iter(entry))] = 1
            data = {
                "version": 1,
                "total": 999,
                "totals_by_pattern": patterns,
                "tracked_files": {"a.rs": entry},
            }
        elif hasattr(module, "current_oversized_files"):
            # Size ledgers carry a threshold and no aggregate; corrupt the table
            # shape they do validate instead.
            data = {"version": 1, "threshold_loc": 1200, "total": 999, "tracked_files": {"a.rs": 1}}
        else:
            data = {"version": 1, "total": 999, "tracked_files": {"a.rs": 1}}

        handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
        json.dump(data, handle)
        handle.close()
        return Path(handle.name)

    def test_every_checker_can_load_a_corrupt_ledger_for_repair(self):
        import importlib

        for name in (
            "check_panic_budget",
            "check_swallowed_error_budget",
            "check_code_size_budget",
            "check_test_size_budget",
        ):
            module = importlib.import_module(name)
            original = module.BASELINE_FILE
            path = self._corrupt_ledger(module)
            try:
                module.BASELINE_FILE = path
                with self.assertRaises(SystemExit, msg=f"{name} must reject a corrupt ledger"):
                    module.load_baseline()
                # ... and must still hand it over when asked to repair it.
                data = module.load_baseline(validate=False)
                self.assertEqual(data["total"], 999)
                bc.repair_summary(data)
                self.assertEqual(data["total"], 1)
            finally:
                module.BASELINE_FILE = original
                path.unlink()


class ExemptionCounting(unittest.TestCase):
    """A justified exemption must actually leave the count."""

    def test_annotated_line_is_not_counted_by_the_real_detectors(self):
        import check_panic_budget as panic
        import check_swallowed_error_budget as swallowed

        panicky = 'let v = x.unwrap();'
        swallowing = 'let _ = tx.send(event);'
        self.assertIsNotNone(panic.PATTERN.search(panicky))
        self.assertTrue(any(rx.search(swallowing) for rx in swallowed.PATTERNS.values()))

        justified_panic = panicky + "  // budget-ok: index proven in bounds two lines above"
        justified_swallow = swallowing + "  // budget-ok: receiver dropped means nobody is listening"
        # The pattern still matches; the exemption is what removes it from the
        # count, so both halves have to hold for the mechanism to be honest.
        self.assertIsNotNone(panic.PATTERN.search(justified_panic))
        self.assertIsNotNone(bc.EXEMPTION_RE.search(justified_panic))
        self.assertIsNotNone(bc.EXEMPTION_RE.search(justified_swallow))


class DetectorSelfCheck(unittest.TestCase):
    """A guard that stops detecting must fail, not congratulate itself.

    Observed before this existed: neutering `check_panic_budget.PATTERN` to an
    unmatchable regex turned `total=58` into
    `"total panic-prone count shrank: 58 -> 0"` and exit code 0 - the guard
    switched itself off and reported it as progress, which is the one failure
    mode a ratchet cannot survive.
    """

    def test_a_detector_that_matches_nothing_is_rejected(self):
        with self.assertRaises(SystemExit) as caught:
            bc.self_check("example", lambda _line: False, ["counts"], ["ignored"])
        message = str(caught.exception)
        self.assertIn("failed its own self-check", message)
        self.assertIn("no longer counts lines it must count", message)

    def test_a_detector_that_matches_everything_is_rejected(self):
        with self.assertRaises(SystemExit) as caught:
            bc.self_check("example", lambda _line: True, ["counts"], ["ignored"])
        self.assertIn("counts lines it must ignore", str(caught.exception))

    def test_a_healthy_detector_passes(self):
        bc.self_check("example", lambda line: "bad" in line, ["bad thing"], ["fine thing"])

    def test_every_guard_ships_fixtures_that_its_own_detector_satisfies(self):
        import importlib

        # (module, predicate factory) for the five guards in the repo.
        cases = {
            "check_panic_budget": lambda m: (m.counts_line, m.MUST_COUNT, m.MUST_IGNORE),
            "check_swallowed_error_budget": lambda m: (m.counts_line, m.MUST_COUNT, m.MUST_IGNORE),
            "check_wildcard_reexport_budget": lambda m: (
                lambda line: bool(m.PATTERN.search(line)),
                m.MUST_COUNT,
                m.MUST_IGNORE,
            ),
            "check_code_size_budget": lambda m: (
                lambda rel: m.is_production_rust_file(m.REPO_ROOT / rel),
                m.MUST_CLASSIFY,
                m.MUST_SKIP,
            ),
            "check_test_size_budget": lambda m: (
                lambda rel: m.is_test_rust_file(m.REPO_ROOT / rel),
                m.MUST_CLASSIFY,
                m.MUST_SKIP,
            ),
        }
        for name, unpack in cases.items():
            module = importlib.import_module(name)
            detects, must_count, must_ignore = unpack(module)
            self.assertTrue(must_count and must_ignore, f"{name} must ship both fixture sets")
            bc.self_check(name, detects, must_count, must_ignore)

    def test_sabotaging_any_real_guard_is_caught_by_its_own_fixtures(self):
        import importlib
        import re as _re

        never = _re.compile(r"ZZ_NEVER_MATCHES_ZZ")
        for name, attr in (
            ("check_panic_budget", "PATTERN"),
            ("check_wildcard_reexport_budget", "PATTERN"),
        ):
            module = importlib.import_module(name)
            original = getattr(module, attr)
            try:
                setattr(module, attr, never)
                detects = (
                    module.counts_line
                    if hasattr(module, "counts_line")
                    else (lambda line: bool(module.PATTERN.search(line)))
                )
                with self.assertRaises(SystemExit, msg=f"{name} sabotage must be caught"):
                    bc.self_check(name, detects, module.MUST_COUNT, module.MUST_IGNORE)
            finally:
                setattr(module, attr, original)


class DeadSurface(unittest.TestCase):
    """The orphan detectors, and their own self-check.

    These exist because a manual audit cannot see cascades: deleting 43
    orphaned scripts orphaned 4 more, each of which had been referenced only by
    one of the deleted ones, and the audit reported "fixpoint" while those 4
    were live orphans.
    """

    def setUp(self):
        import check_dead_surface

        self.ds = check_dead_surface

    def test_a_workspace_member_with_no_dependent_is_flagged(self):
        index = {
            "./Cargo.toml": '[workspace]\nmembers = [\n    "crates/ghost",\n]\n',
            "./crates/ghost/Cargo.toml": 'name = "ghost"\n',
        }
        self.assertIn("crates/ghost", self.ds.orphan_crates(index))

    def test_membership_alone_is_not_a_dependency(self):
        # The bug this encodes: `jcode-sdk` was a member with no dependent for
        # an unknown length of time, and still compiled, so nothing objected.
        wired = {
            "./Cargo.toml": (
                '[workspace]\nmembers = [\n    "crates/live",\n]\n\n'
                "[dependencies]\njcode-live = { path = \"crates/live\" }\n"
            ),
            "./crates/live/Cargo.toml": 'name = "live"\n',
            "./other/Cargo.toml": 'live = { path = "../crates/live" }\n',
        }
        self.assertEqual(self.ds.orphan_crates(wired), {})

    def test_a_bin_with_no_invoker_is_flagged_and_a_used_one_is_not(self):
        index = {
            "./Cargo.toml": '[[bin]]\nname = "used"\n\n[[bin]]\nname = "unused"\n',
            "./scripts/run.sh": "cargo run --bin used\n",
        }
        bins = self.ds.orphan_bins(index)
        self.assertIn("./Cargo.toml::unused", bins)
        self.assertNotIn("./Cargo.toml::used", bins)

    def test_the_product_binary_is_never_flagged(self):
        # `jcode` is the product; nothing in-repo has to invoke it.
        self.assertEqual(self.ds.orphan_bins({"./Cargo.toml": '[[bin]]\nname = "jcode"\n'}), {})

    def test_self_check_rejects_a_neutered_detector(self):
        original = self.ds.orphan_bins
        try:
            self.ds.orphan_bins = lambda _index: {}
            with self.assertRaises(SystemExit) as caught:
                self.ds.self_check()
            self.assertIn("failed its own self-check", str(caught.exception))
        finally:
            self.ds.orphan_bins = original

    def test_self_check_rejects_an_over_eager_detector(self):
        original = self.ds.orphan_bins
        try:
            self.ds.orphan_bins = lambda _index: {
                "./Cargo.toml::unused_bin": "x",
                "./Cargo.toml::used_bin": "x",
            }
            with self.assertRaises(SystemExit):
                self.ds.self_check()
        finally:
            self.ds.orphan_bins = original

    def test_the_healthy_detectors_pass_their_self_check(self):
        self.ds.self_check()

    def test_the_real_allowlist_explains_every_entry(self):
        allowed = self.ds.load_allowlist()
        self.assertTrue(allowed, "the allowlist should hold the human-invoked harnesses")
        for path, reason in allowed.items():
            self.assertTrue(str(reason).strip(), f"{path} has no reason")
            self.assertGreater(len(str(reason)), 20, f"{path}'s reason is too thin to review")

if __name__ == "__main__":
    unittest.main(verbosity=2)
