#!/usr/bin/env python3
"""Summarize current-contract fanout runs; overlap is never a relevance judgment."""
import argparse
import collections
import hashlib
import json
import math
from pathlib import Path
import statistics

POLICIES = ("full", "batch", "stream", "diversity")


def distribution(values):
    values = sorted(v for v in values if v is not None)
    return {"n": len(values), "p50": statistics.median(values) if values else None,
            "p95": values[max(0, math.ceil(len(values) * .95) - 1)] if values else None}


def key(run):
    return run["query_id"], run["trial"], run["fresh_clients"], run["policy"]


def summarize(metadata, runs, judgments=None):
    judgments = judgments or {}
    indexed = {key(run): run for run in runs}
    if len(indexed) != len(runs):
        raise ValueError("duplicate run identifiers")
    expected = {(q["id"], trial, fresh, policy)
                for q in metadata["corpus"]["queries"][:metadata["query_limit"]]
                for trial in range(1, metadata["trials"] + 1)
                for fresh in (True, False) for policy in POLICIES}
    if not indexed.keys() <= expected:
        raise ValueError("unexpected run identifiers")
    groups = []
    for fresh in (True, False):
        for policy in POLICIES:
            selected = [r for r in runs if r["fresh_clients"] == fresh and r["policy"] == policy]
            overlaps, paired_deltas, quality = [], [], []
            unknown_results = 0
            for run in selected:
                baseline = indexed.get((*key(run)[:3], "full"))
                if baseline is not None:
                    # Keys come from the production canonicalizer. Empty sets are not perfect overlap.
                    left = set(run["final_keys"])
                    right = set(baseline["final_keys"])
                    overlaps.append(len(left & right) / 5)
                    paired_deltas.append(run["elapsed_ms"] - baseline["elapsed_ms"])
                scores = [judgments.get(run["query_id"], {}).get(r["url"])
                          for r in run["final_results"]]
                if any(score is not None and type(score) is not bool for score in scores):
                    raise ValueError("judgments must be true, false, or null")
                unknown_results += sum(score is None for score in scores)
                # Missing slots score zero only when every returned URL is judged.
                # Errors do not masquerade as fully judged empty retrievals.
                if not run["error"] and all(score is not None for score in scores):
                    quality.append(sum(scores) / 5)
            groups.append({
                "policy": policy, "fresh_clients": fresh, "scheduled_observed": len(selected),
                "errors": sum(r["error"] is not None for r in selected),
                "target_met": sum(r["threshold_met_at_return"] for r in selected),
                "two_providers_at_return": sum(len(r["contributing_providers"]) >= 2 for r in selected),
                "stop_condition_observed": sum(r["collector"]["threshold_observed_ms"] is not None for r in selected),
                "latency_ms": distribution(r["elapsed_ms"] for r in selected),
                "successful_latency_ms": distribution(r["elapsed_ms"] for r in selected if not r["error"]),
                "time_to_five_ms": distribution(r["collector"]["first_five_unique_ms"] for r in selected),
                "decoded_bytes": distribution(sum(p["decompressed_bytes_received"] for p in r["providers"].values()) for r in selected),
                "return_after_threshold_ms": distribution(r["elapsed_ms"] - r["collector"]["threshold_observed_ms"] for r in selected if r["collector"]["threshold_observed_ms"] is not None),
                "provider_contributions": distribution(len(r["contributing_providers"]) for r in selected),
                "paired_latency_delta_vs_full_ms": distribution(paired_deltas),
                "top_five_slot_overlap_vs_full": distribution(overlaps),
                "precision_at_five_fully_judged_runs": distribution(quality),
                "unjudged_returned_results": unknown_results,
                "outcomes": dict(collections.Counter(d["outcome"] for r in selected for d in r["diagnostics"])),
            })
    matrix = []
    for engine in metadata["enabled_providers"]:
        observed = [r["providers"][engine] for r in runs if engine in r["providers"]]
        diagnostics = [d for r in runs for d in r["diagnostics"] if d["engine"] == engine]
        matrix.append({
            "provider": engine, "scheduled_searches": len(runs),
            "observed_searches": len(observed), "diagnostic_searches": len(diagnostics),
            "protocols": sorted({p["http_version"] for p in observed if p["http_version"]}),
            "statuses": dict(collections.Counter(str(p["http_status"]) for p in observed if p["http_status"])),
            "outcomes": dict(collections.Counter(d["outcome"] for d in diagnostics)),
            "headers_ms": distribution(p["headers_ms"] for p in observed if p["http_version"]),
            "first_record_ms": distribution(p["first_record_ms"] for p in observed),
            "fifth_record_ms": distribution(p["fifth_unique_record_ms"] for p in observed),
            "eof_ms": distribution(p["body_eof_ms"] for p in observed),
            "pre_eof_record_observations": sum(p["first_record_ms"] is not None and
                (p["body_eof_ms"] is None or p["first_record_ms"] < p["body_eof_ms"]) for p in observed),
            "incremental_parser_worker_microseconds": distribution(p["parse_microseconds"] for p in observed if p["parse_microseconds"]),
            "caveat": "logical-search aggregation; not per-attempt; cancellation and blocks censor feasibility",
        })
    return {"schema_version": 1, "observed_runs": len(runs), "expected_runs": len(expected),
            "complete": indexed.keys() == expected, "groups": groups, "provider_matrix": matrix,
            "limitations": ["Small-sample p95 is descriptive, not a stable tail estimate.",
                "Full fanout is deadline-bounded, not exhaustive web coverage.",
                "Time-to-five is first observed, even if a later error retracts provisional records.",
                "Sequential live pairs have provider/network variance; reused pools initially start cold.",
                "URL overlap is not relevance, freshness, or evidence correctness.",
                "Missing instrumentation remains unmeasured; see metadata.unmeasured."]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--judgments", type=Path,
                        help="versioned JSON with judgments: {query_id: {url: true|false|null}}")
    args = parser.parse_args()
    metadata = json.loads((args.input / "metadata.json").read_text())
    raw = (args.input / "runs.jsonl").read_bytes()
    runs = [json.loads(line) for line in raw.splitlines() if line.strip()]
    judgments = json.loads(args.judgments.read_text()) if args.judgments else None
    if judgments is not None and judgments.get("version") != 1:
        raise ValueError("judgments require schema version 1")
    summary = summarize(metadata, runs, judgments["judgments"] if judgments else None)
    summary["input_sha256"] = hashlib.sha256(raw).hexdigest()
    summary["metadata_sha256"] = hashlib.sha256((args.input / "metadata.json").read_bytes()).hexdigest()
    summary["judgments_sha256"] = hashlib.sha256(args.judgments.read_bytes()).hexdigest() if judgments else None
    summary["completion_marker"] = (args.input / "COMPLETE").exists()
    with args.output.open("x") as output:
        json.dump(summary, output, indent=2)
        output.write("\n")
    print(f"Recorded {summary['observed_runs']}/{summary['expected_runs']} runs; summary: {args.output}")


if __name__ == "__main__":
    main()
