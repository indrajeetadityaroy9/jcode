#!/usr/bin/env python3
from __future__ import annotations

import contextlib
import importlib.util
import io
import sys
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("analyze_runtime_memory_log.py")
SPEC = importlib.util.spec_from_file_location("runtime_memory_analyzer", MODULE_PATH)
assert SPEC and SPEC.loader
analyzer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = analyzer
SPEC.loader.exec_module(analyzer)

MB = 1024 * 1024


def sample(
    *,
    timestamp_ms: int,
    instance_id: str,
    footprint_mb: int,
    allocated_mb: int,
    live_sessions: int | None = None,
    connected_clients: int = 0,
    total_json_mb: int = 0,
    platform: str = "linux",
) -> analyzer.Sample:
    """Build one sample.

    `platform="linux"` reports the PSS family; `platform="macos"` reproduces a
    real macOS record, where `os.pss_bytes` is null and only `rss_bytes` and the
    RSS splits carry a value.
    """
    sessions = None
    kind = "process"
    if live_sessions is not None:
        kind = "attribution"
        sessions = {
            "live_count": live_sessions,
            "total_json_bytes": total_json_mb * MB,
            "total_payload_text_bytes": 0,
            "total_provider_cache_json_bytes": 0,
            "total_tool_result_bytes": 0,
            "total_large_blob_bytes": 0,
            "top_by_json_bytes": [],
        }
    if platform == "macos":
        os_info: dict[str, int | None] = {
            "pss_bytes": None,
            "pss_anon_bytes": None,
            "pss_file_bytes": None,
            "rss_anon_bytes": (footprint_mb // 2) * MB,
            "rss_file_bytes": (footprint_mb - footprint_mb // 2) * MB,
        }
    else:
        os_info = {"pss_bytes": footprint_mb * MB, "pss_anon_bytes": footprint_mb * MB}
    raw = {
        "server": {"id": instance_id},
        "process": {
            "rss_bytes": footprint_mb * MB,
            "os": os_info,
            "allocator": {
                "stats": {
                    "allocated_bytes": allocated_mb * MB,
                    "retained_bytes": 0,
                }
            },
        },
        "process_diagnostics": {"allocator_retained_resident_estimate_bytes": 0},
        "clients": {"connected_count": connected_clients},
        "sessions": sessions,
    }
    return analyzer.Sample(
        path=Path("server-runtime-memory-test.jsonl"),
        line_no=timestamp_ms,
        raw=raw,
        timestamp_ms=timestamp_ms,
        kind=kind,
        target="server",
        instance_id=instance_id,
        source=f"{kind}:test",
        trigger_category="test",
        trigger_reason="unit",
        sessions=sessions,
        totals=None,
    )


class RuntimeMemoryAnalyzerTests(unittest.TestCase):
    def test_latest_instance_filter_prevents_cross_reload_spikes(self) -> None:
        samples = [
            sample(timestamp_ms=1, instance_id="old", footprint_mb=1800, allocated_mb=1700),
            sample(timestamp_ms=2, instance_id="old", footprint_mb=1900, allocated_mb=1800),
            sample(timestamp_ms=3, instance_id="new", footprint_mb=80, allocated_mb=40),
            sample(timestamp_ms=4, instance_id="new", footprint_mb=120, allocated_mb=70),
        ]

        selected = analyzer.select_latest_instances(samples)

        self.assertEqual({item.instance_id for item in selected}, {"new"})
        summary = analyzer.process_summary(selected)
        self.assertEqual(summary["footprint_metric"], "PSS")
        self.assertEqual(summary["baseline_footprint_bytes"], 80 * MB)
        self.assertEqual(summary["net_footprint_growth_bytes"], 40 * MB)

    def test_explicit_instance_selection_preserves_historical_incident(self) -> None:
        samples = [
            sample(
                timestamp_ms=1, instance_id="old", footprint_mb=70, allocated_mb=38, live_sessions=9
            ),
            sample(
                timestamp_ms=2,
                instance_id="old",
                footprint_mb=3900,
                allocated_mb=3700,
                live_sessions=1140,
                connected_clients=5,
            ),
            sample(
                timestamp_ms=3, instance_id="new", footprint_mb=80, allocated_mb=40, live_sessions=2
            ),
        ]

        selected = analyzer.select_instance(samples, "old")
        summary = analyzer.summarize_target(selected, top_n=5, min_spike_bytes=8 * MB)
        inventory = analyzer.instance_inventory(samples)

        self.assertEqual({item.instance_id for item in selected}, {"old"})
        self.assertEqual(summary["session_population"]["peak_live_sessions"], 1140)
        self.assertEqual(summary["incident"]["primary_cause"], "runaway_live_session_population")
        self.assertEqual([item["instance_id"] for item in inventory], ["new", "old"])

    def test_runaway_session_population_is_primary_cause(self) -> None:
        samples = [
            sample(
                timestamp_ms=1,
                instance_id="server",
                footprint_mb=70,
                allocated_mb=38,
                live_sessions=9,
                connected_clients=0,
                total_json_mb=1,
            ),
            sample(
                timestamp_ms=2,
                instance_id="server",
                footprint_mb=3900,
                allocated_mb=3700,
                live_sessions=1140,
                connected_clients=5,
                total_json_mb=360,
            ),
        ]

        summary = analyzer.summarize_target(samples, top_n=5, min_spike_bytes=8 * MB)
        incident = summary["incident"]
        population = summary["session_population"]

        self.assertEqual(incident["severity"], "critical")
        self.assertEqual(incident["primary_cause"], "runaway_live_session_population")
        self.assertEqual(incident["confidence"], "high")
        self.assertEqual(population["net_live_session_growth"], 1131)
        self.assertGreater(population["allocator_growth_per_added_session_bytes"], 2 * MB)
        self.assertIn("Pause or cap", incident["recommended_actions"][0]["action"])

    def test_allocator_retention_has_purge_first_action(self) -> None:
        current = sample(
            timestamp_ms=1,
            instance_id="server",
            footprint_mb=1500,
            allocated_mb=500,
            live_sessions=8,
            connected_clients=5,
            total_json_mb=100,
        )
        current.raw["process_diagnostics"]["allocator_retained_resident_estimate_bytes"] = 600 * MB
        coverage = analyzer.build_coverage_report(current)
        incident = analyzer.build_incident_assessment(
            [current], analyzer.process_summary([current]), coverage, analyzer.session_population_summary([current])
        )

        self.assertEqual(incident["primary_cause"], "allocator_retention")
        self.assertIn("purge", incident["recommended_actions"][0]["action"].lower())

    def test_rss_only_log_reports_a_real_footprint_and_names_rss(self) -> None:
        """The macOS blind spot: PSS is absent, so every number came from RSS.

        Before the footprint fallback the process filter dropped these samples
        entirely, `final_pss_bytes` was missing, and the incident block printed
        "final PSS 0.0 MB" for a process holding tens of megabytes.
        """
        samples = [
            sample(
                timestamp_ms=1,
                instance_id="server",
                footprint_mb=38,
                allocated_mb=3,
                live_sessions=0,
                platform="macos",
            ),
            sample(
                timestamp_ms=2,
                instance_id="server",
                footprint_mb=53,
                allocated_mb=4,
                live_sessions=0,
                platform="macos",
            ),
        ]

        summary = analyzer.summarize_target(samples, top_n=5, min_spike_bytes=8 * MB)
        process = summary["process"]
        incident = summary["incident"]

        self.assertEqual(process["footprint_metric"], "RSS")
        self.assertEqual(process["final_footprint_bytes"], 53 * MB)
        self.assertEqual(process["baseline_footprint_bytes"], 38 * MB)
        self.assertEqual(process["net_footprint_growth_bytes"], 15 * MB)
        self.assertEqual(summary["coverage"]["footprint_metric"], "RSS")
        self.assertEqual(summary["coverage"]["footprint_bytes"], 53 * MB)

        # Spikes are computable from RSS; the old PSS-only filter reported none.
        self.assertEqual(len(summary["top_spikes"]), 1)
        self.assertEqual(summary["top_spikes"][0]["delta_footprint_bytes"], 15 * MB)
        self.assertEqual(summary["top_spikes"][0]["footprint_metric"], "RSS")

        # Every label names RSS, and no label claims a 0.0 MB footprint.
        self.assertEqual(incident["footprint_metric"], "RSS")
        self.assertIn("final RSS 53.0 MB", incident["evidence"])
        self.assertIn("net RSS growth +15.0 MB", incident["evidence"])
        for item in incident["evidence"]:
            self.assertNotIn("PSS", item)
            self.assertNotIn("0.0 MB", item)
        self.assertIn("os.pss_bytes has no macOS source", incident["footprint_metric_note"])

    def test_rendered_report_names_rss_and_never_claims_a_zero_footprint(self) -> None:
        """The printed report is the artifact operators read.

        Pre-fix it said `evidence: final PSS 0.0 MB` for a 53 MB process and
        omitted the whole Process memory section.
        """
        samples = [
            sample(
                timestamp_ms=1,
                instance_id="server",
                footprint_mb=38,
                allocated_mb=3,
                live_sessions=0,
                platform="macos",
            ),
            sample(
                timestamp_ms=2,
                instance_id="server",
                footprint_mb=53,
                allocated_mb=4,
                live_sessions=0,
                platform="macos",
            ),
        ]
        summary = analyzer.summarize_target(samples, top_n=5, min_spike_bytes=8 * MB)

        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            analyzer.print_human(summary, [])
        report = buffer.getvalue()

        self.assertIn("final RSS:    53.0 MB (+15.0 MB)", report)
        self.assertIn("- evidence: final RSS 53.0 MB", report)
        self.assertIn("- evidence: net RSS growth +15.0 MB", report)
        self.assertIn("PSS split n/a on macOS", report)
        self.assertNotIn("final PSS", report)
        self.assertNotIn("evidence: final PSS 0.0 MB", report)
        self.assertNotIn("net PSS growth", report)

    def test_pss_bearing_log_still_reports_pss(self) -> None:
        """A Linux log analyzes exactly as before: PSS wins the fallback."""
        samples = [
            sample(
                timestamp_ms=1,
                instance_id="server",
                footprint_mb=38,
                allocated_mb=3,
                live_sessions=0,
            ),
            sample(
                timestamp_ms=2,
                instance_id="server",
                footprint_mb=53,
                allocated_mb=4,
                live_sessions=0,
            ),
        ]

        summary = analyzer.summarize_target(samples, top_n=5, min_spike_bytes=8 * MB)

        self.assertEqual(summary["process"]["footprint_metric"], "PSS")
        self.assertEqual(summary["process"]["final_footprint_bytes"], 53 * MB)
        self.assertEqual(summary["coverage"]["footprint_metric"], "PSS")
        self.assertEqual(summary["top_spikes"][0]["footprint_metric"], "PSS")
        self.assertIn("final PSS 53.0 MB", summary["incident"]["evidence"])

    def test_pss_wins_over_rss_when_both_are_present(self) -> None:
        """Mirror of the Rust `os.pss_bytes.or(rss_bytes)` order."""
        entry = sample(timestamp_ms=1, instance_id="server", footprint_mb=100, allocated_mb=10)
        entry.raw["process"]["rss_bytes"] = 900 * MB

        self.assertEqual(entry.footprint_bytes, 100 * MB)
        self.assertEqual(entry.footprint_metric, "PSS")

    def test_unmeasured_footprint_is_none_not_zero(self) -> None:
        """Absence must never render as a 0.0 MB measurement."""
        entry = sample(timestamp_ms=1, instance_id="server", footprint_mb=100, allocated_mb=10)
        entry.raw["process"]["os"]["pss_bytes"] = None
        entry.raw["process"]["rss_bytes"] = None

        self.assertIsNone(entry.footprint_bytes)
        self.assertIsNone(entry.footprint_metric)
        self.assertEqual(analyzer.process_summary([entry]), {})
        self.assertEqual(analyzer.build_coverage_report(entry)["footprint_bytes"], None)
        incident = analyzer.build_incident_assessment([entry], {}, None, {})
        self.assertIn("final unmeasured n/a", incident["evidence"])
        self.assertIn("none recorded", incident["footprint_metric_note"])

    def test_spikes_never_mix_pss_and_rss_endpoints(self) -> None:
        """RSS counts shared pages in full; a mixed delta would be fiction."""
        samples = [
            sample(
                timestamp_ms=1, instance_id="server", footprint_mb=38, allocated_mb=3, platform="macos"
            ),
            sample(timestamp_ms=2, instance_id="server", footprint_mb=900, allocated_mb=4),
        ]

        self.assertEqual(analyzer.compute_spikes(samples, min_spike_bytes=8 * MB), [])


if __name__ == "__main__":
    unittest.main()
