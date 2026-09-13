#!/usr/bin/env python3
"""Validate fixture outcomes and summarize milliseconds (nearest-rank p95)."""
import argparse
import collections
import json
import math
import pathlib
import statistics


def distribution(values):
    values = sorted(v * 1000 for v in values)
    return {"n": len(values), "min": values[0], "p50": statistics.median(values),
            "p95": values[math.ceil(0.95 * len(values)) - 1], "max": values[-1]}


def summarize(folder):
    metadata = json.loads((folder / "metadata.json").read_text())
    records = json.loads((folder / "runs.json").read_text())
    groups = collections.defaultdict(lambda: collections.defaultdict(list))
    fixture = not metadata["args"]["live"] and not metadata["args"]["baseline"]
    for row in records:
        key = f'{row["kind"]}/{row["scenario"]}/fetch={row["fetch"]}/trace={row["trace"]}'
        if metadata["args"].get("compare_binary"):
            key += "/" + row["variant"]
        out = groups[key]
        events = row["events"]
        named = dict(events)
        if row["kind"] == "cli":
            if fixture:
                expected_code = 1 if row["scenario"] == "hang" else 0
                assert row["returncode"] == expected_code, row
                if expected_code == 0:
                    result = json.loads((folder / row["stdout_file"]).read_text())
                    assert len(result["results"]) == (1 if row["scenario"] == "page" else 0)
                    if row["fetch"] and row["scenario"] == "page":
                        assert "fixture evidence" in result["results"][0]["content"]
            out["external"].append(row["external_seconds"])
            if row["returncode"] == 0:
                payload = json.loads((folder / row["stdout_file"]).read_text())
                out["command"].append(payload["elapsed_seconds"])
            if "runtime_dropped" in named:
                out["outside_main_observation"].append(row["external_seconds"] - named["runtime_dropped"])
        else:
            assert row["returncode"] == 0
            calls = [json.loads(line) for line in (folder / row["stdout_file"]).read_text().splitlines()]
            for call in calls:
                if fixture:
                    assert (call["error"] is not None) == (row["scenario"] == "hang")
                    assert call["results"] == {"hang": None, "empty": 0, "page": 1}[row["scenario"]]
                    assert call["content_count"] == int(row["scenario"] == "page" and row["fetch"])
                out["search_first" if call["round"] == 0 else "search_reused"].append(call["search_seconds"])
                if row["fetch"] and row["scenario"] == "page":
                    out["fetch_calls"].append(call["fetch_seconds"])
        pairs = [("runtime", "main_entry", "runtime_ready"), ("parse", "parse_begin", "parse_end"),
                 ("initialize", "client_begin", "client_end"),
                 ("standard_client", "standard_begin", "standard_end"),
                 ("yahoo_client", "yahoo_begin", "yahoo_end"),
                 ("fetch_client", "fetch_client_begin", "fetch_client_end"),
                 ("search", "deadline_start", "collector_return"),
                 ("fetch", "fetch_begin", "fetch_end"),
                 ("serialize_output", "serialize_begin", "output_done"),
                 ("handler_tail", "output_done", "handler_return"),
                 ("runtime_shutdown", "handler_return", "runtime_dropped"),
                 ("client_drop", "client_drop_begin", "client_drop_end"),
                 ("provider_drop", "provider_drop_begin", "provider_drop_end")]
        for metric, begin, end in pairs:
            pending = None
            for name, seconds in events:
                if name == begin:
                    pending = seconds
                elif name == end and pending is not None:
                    out[metric].append(seconds - pending)
                    pending = None
    return {key: {metric: distribution(values) for metric, values in metrics.items()}
            for key, metrics in groups.items()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=pathlib.Path)
    args = parser.parse_args()
    print(json.dumps(summarize(args.folder), indent=2, sort_keys=True))
