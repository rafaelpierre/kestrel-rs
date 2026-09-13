#!/usr/bin/env python3
"""Sequential controlled subprocess/retained-client trials for issue #80."""
import argparse
import hashlib
import http.server
import json
import math
import os
import pathlib
import platform
import random
import subprocess
import threading
import time
import urllib.parse


class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        parsed = urllib.parse.urlsplit(self.path)
        query = urllib.parse.parse_qs(parsed.query).get("q", [""])[0]
        if parsed.path == "/page":
            time.sleep(0.05)
            body = b"<html><main><p>fixture evidence content for the controlled budget experiment.</p></main></html>"
        elif parsed.path == "/bing" and query == "empty":
            body = b"<html><li class='b_no'>No results</li></html>"
        elif parsed.path == "/bing" and query == "page":
            url = f"http://127.0.0.1:{self.server.server_port}/page"
            body = f'<html><ol id="b_results"><li class="b_algo"><h2><a href="{url}">fixture page</a></h2><p>fixture evidence</p></li></ol></html>'.encode()
        else:
            # Send headers and a non-result prefix, then wait for cancellation.
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", "100000")
            self.end_headers()
            self.wfile.write(b"<html><body>")
            self.wfile.flush()
            self.connection.settimeout(10)
            try:
                while self.connection.recv(1024):
                    pass
            except (TimeoutError, ConnectionError):
                pass
            self.close_connection = True
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def trial(command, env, folder, label):
    started = time.perf_counter_ns()
    with subprocess.Popen(command, env=env, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True) as process:
        spawned = time.perf_counter_ns()
        try:
            stdout, stderr = process.communicate(timeout=180)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate()
            raise
    ended = time.perf_counter_ns()
    (folder / f"{label}.stdout").write_text(stdout)
    (folder / f"{label}.stderr").write_text(stderr)
    events = []
    for line in stderr.splitlines():
        if line.startswith("BUDGET_PROBE "):
            events = json.loads(line.removeprefix("BUDGET_PROBE "))
    return {"command": command, "returncode": process.returncode,
            "launch_ns": started, "spawn_return_ns": spawned, "reaped_ns": ended,
            "external_seconds": (ended - started) / 1e9, "events": events,
            "stdout_file": f"{label}.stdout", "stderr_file": f"{label}.stderr"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--compare-binary", type=pathlib.Path, help="Interleave a second CLI binary in the same conditions")
    parser.add_argument("--retained", type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--trials", type=int, default=10)
    parser.add_argument("--budget", type=float, default=1)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--no-fetch-only", action="store_true", help="Restrict the matrix to discovery for boundary checks")
    parser.add_argument("--query", default="ambient music new releases September 2026")
    parser.add_argument("--baseline", action="store_true", help="Unmodified binary: tiny-budget controls, no endpoint overrides")
    args = parser.parse_args()
    if args.trials < 1 or not math.isfinite(args.budget) or args.budget <= 0:
        parser.error("trials and finite budget must be positive")
    if args.baseline and (args.live or args.budget > 1e-9 or args.retained):
        parser.error("baseline requires a budget <= 1 ns, no live mode and no retained binary")
    args.binary = args.binary.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("KESTRELSEARCH_", "BUDGET_"))}
    # Keep fixture traffic local and prevent ambient proxy variables from changing it.
    # Live trials retain proxy configuration, without recording its contents.
    if not args.live:
        env = {k: v for k, v in env.items() if k.lower() not in
               ("http_proxy", "https_proxy", "all_proxy", "no_proxy")}
        env["NO_PROXY"] = "127.0.0.1,localhost"
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    if not args.live and not args.baseline:
        env["BUDGET_FIXTURE"] = f"http://127.0.0.1:{server.server_port}"
    metadata = {"platform": platform.platform(), "machine": platform.machine(),
                "python": platform.python_version(), "clock": vars(time.get_clock_info("perf_counter")),
                "binary_sha256": sha(args.binary), "profile": "release", "args": vars(args),
                "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
                "cpu": subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip(),
                "proxy_variables_present": any(k.lower().endswith("_proxy") for k in env)}
    if args.retained:
        metadata["retained_sha256"] = sha(args.retained)
    if args.compare_binary:
        metadata["compare_binary_sha256"] = sha(args.compare_binary)
    (args.output / "metadata.json").write_text(json.dumps(metadata, default=str, indent=2))
    scenarios = [args.query] if args.live else (["empty"] if args.baseline else ["hang", "empty", "page"])
    conditions = [(scenario, fetch, trace) for scenario in scenarios
                  for fetch in ([False] if args.no_fetch_only else [False, True]) for trace in (False, True)]
    records = []
    rng = random.Random(80)
    try:
        for round_number in range(args.trials):
            rng.shuffle(conditions)
            for scenario, fetch, trace in conditions:
                label = f"cli-{round_number}-{scenarios.index(scenario)}-{int(fetch)}-{int(trace)}"
                trial_env = env.copy()
                if trace:
                    trial_env.update(KESTRELSEARCH_PROVIDER_TRACE_DIR=str(args.output / label / "providers"),
                                     KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR=str(args.output / label),
                                     KESTRELSEARCH_BENCHMARK_RUN_ID=label)
                command = [str(args.binary), "search", scenario, "--search-budget", str(args.budget),
                           "--output", "json", "--no-rank"]
                if not args.live:
                    command += ["-e", "bing", "-e", "yahoo", "--search-concurrency", "1"]
                else:
                    command += ["--min-results", "1", "--top-k", "1"]
                if not fetch:
                    command.append("--no-fetch")
                binaries = [args.binary]
                if args.compare_binary:
                    binaries.append(args.compare_binary.resolve())
                    rng.shuffle(binaries)
                for binary in binaries:
                    variant = "primary" if binary == args.binary else "comparison"
                    command[0] = str(binary)
                    if trace and args.compare_binary:
                        trial_env.update(KESTRELSEARCH_PROVIDER_TRACE_DIR=str(args.output / (label + "-" + variant) / "providers"),
                                         KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR=str(args.output / (label + "-" + variant)),
                                         KESTRELSEARCH_BENCHMARK_RUN_ID=label + "-" + variant)
                    result = trial(command, trial_env, args.output, label + "-" + variant)
                    result.update(kind="cli", variant=variant, round=round_number, scenario=scenario, fetch=fetch, trace=trace)
                    records.append(result)
                (args.output / "runs.json").write_text(json.dumps(records, indent=2))
            print(f"CLI round {round_number + 1}/{args.trials} complete", flush=True)
        if args.retained:
            for scenario, fetch, trace in conditions:
                label = f"retained-{scenarios.index(scenario)}-{int(fetch)}-{int(trace)}"
                trial_env = env.copy()
                if trace:
                    trial_env["KESTRELSEARCH_PROVIDER_TRACE_DIR"] = str(args.output / label / "providers")
                command = [str(args.retained.resolve()), str(args.trials), str(args.budget),
                           "fetch" if fetch else "no-fetch", scenario]
                result = trial(command, trial_env, args.output, label)
                result.update(kind="retained", scenario=scenario, fetch=fetch, trace=trace)
                records.append(result)
                (args.output / "runs.json").write_text(json.dumps(records, indent=2))
                print(f"{label} complete", flush=True)
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
