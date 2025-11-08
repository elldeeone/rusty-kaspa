#!/usr/bin/env python3
"""Run run-simpa-pruning.sh and kill it once a log line appears."""
import argparse
import os
import re
import signal
import subprocess
import sys
import threading
import time


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--kill-on",
        default="Header and Block pruning: starting traversal",
        help="Substring to watch for in the Simpa log before sending SIGKILL",
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=0.0,
        help="Seconds to wait after the trigger line before killing the process",
    )
    parser.add_argument(
        "cmd",
        nargs=argparse.REMAINDER,
        default=["./pruning-optimisation/run-simpa-pruning.sh"],
        help="Command to run (default: run-simpa-pruning.sh). Pass -- to separate",
    )
    return parser.parse_args()


def main():
    args = parse_args()
    if args.cmd and args.cmd[0] == "--":
        cmd = args.cmd[1:]
    else:
        cmd = args.cmd or ["./pruning-optimisation/run-simpa-pruning.sh"]

    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    log_path_holder = {"path": None}
    stop_event = threading.Event()

    def monitor_log():
        while not stop_event.is_set():
            path = log_path_holder["path"]
            if not path:
                if proc.poll() is not None:
                    return
                time.sleep(0.05)
                continue
            if not os.path.exists(path):
                if proc.poll() is not None:
                    return
                time.sleep(0.05)
                continue
            with open(path, "r") as f:
                # tail the file
                while not stop_event.is_set():
                    line = f.readline()
                    if not line:
                        if proc.poll() is not None:
                            return
                        time.sleep(0.05)
                        continue
                    if args.kill_on in line:
                        time.sleep(max(args.delay, 0.0))
                        try:
                            os.kill(proc.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        return

    monitor_thread = threading.Thread(target=monitor_log, daemon=True)
    monitor_thread.start()

    log_line_re = re.compile(r"^>> Running Simpa\. Log: (.+)$")
    try:
        for line in proc.stdout:
            print(line, end="")
            match = log_line_re.match(line.strip())
            if match:
                log_path_holder["path"] = match.group(1)
    finally:
        stop_event.set()
        monitor_thread.join()

    return proc.wait()


if __name__ == "__main__":
    sys.exit(main())
