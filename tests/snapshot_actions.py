"""Bounded action completion and watcher shutdown for real Herdr snapshot sessions."""

import fcntl
import time


def completed_action(api, action):
    invocation = api("plugin", "action", "invoke", f"herdrpp.{action}")
    log_id = invocation["log"]["log_id"]
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        logs = api(
            "plugin", "log", "list", "--plugin", "herdrpp", "--limit", "20"
        )["logs"]
        completed = next(
            (
                log
                for log in logs
                if log["log_id"] == log_id and log["status"] != "running"
            ),
            None,
        )
        if completed is not None:
            assert completed["status"] == "succeeded", completed
            return completed
        time.sleep(0.05)
    raise TimeoutError(f"Plugin action {action} did not finish: {logs}")


def stop_startup_watcher(api, state_directory):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        logs = api(
            "plugin", "log", "list", "--plugin", "herdrpp", "--limit", "20"
        )["logs"]
        startup = next(
            (
                log
                for log in logs
                if log.get("event") == "startup" and log["status"] != "running"
            ),
            None,
        )
        if startup is not None:
            assert startup["status"] == "succeeded", startup
            break
        time.sleep(0.05)
    else:
        raise TimeoutError(f"Plugin startup did not finish: {logs}")
    completed_action(api, "stop")
    lock_path = next(state_directory.glob("session-*/watch.lock"))
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        with lock_path.open("r+") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return
            except BlockingIOError:
                pass
        time.sleep(0.05)
    raise TimeoutError("Startup watcher did not release its session lock")
